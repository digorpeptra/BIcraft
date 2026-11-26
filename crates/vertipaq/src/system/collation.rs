//! String collation used to order attribute-hierarchy members and to match relationship keys.
//!
//! A model without an explicit `Model.Collation` sorts strings with the Windows linguistic
//! sort of its culture: case-insensitive, accent-, kana- and width-sensitive, with "word sort"
//! (hyphens and apostrophes only break ties) and, for French cultures, backward accent
//! ordering. [`Collation::Linguistic`] is our own approximation of that order, built from
//! black-box observations of the engine (see `docs/formats/vertipaq-system-tables.md`); it is
//! exact for ASCII, Latin-1 and Latin Extended-A text and approximate elsewhere. Cultures whose
//! alphabet orders some letters on their own ([`Tailoring`]: Spanish ñ, Swedish å ä ö after z,
//! Czech ch after h, Turkish dotless ı, …) follow the engine's order for those letters
//! (`docs/formats/locales.md`, *Collation*).
//! [`Collation::Ordinal`] compares UTF-16 code units.

use std::cmp::Ordering;

/// How strings are compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collation {
    /// UTF-16 code-unit order, case-sensitive; two strings are equal only when identical.
    Ordinal,
    /// Windows-like linguistic order (the default collation of a model).
    Linguistic {
        /// Compare accents from the end of the string (French cultures).
        french_accents: bool,
        /// Letters the culture's alphabet orders on their own.
        tailoring: Tailoring,
    },
}

impl Default for Collation {
    fn default() -> Self {
        Collation::Linguistic {
            french_accents: false,
            tailoring: Tailoring::None,
        }
    }
}

pub use super::tailoring::Tailoring;

impl Collation {
    /// The default collation of a model whose `Model.Culture` is `culture` (e.g. `fr-FR`).
    pub fn for_culture(culture: &str) -> Collation {
        let lang = culture.split(['-', '_']).next().unwrap_or("");
        Collation::Linguistic {
            french_accents: lang.eq_ignore_ascii_case("fr"),
            tailoring: Tailoring::for_culture(culture),
        }
    }

    /// Total order of two strings. Strings that are equal under [`Collation::equivalent`] are
    /// ordered by their UTF-16 code units, so the order is deterministic.
    pub fn compare(&self, a: &str, b: &str) -> Ordering {
        match self {
            Collation::Ordinal => a.encode_utf16().cmp(b.encode_utf16()),
            Collation::Linguistic { .. } => self
                .sort_key(a)
                .cmp(&self.sort_key(b))
                .then_with(|| a.encode_utf16().cmp(b.encode_utf16())),
        }
    }

    /// True when the engine treats `a` and `b` as the same value (same dictionary entry, same
    /// relationship key): for the linguistic collation, equal weights at every level and the
    /// same UTF-16 length (so `"A"` = `"a"`, but `"é"` ≠ `"e\u{301}"` and `"ß"` ≠ `"ss"`).
    pub fn equivalent(&self, a: &str, b: &str) -> bool {
        match self {
            Collation::Ordinal => a == b,
            Collation::Linguistic { .. } => {
                a.encode_utf16().count() == b.encode_utf16().count() && self.sort_key(a) == self.sort_key(b)
            }
        }
    }

    /// A key such that `equivalence_key(a) == equivalence_key(b)` exactly when
    /// [`Collation::equivalent`] holds; usable in hash maps.
    pub fn equivalence_key(&self, s: &str) -> EquivalenceKey {
        match self {
            Collation::Ordinal => EquivalenceKey::Ordinal(s.encode_utf16().collect()),
            Collation::Linguistic { .. } => {
                EquivalenceKey::Linguistic(self.sort_key(s), s.encode_utf16().count())
            }
        }
    }

    /// The multi-level sort key of `s` (linguistic collation; for [`Collation::Ordinal`] the
    /// key holds the code units as primary weights).
    pub fn sort_key(&self, s: &str) -> SortKey {
        match self {
            Collation::Ordinal => SortKey {
                primary: s.encode_utf16().map(u64::from).collect(),
                ..SortKey::default()
            },
            Collation::Linguistic {
                french_accents,
                tailoring,
            } => linguistic_key(s, *french_accents, *tailoring),
        }
    }
}

/// Which strings count as the same value, as a hashable key: a [`Collation`] (linguistic
/// equivalence) or a [`TextStorage`] (the engine's stored-text identity, which relationship keys
/// follow).
pub trait TextEquivalence {
    /// A key equal for two strings exactly when they are the same value.
    fn text_key(&self, s: &str) -> EquivalenceKey;
}

impl TextEquivalence for Collation {
    fn text_key(&self, s: &str) -> EquivalenceKey {
        self.equivalence_key(s)
    }
}

impl TextEquivalence for TextStorage {
    fn text_key(&self, s: &str) -> EquivalenceKey {
        self.equivalence_key(s)
    }
}

/// Hashable equality key, see [`Collation::equivalence_key`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EquivalenceKey {
    Ordinal(Vec<u16>),
    Linguistic(SortKey, usize),
}

