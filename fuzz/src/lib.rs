//! What each cargo-fuzz target does with its input. The stable-Rust smoke tests
//! (`crates/*/tests/fuzz_smoke.rs`) run the same logic and the same input encodings, so a
//! reproducer found here replays there with `BICRAFT_FUZZ_REPLAY=<file>` (see docs/fuzzing.md).
//!
//! Every function must return normally for every input: a panic, a timeout or an out-of-memory
//! is a finding.

use std::sync::OnceLock;

use bicraft_opc::{Package, Part};
use serde_json::Value;

// ------------------------------------------------------------------------------------------------
// Input encodings

/// Splits `data` into `n` frames; all but the last carry a u32 LE length prefix (clamped to
/// what is left).
pub fn frames(data: &[u8], n: usize) -> Vec<&[u8]> {
    let mut out = Vec::with_capacity(n);
    let mut rest = data;
    for _ in 1..n {
        let (len, tail) = match rest.split_first_chunk::<4>() {
            Some((l, t)) => (u32::from_le_bytes(*l) as usize, t),
            None => (0, &[][..]),
        };
        let len = len.min(tail.len());
        out.push(&tail[..len]);
        rest = &tail[len..];
    }
    out.push(rest);
    out
}

/// Documents as `[u16 LE name length][name][u32 LE data length][data]`, repeated (lengths
/// clamped, at most 4096 documents).
pub fn docs(mut data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    while !data.is_empty() && out.len() < 4096 {
        let (n, rest) = match data.split_first_chunk::<2>() {
            Some((l, t)) => (usize::from(u16::from_le_bytes(*l)), t),
            None => (0, &[][..]),
        };
        let n = n.min(rest.len());
        let name = String::from_utf8_lossy(&rest[..n]).into_owned();
        let rest = &rest[n..];
        let (m, rest) = match rest.split_first_chunk::<4>() {
            Some((l, t)) => (u32::from_le_bytes(*l) as usize, t),
            None => (0, &[][..]),
        };
        let m = m.min(rest.len());
        out.push((name, rest[..m].to_vec()));
        data = &rest[m..];
    }
    out
}

// ------------------------------------------------------------------------------------------------
// L0

pub fn opc_package(data: &[u8]) {
    if let Ok(pkg) = Package::read_bytes(data) {
        for part in &pkg.parts {
            let _ = bicraft_opc::decode_text(&part.name, &part.data);
        }
        let _ = pkg.to_bytes();
    }
}

pub fn opc_text(data: &[u8]) {
    if let Ok((text, enc)) = bicraft_opc::decode_text("part", data) {
        let again = bicraft_opc::encode_text(&text, enc);
        assert_eq!(
            bicraft_opc::decode_text("part", &again).ok().map(|t| t.0),
            Some(text)
        );
    }
}

/// Output cap for one fuzzed stream (the expected length comes from the input).
pub const MAX_STREAM: usize = 1 << 22;
/// Output cap for one fuzzed `DataModel` part.
pub const MAX_DATAMODEL: usize = 1 << 24;

/// Input: u32 LE expected length (capped at [`MAX_STREAM`]), then the XPress9 stream.
pub fn xpress9_decompress(data: &[u8]) {
    let Some((len, stream)) = data.split_first_chunk::<4>() else {
        return;
    };
    let expected = (u32::from_le_bytes(*len) as usize).min(MAX_STREAM);
    if let Ok(out) = bicraft_xpress9::decompress(stream, expected) {
        assert_eq!(out.len(), expected);
    }
}

pub fn xpress9_datamodel(data: &[u8]) {
    let _ = bicraft_xpress9::inspect_datamodel(data);
    let _ = bicraft_xpress9::datamodel::detect_layout(data);
    if let Ok(out) = bicraft_xpress9::decompress_datamodel(data, MAX_DATAMODEL) {
        assert!(out.len() <= MAX_DATAMODEL);
    }
}

/// Plain LZ77 (Excel's xpress8): `bicraft_xpress8::fuzz` (chunked, then a single block).
pub fn xpress8_decompress(data: &[u8]) {
    bicraft_xpress8::fuzz(data);
}