/// Multi-level collation key. Ordering compares the levels in turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SortKey {
    /// Level 1: base characters.
    pub primary: Vec<u64>,
    /// Level 2: accents, one per primary element (already reversed for French accents).
    pub secondary: Vec<u16>,
    /// Level 3: width, ligature and compatibility variants (case is ignored).
    pub tertiary: Vec<u8>,
    /// Level 4: word-sort characters (hyphens, apostrophes) as (insertion point, weight).
    pub special: Vec<(u32, u8)>,
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.primary
            .cmp(&other.primary)
            .then_with(|| self.secondary.cmp(&other.secondary))
            .then_with(|| self.tertiary.cmp(&other.tertiary))
            .then_with(|| {
                if self.primary.is_empty() && other.primary.is_empty() {
                    // Strings made only of word-sort characters: shorter first.
                    self.special.len().cmp(&other.special.len())
                } else {
                    Ordering::Equal
                }
            })
            .then_with(|| self.special.cmp(&other.special))
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------------------------
// Primary weight groups, in sort order. A primary weight is `(group << 32) | value`.

const G_WHITESPACE: u64 = 1;
const G_PUNCT: u64 = 2;
const G_SYMBOL: u64 = 3;
const G_DIGIT: u64 = 4;
const G_NUMBER_FORM: u64 = 5;
const G_LATIN: u64 = 6;
const G_LATIN_OTHER: u64 = 7;
const G_GREEK: u64 = 8;
const G_CYRILLIC: u64 = 9;
const G_ARMENIAN: u64 = 10;
const G_GEORGIAN: u64 = 11;
const G_KANA: u64 = 12;
const G_HEBREW: u64 = 13;
const G_ARABIC: u64 = 14;
const G_INDIC: u64 = 15;
const G_SOUTHEAST_ASIAN: u64 = 16;
const G_OTHER_LETTER: u64 = 17;
const G_PRIVATE_USE: u64 = 18;
const G_SUPPLEMENTARY: u64 = 19;
const G_HANGUL_JAMO: u64 = 20;
const G_HANGUL: u64 = 21;
const G_CJK: u64 = 22;
const G_CJK_SUPPLEMENTARY: u64 = 23;

fn weight(group: u64, value: u32) -> u64 {
    (group << 32) | u64::from(value)
}

/// Whitespace in sort order (observed).
const WHITESPACE: &[char] = &[' ', '\u{A0}', '\t', '\u{200B}'];

/// Punctuation and symbols in sort order (observed on the engine, default collation).
const PUNCTUATION: &[char] = &[
    '!', '"', '#', '$', '%', '&', '(', ')', '*', ',', '\u{3001}', '.', '\u{3002}', '/', ':', ';', '?', '@',
    '[', '\\', ']', '^', '_', '`', '{', '|', '}', '~', '\u{A1}', '\u{A6}', '\u{A8}', '\u{AF}', '\u{B4}',
    '\u{B8}', '\u{BF}', '\u{2016}', '\u{2017}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}', '\u{201C}',
    '\u{201D}', '\u{201E}', '\u{201F}', '\u{2032}', '\u{2033}', '\u{2036}', '\u{2034}', '\u{2037}',
    '\u{2035}', '\u{2038}', '\u{2039}', '\u{203A}', '\u{A2}', '\u{A3}', '\u{A4}', '\u{A5}', '\u{20AC}', '+',
    '<', '=', '>', '\u{B1}', '\u{AB}', '\u{BB}', '\u{D7}', '\u{F7}', '\u{2211}', '\u{221A}', '\u{25A0}',
    '\u{2192}', '\u{2190}', '\u{2500}', '\u{A7}', '\u{A9}', '\u{AC}', '\u{AE}', '\u{B0}', '\u{B5}', '\u{B6}',
    '\u{B7}', '\u{2025}', '\u{2026}', '\u{2605}', '\u{263A}', '\u{2660}', '\u{266A}', '\u{2020}', '\u{2021}',
    '\u{2022}', '\u{2023}', '\u{2030}', '\u{2031}',
];

/// Word-sort characters in weight order: ignored at the first three levels.
const WORD_SORT: &[char] = &[
    '\'', '-', '\u{2010}', '\u{2011}', '\u{2027}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}',
];

// Diacritic (secondary) weights, in observed order.
const D_DOTLESS: u16 = 1;
const D_ACUTE: u16 = 2;
const D_GRAVE: u16 = 3;
const D_DOT: u16 = 4;
const D_CIRCUMFLEX: u16 = 5;
const D_DIAERESIS: u16 = 6;
const D_CARON: u16 = 7;
const D_BREVE: u16 = 8;
const D_MACRON: u16 = 9;
const D_TILDE: u16 = 10;
const D_RING: u16 = 11;
const D_OGONEK: u16 = 12;
const D_DOUBLE_ACUTE: u16 = 13;
const D_CEDILLA: u16 = 14;
const D_STROKE: u16 = 15;
const D_ETH: u16 = 16;
const D_HORN: u16 = 17;
const D_HOOK: u16 = 18;
const D_VARIANT: u16 = 19;
const D_TURNED: u16 = 20;
const D_APOSTROPHE: u16 = 21;
const D_CIRCLED: u16 = 23;
const D_OTHER: u16 = 24;

// Tertiary weights.
const T_WIDE: u8 = 1;
const T_SUPERSCRIPT: u8 = 2;
const T_OTHER_DIGIT: u8 = 3;
const T_EXPANSION: u8 = 4;
const T_HALF_WIDTH: u8 = 0;
const T_KATAKANA: u8 = 5;
const T_HIRAGANA: u8 = 6;

/// One collation element.
#[derive(Clone, Copy)]
struct Element {
    primary: u64,
    secondary: u16,
    tertiary: u8,
}

fn element(primary: u64, secondary: u16, tertiary: u8) -> Element {
    Element {
        primary,
        secondary,
        tertiary,
    }
}

/// Steps per Latin letter: the values after a letter hold letters that sort as their own letter
/// right after it (ŋ after n, tailored letters).
pub(super) const LATIN_STEP: u32 = 16;

/// The primary weight of a Latin letter plus `rank` steps.
pub(super) fn latin_primary(letter: char, rank: u32) -> u64 {
    let index = u32::from(letter).saturating_sub(u32::from('a'));
    weight(G_LATIN, index * LATIN_STEP + rank)
}

fn latin(letter: char, secondary: u16, tertiary: u8) -> Element {
    element(latin_primary(letter, 0), secondary, tertiary)
}

/// What one character contributes.
enum Weights {
    /// Nothing at any level.
    Ignorable,
    /// A combining mark: adds to the previous element's accent weight.
    Diacritic(u16),
    /// A word-sort character (level 4 only).
    WordSort(u8),
    /// One or more collation elements.
    Elements([Option<Element>; 3]),
}

fn one(e: Element) -> Weights {
    Weights::Elements([Some(e), None, None])
}

fn expansion(letters: &str, tertiary: u8) -> Weights {
    let mut out = [None, None, None];
    for (slot, c) in out.iter_mut().zip(letters.chars()) {
        *slot = Some(latin(c, 0, 0));
    }
    if let Some(Some(first)) = out.first_mut() {
        first.tertiary = tertiary;
    }
    Weights::Elements(out)
}

fn linguistic_key(s: &str, french_accents: bool, tailoring: Tailoring) -> SortKey {
    let mut key = SortKey::default();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if let Some((primary, secondary, tertiary, used)) =
            chars.get(i..).and_then(|r| tailoring.element_at(r))
        {
            key.primary.push(primary);
            key.secondary.push(secondary);
            // a contraction (Danish `aa`) sorts after the letter it stands for
            key.tertiary.push(tertiary);
            i += used.max(1);
            continue;
        }
        i += 1;
        let c = tailoring.fold_special(c);
        match weights_of(c) {
            Weights::Ignorable => {}
            Weights::Diacritic(w) => {
                if let Some(last) = key.secondary.last_mut() {
                    *last = last.saturating_add(w);
                }
            }
            Weights::WordSort(w) => {
                let position = u32::try_from(key.primary.len()).unwrap_or(u32::MAX);
                key.special.push((position, w));
            }
            Weights::Elements(elements) => {
                for e in elements.into_iter().flatten() {
                    key.primary.push(tailoring.moved_primary(e.primary));
                    // Czech and Slovak: diaeresis and dot accents before the acute (ä < á, ż < ź).
                    let secondary = match (tailoring, e.secondary) {
                        (Tailoring::Czech | Tailoring::Slovak, D_DIAERESIS | D_DOT) => D_DOTLESS,
                        (_, s) => s,
                    };
                    key.secondary.push(secondary);
                    key.tertiary.push(e.tertiary);
                }
            }
        }
    }
    if french_accents {
        key.secondary.reverse();
    }
    key
}

fn diacritic_of_combining(c: char) -> Option<u16> {
    Some(match c {
        '\u{300}' => D_GRAVE,
        '\u{301}' => D_ACUTE,
        '\u{302}' => D_CIRCUMFLEX,
        '\u{303}' => D_TILDE,
        '\u{304}' => D_MACRON,
        '\u{306}' => D_BREVE,
        '\u{307}' => D_DOT,
        '\u{308}' => D_DIAERESIS,
        '\u{30A}' => D_RING,
        '\u{30B}' => D_DOUBLE_ACUTE,
        '\u{30C}' => D_CARON,
        '\u{327}' => D_CEDILLA,
        '\u{328}' => D_OGONEK,
        '\u{305}' | '\u{309}'..='\u{326}' | '\u{329}'..='\u{36F}' => D_OTHER,
        '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' => D_OTHER,
        '\u{FE20}'..='\u{FE2F}' => D_OTHER,
        _ => return None,
    })
}