/// A Power Pivot workbook or bare backup: `bicraft_powerpivot::fuzz` (small caps).
pub fn powerpivot_workbook(data: &[u8]) {
    bicraft_powerpivot::fuzz(data);
}

pub fn abf_parse(data: &[u8]) {
    let Ok(abf) = bicraft_abf::Abf::parse(data) else {
        return;
    };
    let _ = abf.header();
    let _ = abf.backup_log();
    let _ = abf.summary();
    let _ = abf.metadata_sqlitedb();
    for entry in abf.files() {
        let _ = abf.data(entry);
        let _ = abf.entry(&entry.path);
    }
    if let Ok(writer) = abf.to_writer()
        && let Ok(bytes) = writer.finish()
    {
        let again = bicraft_abf::Abf::parse(&bytes).expect("a re-packed backup parses");
        assert_eq!(again.files().len(), abf.files().len());
    }
}

pub fn sqlite_database(data: &[u8]) {
    let Ok(db) = bicraft_sqlite::Database::from_bytes(data) else {
        return;
    };
    let _ = db.header();
    let _ = db.schema();
    let names: Vec<String> = std::iter::once("sqlite_master".to_string())
        .chain(db.tables().iter().map(|t| t.name.clone()))
        .collect();
    for name in &names {
        let _ = db.column_names(name);
        if let Ok(rows) = db.rows(name) {
            for row in rows {
                if row.is_err() {
                    break;
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// VertiPaq

use bicraft_vertipaq::{ColumnEncoding, Limits, LogicalType, StorageType};

/// Row cap for fuzzed columns (the default 2^26 is bounded but too slow for a fuzz loop).
pub const VERTIPAQ_LIMITS: Limits = Limits {
    max_rows: 1 << 16,
    ..Limits::DEFAULT
};

pub fn vertipaq_idfmeta(data: &[u8]) {
    if let Ok(meta) = bicraft_vertipaq::parse_idfmeta(data) {
        let _ = meta.to_bytes();
    }
}

/// Input: frames `[idfmeta, idf]`.
pub fn vertipaq_idf(data: &[u8]) {
    let f = frames(data, 2);
    if let Ok(meta) = bicraft_vertipaq::parse_idfmeta(f[0]) {
        let _ = bicraft_vertipaq::split_idf(f[1], &meta);
        if let Ok(ids) = bicraft_vertipaq::decode_data_ids(f[1], &meta, &VERTIPAQ_LIMITS) {
            assert!(ids.len() as u64 <= VERTIPAQ_LIMITS.max_rows);
        }
    }
}

pub fn vertipaq_dictionary(data: &[u8]) {
    let _ = bicraft_vertipaq::parse_dictionary(data);
}

pub fn vertipaq_hidx(data: &[u8]) {
    let _ = bicraft_vertipaq::parse_hash_index(data);
}

/// The encoding a `vertipaq_column` input asks for (the first byte of its first frame).
pub fn vertipaq_encoding(selector: u8) -> ColumnEncoding {
    let e = match selector % 8 {
        0 => ColumnEncoding::hash(StorageType::Long),
        1 => ColumnEncoding::hash(StorageType::Real),
        2 => ColumnEncoding::hash(StorageType::String),
        3 => ColumnEncoding::value(-3, 1.0, StorageType::Long),
        4 => ColumnEncoding::value(-2, 4.0, StorageType::Real),
        5 => ColumnEncoding::raw(),
        6 => ColumnEncoding::value(-2, 0.0001, StorageType::Long),
        _ => ColumnEncoding::value(i64::MAX - 5, 0.1, StorageType::Long),
    };
    let codes = [0, 2, 6, 8, 9, 10, 11, 17];
    e.with_logical_type(LogicalType::from_code(codes[usize::from(selector >> 5)]))
}

/// Input: frames `[selector, idfmeta, dictionary, idf]`.
pub fn vertipaq_column(data: &[u8]) {
    let f = frames(data, 4);
    let selector = f[0].first().copied().unwrap_or(0);
    let dict = (!f[2].is_empty()).then_some(f[2]);
    let encoding = vertipaq_encoding(selector);
    if let Ok(cells) =
        bicraft_vertipaq::decode_column_with_limits(f[3], f[1], dict, encoding, &VERTIPAQ_LIMITS)
    {
        assert!(cells.len() as u64 <= VERTIPAQ_LIMITS.max_rows);
    }
}

// ------------------------------------------------------------------------------------------------
// L3

pub fn metadata_sqlitedb(data: &[u8]) {
    let _ = bicraft_metadata::read_sqlitedb(data);
}

const METADATA_FIXTURE: &[u8] =
    include_bytes!("../../crates/metadata/tests/fixtures/synthetic_metadata.sqlitedb");
const TEXTS: &[&str] = &[
    "",
    "Sales",
    "Calendar",
    "Date",
    "Amount",
    "x",
    "[",
    "'",
    "\u{0}",
    "Sales[Amount]",
];
const INTS: &[i64] = &[
    0,
    1,
    2,
    3,
    4,
    -1,
    6,
    8,
    9,
    10,
    17,
    19,
    1567,
    i64::MAX,
    i64::MIN,
    1 << 32,
];

fn metadata_base() -> &'static bicraft_metadata::MetadataTables {
    static BASE: OnceLock<bicraft_metadata::MetadataTables> = OnceLock::new();
    BASE.get_or_init(|| bicraft_metadata::tables_from_sqlitedb(METADATA_FIXTURE).unwrap_or_default())
}

/// Applies a byte script to the fixture rows: each 6-byte op picks a table, row and column, then
/// sets a value, nulls it, removes or duplicates the row.
pub fn metadata_apply_script(
    base: &bicraft_metadata::MetadataTables,
    script: &[u8],
) -> bicraft_metadata::MetadataTables {
    use bicraft_metadata::MetaValue;
    let mut t = base.clone();
    let names: Vec<String> = base.table_names().map(str::to_string).collect();
    if names.is_empty() {
        return t;
    }
    for op in script.chunks(6).take(256) {
        let at = |i: usize| op.get(i).copied().unwrap_or(0) as usize;
        let name = &names[at(1) % names.len()];
        let mut rows = t.rows(name).to_vec();
        if rows.is_empty() {
            continue;
        }
        let r = at(2) % rows.len();
        let col = rows[r].keys().nth(at(3) % rows[r].len().max(1)).cloned();
        let value = match at(0) % 7 {
            0 => Some(MetaValue::Integer(INTS[at(4) % INTS.len()])),
            1 => Some(MetaValue::Text(TEXTS[at(4) % TEXTS.len()].to_string())),
            2 => Some(MetaValue::Null),
            3 => Some(MetaValue::Real([f64::NAN, -0.0, 1e300, 0.5][at(4) % 4])),
            4 => Some(MetaValue::Blob(vec![at(4) as u8; at(5) % 9])),
            5 => {
                rows.remove(r);
                None
            }
            _ => {
                let copy = rows[r].clone();
                rows.push(copy);
                None
            }
        };
        if let (Some(v), Some(c)) = (value, col) {
            rows[r].insert(c, v);
        }
        t.insert_table(name.clone(), rows);
    }
    t
}

pub fn metadata_rows(script: &[u8]) {
    let tables = metadata_apply_script(metadata_base(), script);
    if let Ok(model) = bicraft_metadata::build_model(&tables) {
        let _ = bicraft_metadata::apply_model_at(&tables, &model, 134_000_000_000_000_000);
    }
}

pub fn tmsl_read(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    if let Ok(model) = bicraft_tmsl::read(&text) {
        let written = bicraft_tmsl::write(&model);
        if let Err(e) = bicraft_tmsl::read(&written) {
            panic!("written model must read back: {e}");
        }
    }
}

/// Input: documents (see [`docs`]), decoded as UTF-8 (lossy).
pub fn tmdl_read(data: &[u8]) {
    let docs: Vec<(String, String)> = docs(data)
        .into_iter()
        .map(|(n, d)| (n, String::from_utf8_lossy(&d).into_owned()))
        .collect();
    for (file, text) in &docs {
        let _ = bicraft_tmdl::parse(file, text);
    }
    if let Ok(model) = bicraft_tmdl::read_documents(&docs) {
        let written = bicraft_tmdl::write_documents(&model);
        if let Err(e) = bicraft_tmdl::read_documents(&written) {
            panic!("written model must read back: {e}");
        }
    }
}

/// Input: documents (see [`docs`]), paths relative to the report root.
pub fn pbir_read(data: &[u8]) {
    let docs = docs(data);
    let _ = bicraft_pbir::validate_documents(&docs);
    if let Ok(report) = bicraft_pbir::read_documents(&docs) {
        let _ = bicraft_pbir::check_names(&report);
        let written = bicraft_pbir::write_documents(&report);
        if let Err(e) = bicraft_pbir::read_documents(&written) {
            panic!("written report must read back: {e}");
        }
    }
}

pub fn layout_read(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = bicraft_layout::read_layout(&text);
    let _ = bicraft_layout::read_layout_with_summary(&text);
}

// ------------------------------------------------------------------------------------------------
// L4

fn pbix_exercise(pkg: &Package) {
    let _ = bicraft_pbix::inventory(pkg);
    let _ = bicraft_pbix::read_model(pkg);
    let _ = bicraft_pbix::read_report(pkg);
}

pub fn pbix_package(data: &[u8]) {
    if let Ok(pkg) = Package::read_bytes(data) {
        pbix_exercise(&pkg);
    }
}

/// Input: documents (see [`docs`]) used as package parts, bypassing the ZIP CRC check.
pub fn pbix_parts(data: &[u8]) {
    let pkg = Package {
        parts: docs(data)
            .into_iter()
            .map(|(name, data)| Part {
                name,
                data,
                stored: true,
            })
            .collect(),
    };
    pbix_exercise(&pkg);
}

/// Input: a `.pbiviz` archive (the reader of `visual.import_custom`).
pub fn pbiviz_read(data: &[u8]) {
    let _ = bicraft_pbix::pbiviz::read_pbiviz(data);
}

/// The engine's readers of `file.open` (M11.36): the first frame picks the reader, the second
/// is the file, the third a Dataflow's M document (as `crates/engine/tests/fuzz_smoke.rs`).
pub fn import_readers(data: &[u8]) {
    use bicraft_engine::readers::{Kind, read_bytes};
    let f = frames(data, 3);
    let (Some(pick), Some(body), Some(doc)) = (f.first(), f.get(1), f.get(2)) else {
        return;
    };
    let kind = match pick.first().copied().unwrap_or(0) % 6 {
        0 => Kind::Vpax,
        1 => Kind::Pbids,
        2 => Kind::M,
        3 => Kind::Pqt,
        4 => Kind::Cdm,
        _ => Kind::Dataflow,
    };
    let _ = read_bytes(kind, "fuzz", body, Some(doc));
}

// ------------------------------------------------------------------------------------------------
// DAX

const KINDS: [bicraft_dax::ExprKind; 5] = [
    bicraft_dax::ExprKind::Expression,
    bicraft_dax::ExprKind::Measure,
    bicraft_dax::ExprKind::CalculatedColumn,
    bicraft_dax::ExprKind::CalculatedTable,
    bicraft_dax::ExprKind::RowFilter,
];

/// The DAX oracle's synthetic model (crates/dax/tests/oracle/model.bim).
fn dax_model() -> &'static bicraft_model::SemanticModel {
    static MODEL: OnceLock<bicraft_model::SemanticModel> = OnceLock::new();
    MODEL.get_or_init(|| {
        bicraft_tmsl::read(include_str!("../../crates/dax/tests/oracle/model.bim")).unwrap_or_default()
    })
}

pub fn dax_expression(data: &[u8]) {
    use bicraft_dax::Context;
    let text = String::from_utf8_lossy(data);
    let _ = bicraft_dax::parse_expression(&text);
    let _ = bicraft_dax::validate(&text, &Context::default());
    let ctx = Context {
        model: Some(dax_model()),
        table: Some("Sales"),
    };
    let _ = bicraft_dax::validate_as(&text, &ctx, KINDS[data.len() % KINDS.len()]);
}

pub fn dax_query(data: &[u8]) {
    use bicraft_dax::Context;
    let text = String::from_utf8_lossy(data);
    let _ = bicraft_dax::parse_query(&text);
    let _ = bicraft_dax::validate_query(&text, &Context::default());
    let ctx = Context {
        model: Some(dax_model()),
        table: None,
    };
    let _ = bicraft_dax::validate_query(&text, &ctx);
}

// ------------------------------------------------------------------------------------------------
// MCP

/// Substrings (ASCII case-insensitive, in any decoded JSON key or string) that mark a message
/// able to reach the file system; such messages are dropped so fuzzing never writes files.
const FS_WORDS: &[&str] = &["open", "inspect", "save", "project", "file", "path", "dir"];

pub fn touches_fs(v: &Value) -> bool {
    let bad = |s: &str| {
        let s = s.to_ascii_lowercase();
        FS_WORDS.iter().any(|w| s.contains(w))
    };
    match v {
        Value::String(s) => bad(s),
        Value::Array(a) => a.iter().any(touches_fs),
        Value::Object(o) => o.iter().any(|(k, v)| bad(k) || touches_fs(v)),
        _ => false,
    }
}

/// Input: one JSON-RPC message per line, on a fresh server (at most 64 lines).
pub fn mcp_handle(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let mut server = bicraft_mcp::Server::new();
    for line in text.split('\n').take(64) {
        if let Ok(v) = serde_json::from_str::<Value>(line.trim())
            && touches_fs(&v)
        {
            continue;
        }
        if let Some(reply) = bicraft_mcp::handle(&mut server, line) {
            assert!(
                serde_json::from_str::<Value>(&reply).is_ok(),
                "reply is JSON: {reply}"
            );
        }
    }
}

/// Every token of a TDS token stream (rows decoded against their columns).
pub fn tds_tokens(data: &[u8]) {
    let mut t = bicraft_sqlserver::token::Tokens::new(data, 1 << 20);
    for _ in 0..100_000 {
        match t.next_token() {
            Ok(Some(_)) => {}
            _ => break,
        }
    }
}

/// The OData readers over one input: `$metadata` (CSDL), the URL checks, the value readers, an
/// entity-set page against a column of every type.
pub fn odata_payloads(data: &[u8]) {
    use bicraft_webdata::{csdl, http, odata};
    let columns: Vec<csdl::Property> = [
        ("s", "Edm.String"),
        ("b", "Edm.Boolean"),
        ("i8", "Edm.SByte"),
        ("i16", "Edm.Int16"),
        ("i32", "Edm.Int32"),
        ("i64", "Edm.Int64"),
        ("f", "Edm.Single"),
        ("d", "Edm.Double"),
        ("m", "Edm.Decimal"),
        ("dt", "Edm.Date"),
        ("t", "Edm.DateTime"),
        ("o", "Edm.DateTimeOffset"),
        ("tod", "Edm.TimeOfDay"),
        ("bin", "Edm.Binary"),
    ]
    .into_iter()
    .filter_map(|(n, e)| {
        Some(csdl::Property {
            name: n.into(),
            edm: e.into(),
            ty: csdl::edm_type(e)?,
        })
    })
    .collect();
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = csdl::parse(text);
        if http::check_url(text).is_ok() {
            let _ = http::join_path(text, text);
            let _ = http::with_query(text, &[(text.to_string(), text.to_string())]);
        }
        for s in text.split('"').take(1000) {
            let _ = odata::date(s);
            let _ = odata::time(s);
            let _ = odata::base64(s);
        }
    }
    let _ = odata::page_rows(data, &columns);
}