/// Latin letters with a diacritic or variant: (base letter, accent weight). Lowercase only.
fn latin_variant(c: char) -> Option<(char, u16)> {
    Some(match c {
        'à' => ('a', D_GRAVE),
        'á' => ('a', D_ACUTE),
        'â' => ('a', D_CIRCUMFLEX),
        'ã' => ('a', D_TILDE),
        'ä' => ('a', D_DIAERESIS),
        'å' => ('a', D_RING),
        'ā' => ('a', D_MACRON),
        'ă' => ('a', D_BREVE),
        'ą' => ('a', D_OGONEK),
        'ɐ' => ('a', D_TURNED),
        'ƀ' => ('b', D_STROKE),
        'ç' => ('c', D_CEDILLA),
        'ć' => ('c', D_ACUTE),
        'ĉ' => ('c', D_CIRCUMFLEX),
        'ċ' => ('c', D_DOT),
        'č' => ('c', D_CARON),
        'ƈ' => ('c', D_HOOK),
        'ď' => ('d', D_CARON),
        'đ' => ('d', D_STROKE),
        'ð' => ('d', D_ETH),
        'è' => ('e', D_GRAVE),
        'é' => ('e', D_ACUTE),
        'ê' => ('e', D_CIRCUMFLEX),
        'ë' => ('e', D_DIAERESIS),
        'ē' => ('e', D_MACRON),
        'ĕ' => ('e', D_BREVE),
        'ė' => ('e', D_DOT),
        'ę' => ('e', D_OGONEK),
        'ě' => ('e', D_CARON),
        'ə' => ('e', D_TURNED),
        'ƒ' => ('f', D_HOOK),
        'ĝ' => ('g', D_CIRCUMFLEX),
        'ğ' => ('g', D_BREVE),
        'ġ' => ('g', D_DOT),
        'ģ' => ('g', D_CEDILLA),
        'ĥ' => ('h', D_CIRCUMFLEX),
        'ħ' => ('h', D_STROKE),
        'ì' => ('i', D_GRAVE),
        'í' => ('i', D_ACUTE),
        'î' => ('i', D_CIRCUMFLEX),
        'ï' => ('i', D_DIAERESIS),
        'ĩ' => ('i', D_TILDE),
        'ī' => ('i', D_MACRON),
        'ĭ' => ('i', D_BREVE),
        'į' => ('i', D_OGONEK),
        'ı' => ('i', D_DOTLESS),
        'ĵ' => ('j', D_CIRCUMFLEX),
        'ķ' => ('k', D_CEDILLA),
        'ĸ' => ('k', D_DOTLESS),
        'ĺ' => ('l', D_ACUTE),
        'ļ' => ('l', D_CEDILLA),
        'ľ' => ('l', D_CARON),
        'ŀ' => ('l', D_DOT),
        'ł' => ('l', D_STROKE),
        'ñ' => ('n', D_TILDE),
        'ń' => ('n', D_ACUTE),
        'ņ' => ('n', D_CEDILLA),
        'ň' => ('n', D_CARON),
        'ŉ' => ('n', D_APOSTROPHE),
        'ò' => ('o', D_GRAVE),
        'ó' => ('o', D_ACUTE),
        'ô' => ('o', D_CIRCUMFLEX),
        'õ' => ('o', D_TILDE),
        'ö' => ('o', D_DIAERESIS),
        'ø' => ('o', D_STROKE),
        'ō' => ('o', D_MACRON),
        'ŏ' => ('o', D_BREVE),
        'ő' => ('o', D_DOUBLE_ACUTE),
        'ơ' => ('o', D_HORN),
        'ŕ' => ('r', D_ACUTE),
        'ŗ' => ('r', D_CEDILLA),
        'ř' => ('r', D_CARON),
        'ś' => ('s', D_ACUTE),
        'ŝ' => ('s', D_CIRCUMFLEX),
        'ş' => ('s', D_CEDILLA),
        'š' => ('s', D_CARON),
        'ſ' => ('s', D_VARIANT),
        'ţ' => ('t', D_CEDILLA),
        'ť' => ('t', D_CARON),
        'ŧ' => ('t', D_STROKE),
        'ù' => ('u', D_GRAVE),
        'ú' => ('u', D_ACUTE),
        'û' => ('u', D_CIRCUMFLEX),
        'ü' => ('u', D_DIAERESIS),
        'ũ' => ('u', D_TILDE),
        'ū' => ('u', D_MACRON),
        'ŭ' => ('u', D_BREVE),
        'ů' => ('u', D_RING),
        'ű' => ('u', D_DOUBLE_ACUTE),
        'ų' => ('u', D_OGONEK),
        'ư' => ('u', D_HORN),
        'ŵ' => ('w', D_CIRCUMFLEX),
        'ý' => ('y', D_ACUTE),
        'ÿ' => ('y', D_DIAERESIS),
        'ŷ' => ('y', D_CIRCUMFLEX),
        'ź' => ('z', D_ACUTE),
        'ż' => ('z', D_DOT),
        'ž' => ('z', D_CARON),
        _ => return None,
    })
}

/// Lowercase of a single character (identity when the lowercase is not one character).
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        // 'İ' lowercases to "i\u{307}": keep it as i with a dot.
        _ => c,
    }
}

fn weights_of(c: char) -> Weights {
    if let Some(i) = WORD_SORT.iter().position(|&w| w == c) {
        return Weights::WordSort(u8::try_from(i + 1).unwrap_or(u8::MAX));
    }
    if let Some(d) = diacritic_of_combining(c) {
        return Weights::Diacritic(d);
    }
    match c {
        '\u{AD}' | '\u{FEFF}' | '\u{FFFD}' | '\u{200C}' | '\u{200D}' | '\u{2060}' => {
            return Weights::Ignorable;
        }
        '\u{130}' => return one(latin('i', D_DOT, 0)),
        '\u{3000}' => return one(element(weight(G_WHITESPACE, 0), 0, T_WIDE)),
        '\u{2024}' => return weights_of('.'),
        _ => {}
    }
    if let Some(i) = WHITESPACE.iter().position(|&w| w == c) {
        return one(element(weight(G_WHITESPACE, index_u32(i)), 0, 0));
    }
    if let Some(i) = PUNCTUATION.iter().position(|&w| w == c) {
        return one(element(weight(G_PUNCT, index_u32(i)), 0, 0));
    }
    // Full-width ASCII forms: the ASCII character, one tertiary step up.
    if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
        let ascii = char::from_u32(u32::from(c) - 0xFF01 + 0x21).unwrap_or(c);
        return with_tertiary(weights_of(ascii), T_WIDE);
    }
    let cp = u32::from(c);
    if c.is_control() {
        return if c.is_whitespace() {
            one(element(weight(G_WHITESPACE, 0x100 + cp), 0, 0))
        } else {
            Weights::Ignorable
        };
    }
    if c.is_ascii_digit() {
        return one(digit(cp - u32::from('0'), 0, 0));
    }
    if let Some(w) = numeric_weights(c) {
        return w;
    }
    let lower = fold(c);
    if lower.is_ascii_lowercase() {
        return one(latin(lower, 0, 0));
    }
    if let Some((base, d)) = latin_variant(lower) {
        return one(latin(base, d, 0));
    }
    match lower {
        'æ' => return expansion("ae", T_EXPANSION),
        'œ' => return expansion("oe", T_EXPANSION),
        'ß' => return expansion("ss", T_EXPANSION),
        'ĳ' => return expansion("ij", T_EXPANSION),
        'þ' => return expansion("th", T_EXPANSION),
        'ǆ' | 'ǅ' | 'Ǆ' => {
            let mut w = expansion("dz", T_EXPANSION);
            if let Weights::Elements([_, Some(z), _]) = &mut w {
                z.secondary = D_CARON;
            }
            return w;
        }
        'ǉ' | 'ǈ' | 'Ǉ' => return expansion("lj", T_EXPANSION),
        'ǌ' | 'ǋ' | 'Ǌ' => return expansion("nj", T_EXPANSION),
        '\u{FB00}' => return expansion("ff", T_EXPANSION),
        '\u{FB01}' => return expansion("fi", T_EXPANSION),
        '\u{FB02}' => return expansion("fl", T_EXPANSION),
        '\u{FB03}' => return expansion("ffi", T_EXPANSION),
        '\u{FB04}' => return expansion("ffl", T_EXPANSION),
        '\u{2122}' => return expansion("tm", T_EXPANSION),
        'ŋ' => {
            let n = u32::from('n') - u32::from('a');
            return one(element(weight(G_LATIN, n * LATIN_STEP + 1), 0, 0));
        }
        '\u{AA}' => return one(latin('a', 0, T_SUPERSCRIPT)),
        '\u{BA}' => return one(latin('o', 0, T_SUPERSCRIPT)),
        _ => {}
    }
    // Circled Latin letters.
    if ('\u{24B6}'..='\u{24E9}').contains(&c) {
        let offset = if cp >= 0x24D0 { cp - 0x24D0 } else { cp - 0x24B6 };
        let letter = char::from_u32(u32::from('a') + offset).unwrap_or('a');
        return one(latin(letter, D_CIRCLED, 0));
    }
    script_weights(c, lower)
}

fn index_u32(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

fn with_tertiary(w: Weights, tertiary: u8) -> Weights {
    match w {
        Weights::Elements(mut es) => {
            for e in es.iter_mut().flatten() {
                e.tertiary = tertiary;
            }
            Weights::Elements(es)
        }
        other => other,
    }
}

/// Digit `d` (0–9) plus a fraction step (0–15) between `d` and `d + 1`.
fn digit(d: u32, fraction: u32, tertiary: u8) -> Element {
    element(weight(G_DIGIT, d * 16 + fraction), 0, tertiary)
}

fn numeric_weights(c: char) -> Option<Weights> {
    let cp = u32::from(c);
    let e = match c {
        '\u{B9}' => digit(1, 0, T_SUPERSCRIPT),
        '\u{B2}' => digit(2, 0, T_SUPERSCRIPT),
        '\u{B3}' => digit(3, 0, T_SUPERSCRIPT),
        '\u{2070}' => digit(0, 0, T_SUPERSCRIPT),
        '\u{2074}'..='\u{2079}' => digit(cp - 0x2070, 0, T_SUPERSCRIPT),
        '\u{2080}'..='\u{2089}' => digit(cp - 0x2080, 0, T_SUPERSCRIPT),
        '\u{BC}' => digit(0, 4, 0),
        '\u{BD}' => digit(0, 8, 0),
        '\u{BE}' => digit(3, 12, 0),
        // Arabic-Indic, extended Arabic-Indic, Devanagari … Thai digits.
        '\u{660}'..='\u{669}' => digit(cp - 0x660, 0, T_OTHER_DIGIT),
        '\u{6F0}'..='\u{6F9}' => digit(cp - 0x6F0, 0, T_OTHER_DIGIT),
        '\u{966}'..='\u{96F}' => digit(cp - 0x966, 0, T_OTHER_DIGIT),
        '\u{E50}'..='\u{E59}' => digit(cp - 0xE50, 0, T_OTHER_DIGIT),
        '\u{2150}'..='\u{218F}' | '\u{2460}'..='\u{24B5}' => element(weight(G_NUMBER_FORM, cp), 0, 0),
        _ => return None,
    };
    Some(one(e))
}

/// Half-width katakana U+FF66–U+FF9D as their full-width katakana.
const HALF_WIDTH_KATAKANA: &str = "ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン";

fn script_weights(c: char, lower: char) -> Weights {
    // Script ranges are matched on the folded character (e.g. the ohm sign is Greek omega).
    let cp = u32::from(lower);
    let lcp = cp;
    let primary = match cp {
        0xFF66..=0xFF9D => {
            let full = usize::try_from(cp - 0xFF66)
                .ok()
                .and_then(|i| HALF_WIDTH_KATAKANA.chars().nth(i))
                .map_or(cp, u32::from);
            return one(element(weight(G_KANA, full), 0, T_HALF_WIDTH));
        }
        0x0370..=0x03FF | 0x1F00..=0x1FFF => weight(G_GREEK, greek_base(lower)),
        0x0400..=0x052F => weight(G_CYRILLIC, cyrillic_base(lower)),
        0x0530..=0x058F => weight(G_ARMENIAN, lcp),
        0x10A0..=0x10FF => weight(G_GEORGIAN, lcp),
        // Hiragana sorts with katakana; the kana type is a tertiary difference.
        0x3041..=0x3096 => {
            return one(element(weight(G_KANA, cp + 0x60), 0, T_HIRAGANA));
        }
        0x30A1..=0x30FA => return one(element(weight(G_KANA, cp), 0, T_KATAKANA)),
        0x0590..=0x05FF => weight(G_HEBREW, cp),
        0x0600..=0x06FF | 0x0750..=0x077F => weight(G_ARABIC, cp),
        0x0900..=0x0DFF => weight(G_INDIC, cp),
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF => weight(G_SOUTHEAST_ASIAN, cp),
        0xE000..=0xF8FF => weight(G_PRIVATE_USE, cp),
        0x1100..=0x11FF => weight(G_HANGUL_JAMO, cp),
        0xAC00..=0xD7AF => weight(G_HANGUL, cp),
        0x2E80..=0x2FDF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => weight(G_CJK, cp),
        0x20000..=0x3FFFF => weight(G_CJK_SUPPLEMENTARY, cp),
        0x10000.. => weight(G_SUPPLEMENTARY, cp),
        _ if c.is_whitespace() => weight(G_WHITESPACE, 0x100 + cp),
        _ if c.is_alphabetic() && cp < 0x0370 => weight(G_LATIN_OTHER, lcp),
        _ if c.is_alphabetic() => weight(G_OTHER_LETTER, lcp),
        _ if c.is_numeric() => weight(G_NUMBER_FORM, cp),
        _ => weight(G_SYMBOL, cp),
    };
    one(element(primary, 0, 0))
}

/// Greek letters without tonos/dialytika (accents are not split out: rare in models).
fn greek_base(c: char) -> u32 {
    let base = match c {
        'ά' => 'α',
        'έ' => 'ε',
        'ή' => 'η',
        'ί' | 'ϊ' | 'ΐ' => 'ι',
        'ό' => 'ο',
        'ύ' | 'ϋ' | 'ΰ' => 'υ',
        'ώ' => 'ω',
        'ς' => 'σ',
        other => other,
    };
    u32::from(base)
}

fn cyrillic_base(c: char) -> u32 {
    // ё sorts right after е, й right after и.
    let cp = u32::from(c);
    match c {
        'ё' => u32::from('е') * 2 + 1,
        'й' => u32::from('и') * 2 + 1,
        _ => cp * 2,
    }
}

// ---------------------------------------------------------------------------------------------
// Stored text values

/// Characters the engine drops from the end of an imported text value (after removing the
/// characters of [`removed_on_import`]): space, tab, line feed and carriage return. Other blanks
/// (U+00A0, U+2003, U+3000, U+0085, U+200B) are kept.
const TRAILING_BLANKS: &[char] = &[' ', '\t', '\n', '\r'];

/// Characters the engine removes from imported text wherever they are: those XML 1.0 does not
/// allow (U+0000–U+0008, U+000B, U+000C, U+000E–U+001F, U+FFFE, U+FFFF). Tab, line feed and
/// carriage return inside a value are kept; U+007F and U+0085 too.
fn removed_on_import(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{B}' | '\u{C}' | '\u{E}'..='\u{1F}' | '\u{FFFE}' | '\u{FFFF}')
}

/// Which text values of a column the engine stores as one dictionary entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Identity {
    /// Only identical strings.
    Exact,
    /// Strings equal up to the case of ASCII letters (en-US).
    AsciiCase,
    /// Strings equivalent under a collation ([`Collation::equivalent`]).
    Collation(Collation),
}

/// How Analysis Services stores the text values of one column (observed on Power BI Desktop
/// 2.158, see `docs/formats/vertipaq-system-tables.md` §5):
///
/// - values the engine considers equal are one dictionary entry, spelled as the first one
///   loaded: in en-US, strings equal up to ASCII case (`apple` = `APPLE`, but `à` ≠ `À`); in
///   other cultures (observed: fr-FR), strings equivalent under the culture's collation (`à` =
///   `À`, `Ω` = `ω`; accent, width and kana variants stay apart);
/// - imported values (an import partition, e.g. Power Query) lose the control characters XML
///   does not allow, then their trailing spaces, tabs and line breaks, before that comparison
///   (`"y "` then `"Y"` are both stored `"y"`, `"a\u{1}b"` is stored `"ab"`); values computed by
///   DAX (calculated tables and columns) keep their trailing blanks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStorage {
    identity: Identity,
    imported: bool,
}

impl TextStorage {
    /// The storage of a column of a model whose culture is `culture` (e.g. `en-US`, `fr-FR`);
    /// `imported`: the column's values come from an import partition (not from DAX).
    pub fn for_culture(culture: &str, imported: bool) -> TextStorage {
        let identity = if culture.trim().eq_ignore_ascii_case("en-US") {
            Identity::AsciiCase
        } else {
            Identity::Collation(Collation::for_culture(culture))
        };
        TextStorage { identity, imported }
    }

    /// Every distinct string is its own value, stored as given.
    pub fn exact() -> TextStorage {
        TextStorage {
            identity: Identity::Exact,
            imported: false,
        }
    }

    /// The text the engine keeps of `s`: for imported values, without the characters XML does
    /// not allow and without trailing blanks.
    pub fn stored<'a>(&self, s: &'a str) -> std::borrow::Cow<'a, str> {
        use std::borrow::Cow;
        if !self.imported {
            return Cow::Borrowed(s);
        }
        if s.chars().any(removed_on_import) {
            let kept: String = s.chars().filter(|c| !removed_on_import(*c)).collect();
            Cow::Owned(kept.trim_end_matches(TRAILING_BLANKS).to_string())
        } else {
            Cow::Borrowed(s.trim_end_matches(TRAILING_BLANKS))
        }
    }

    /// A key equal for two strings exactly when the engine stores them as one value.
    pub fn equivalence_key(&self, s: &str) -> EquivalenceKey {
        let stored = self.stored(s);
        let s: &str = &stored;
        match self.identity {
            Identity::Exact => EquivalenceKey::Ordinal(s.encode_utf16().collect()),
            Identity::AsciiCase => EquivalenceKey::Ordinal(
                s.encode_utf16()
                    .map(|u| match u8::try_from(u) {
                        Ok(b) if b.is_ascii_uppercase() => u16::from(b.to_ascii_lowercase()),
                        _ => u,
                    })
                    .collect(),
            ),
            Identity::Collation(c) => c.equivalence_key(s),
        }
    }

    /// True when the engine stores `a` and `b` as one value.
    pub fn equivalent(&self, a: &str, b: &str) -> bool {
        self.equivalence_key(a) == self.equivalence_key(b)
    }

    /// The values as the engine stores them: every text replaced by the stored form of the first
    /// equivalent text of the column. Other values are unchanged.
    pub fn canonicalize(&self, values: &[crate::CellValue]) -> Vec<crate::CellValue> {
        use crate::CellValue;
        use std::collections::HashMap;
        let mut first: HashMap<EquivalenceKey, String> = HashMap::new();
        // Exact text → stored text, so repeated values are keyed once.
        let mut seen: HashMap<&str, String> = HashMap::new();
        values
            .iter()
            .map(|v| match v {
                CellValue::Text(s) => {
                    if let Some(c) = seen.get(s.as_str()) {
                        return CellValue::Text(c.clone());
                    }
                    let key = self.equivalence_key(s);
                    let c = first
                        .entry(key)
                        .or_insert_with(|| self.stored(s).into_owned())
                        .clone();
                    seen.insert(s.as_str(), c.clone());
                    CellValue::Text(c)
                }
                other => other.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(c: Collation, words: &[&str]) -> Vec<String> {
        let mut v: Vec<&str> = words.to_vec();
        v.sort_by(|a, b| c.compare(a, b));
        v.into_iter().map(String::from).collect()
    }

    #[test]
    fn french_accents_are_compared_backward() {
        let fr = Collation::for_culture("fr-FR");
        let en = Collation::for_culture("en-US");
        let words = ["côté", "cote", "coté", "côte"];
        assert_eq!(sorted(fr, &words), ["cote", "côte", "coté", "côté"]);
        assert_eq!(sorted(en, &words), ["cote", "coté", "côte", "côté"]);
    }

    #[test]
    fn case_is_ignored_but_width_is_not() {
        let c = Collation::default();
        assert!(c.equivalent("ABC", "abc"));
        assert!(c.equivalent("Déf", "dÉF"));
        assert!(!c.equivalent("é", "e"));
        assert!(!c.equivalent("ａ", "a"));
        assert!(!c.equivalent("ß", "ss"));
        assert!(!c.equivalent("é", "e\u{301}"));
        assert_eq!(c.equivalence_key("Straße"), c.equivalence_key("STRAßE"));
        assert_ne!(c.equivalence_key("Straße"), c.equivalence_key("Strasse"));
    }

    #[test]
    fn word_sort_breaks_ties_only() {
        let c = Collation::default();
        let words = ["ab-", "a-b", "abc", "'ab", "ab", "a'b", "-ab", "a--b"];
        assert_eq!(
            sorted(c, &words),
            ["ab", "'ab", "-ab", "a'b", "a-b", "a--b", "ab-", "abc"]
        );
        let words = ["co-op", "coop", "co op", "co_op", "cooq"];
        assert_eq!(sorted(c, &words), ["co op", "co_op", "coop", "co-op", "cooq"]);
    }

    #[test]
    fn groups_follow_the_engine() {
        let c = Collation::default();
        let words = ["a", "1", "!", " ", "α", "д", "中", "€", "+", "Ⅳ", "", "-"];
        assert_eq!(
            sorted(c, &words),
            ["", "-", " ", "!", "€", "+", "1", "Ⅳ", "a", "α", "д", "中"]
        );
        let digits = ["2", "10", "1", "01", "½", "¾", "3", "4"];
        assert_eq!(sorted(c, &digits), ["01", "½", "1", "10", "2", "3", "¾", "4"]);
    }

    fn texts(values: &[Option<&str>]) -> Vec<crate::CellValue> {
        values
            .iter()
            .map(|v| v.map_or(crate::CellValue::Null, |s| crate::CellValue::Text(s.into())))
            .collect()
    }

    /// The stored values Power BI Desktop 2.158 returned for our probe strings, loaded through
    /// Power Query (import) and as a calculated table.
    #[test]
    fn stored_text_follows_the_engine() {
        let input = texts(&[
            Some("y "),
            Some("Y"),
            Some("apple"),
            Some("APPLE "),
            Some("x\t"),
            Some("x\u{A0}"),
            Some("àb"),
            Some("Àb"),
            Some("Ω"),
            Some("ω"),
            Some("Ａb"),
            Some("Ab"),
            Some("   "),
            Some(""),
            None,
            Some("c\r\n\u{B}\u{C}"),
            Some("c\u{85}"),
            Some("z\u{3000}"),
            Some(" apple"),
            Some("é"),
            Some("e\u{301}"),
            Some("a\tb"),
            Some("a\u{B}b"),
            Some("c\u{1}x"),
            Some("\u{1}"),
            Some("a\u{FFFF}b\u{0}"),
            Some("c\u{7F}"),
            Some("k\t \r\n"),
        ]);
        let fr = TextStorage::for_culture("fr-FR", true);
        assert_eq!(
            fr.canonicalize(&input),
            texts(&[
                Some("y"),
                Some("y"),
                Some("apple"),
                Some("apple"),
                Some("x"),
                Some("x\u{A0}"),
                Some("àb"),
                Some("àb"),
                Some("Ω"),
                Some("Ω"),
                Some("Ａb"),
                Some("Ab"),
                Some(""),
                Some(""),
                None,
                Some("c"),
                Some("c\u{85}"),
                Some("z\u{3000}"),
                Some(" apple"),
                Some("é"),
                Some("e\u{301}"),
                Some("a\tb"),
                Some("Ab"),
                Some("cx"),
                Some(""),
                Some("Ab"),
                Some("c\u{7F}"),
                Some("k"),
            ])
        );
        // en-US: only ASCII case variants are one value.
        let us = TextStorage::for_culture("en-US", true);
        let out = us.canonicalize(&input);
        assert_eq!(out.get(7), Some(&crate::CellValue::Text("Àb".into())));
        assert_eq!(out.get(9), Some(&crate::CellValue::Text("ω".into())));
        assert_eq!(out.get(1), Some(&crate::CellValue::Text("y".into())));
        // Calculated (DAX) values keep their trailing blanks.
        let calc = TextStorage::for_culture("fr-FR", false);
        let out = calc.canonicalize(&input);
        assert_eq!(out.first(), Some(&crate::CellValue::Text("y ".into())));
        assert_eq!(out.get(1), Some(&crate::CellValue::Text("Y".into())));
        assert_eq!(out.get(4), Some(&crate::CellValue::Text("x\t".into())));
        assert!(TextStorage::exact().canonicalize(&input) == input);
        assert!(us.equivalent("ABC ", "abc"));
        assert!(!TextStorage::for_culture("en-US", false).equivalent("ABC ", "abc"));
    }

    #[test]
    fn ordinal_is_code_unit_order() {
        let c = Collation::Ordinal;
        assert_eq!(sorted(c, &["b", "B", "a", "é"]), ["B", "a", "b", "é"]);
        assert!(!c.equivalent("a", "A"));
    }
}
