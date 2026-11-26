//! A `DataModel` built from scratch (M5): the model of a new project, a PBIT or a PBIP without
//! `.pbi/cache.abf` written as a PBIX. See `docs/formats/datamodel-fresh.md`.
//!
//! [`empty_data_model_at`] writes the image of a brand-new database holding only the model-level
//! part of the model (`bicraft_metadata::fresh_sqlitedb_at`: `Model` row, query groups, shared
//! expressions) in the backup layout the engine uses for a database it has just created
//! (`bicraft_abf::new_backup`), XPress9-framed like Desktop's `DataModel`. Everything else
//! (tables with their VertiPaq data, columns, measures, relationships, hierarchies, roles,
//! cultures, perspectives, annotations) is then added by the structural writer
//! ([`crate::structural::DataModelEditor`]) as if the user had added it to an empty file:
//! [`build_data_model_at`] does both steps.

use bicraft_model::SemanticModel;

use crate::Error;
use crate::structural::{DataModelEditor, StructuralData};

/// The server data folder written in a new backup's log. Only paths relative to the database
/// folder matter on restore (the engine restores into its own workspace).
const SERVER_ROOT: &str = r"\\?\C:\bicraft\Data";

/// The engine's database language when the model's culture is not known.
const DEFAULT_LANGUAGE: u32 = 1033;

/// An empty `DataModel` image for `model` (see the module documentation). `now`: Windows
/// FILETIME ticks used for every time stamp, so the output is reproducible. The model's
/// `name` and `compatibility_level` (default 1606, what Desktop gives a new model) go into the
/// database object.
pub fn empty_data_model_at(model: &SemanticModel, now: i64) -> Result<Vec<u8>, Error> {
    let sqlitedb = bicraft_metadata::fresh_sqlitedb_at(model, now)?;
    let seed = seed_of(model, now);
    let database_id = guid(seed, false);
    let object_id = guid(seed ^ 0x5DB0_0B1E, true);
    let backup_id = guid(seed ^ 0xBAC0_0F17, true);
    let unique_id = guid(seed ^ 0x0E1D_0E1D, true);
    let language = language_of(model.culture.as_deref());
    let key = bicraft_abf::database_key(seed ^ 0x0C0F_FEE5);
    let level = model
        .compatibility_level
        .unwrap_or(bicraft_versions::NEW_MODEL_COMPATIBILITY_LEVEL);
    let time = timestamp(now);
    let name = if model.name.trim().is_empty() {
        database_id.clone()
    } else {
        model.name.clone()
    };
    let xml = database_xml(&DatabaseXml {
        name: &name,
        id: &database_id,
        object_id: &object_id,
        unique_id: &unique_id,
        time: &time,
        language,
        compatibility_level: level,
    });
    let file_time = u64::try_from(now).map_err(|_| Error::Structural("clock before 1601".into()))?;
    let abf = bicraft_abf::new_backup(&bicraft_abf::NewBackup {
        database_id: &database_id,
        server_root: SERVER_ROOT,
        database_xml: xml.as_bytes(),
        object_version: 1,
        object_id: &object_id,
        crypt_key: Some(&key),
        metadata_sqlitedb: &sqlitedb,
        language,
        file_time,
        backup_id: &backup_id,
        encrypted_passwords: true,
    })?;
    Ok(bicraft_xpress9::compress_datamodel(&abf, None)?)
}

/// The `DataModel` of `model` built from scratch: [`empty_data_model_at`], then every table,
/// column, relationship and the rest of the model written by the structural writer with the
/// values in `data` (one [`crate::structural::TableValues`] per table, calculated tables and
/// columns included).
pub fn build_data_model_at(model: &SemanticModel, data: &StructuralData, now: i64) -> Result<Vec<u8>, Error> {
    let model = &with_model_name(model);
    let empty = empty_data_model_at(model, now)?;
    let editor = DataModelEditor::open(&empty, &crate::DataModelLimits::default())?;
    editor.write_at(model, data, Some(now))
}

/// `model` with the `Model` object's name (`extra.name`) that every database stores: `Model`,
/// as Desktop names it, unless the model names it.
pub fn with_model_name(model: &SemanticModel) -> SemanticModel {
    let mut m = model.clone();
    m.extra
        .entry("name".to_string())
        .or_insert_with(|| serde_json::Value::String("Model".into()));
    m
}

/// Sets the M query (`QueryDefinition`) of the partitions of the named tables in a `DataModel`
/// part, nothing else: a table written without rows from a stand-in empty source gets its real
/// source back, so Desktop runs it on the next refresh. `queries`: (table name, query).
pub fn set_partition_queries(part: &[u8], queries: &[(String, String)]) -> Result<Vec<u8>, Error> {
    use bicraft_metadata::MetaValue;
    let info = bicraft_xpress9::inspect_datamodel(part)?;
    let abf_bytes = crate::decompress_data_model(part, &crate::DataModelLimits::default())?;
    let abf = bicraft_abf::Abf::parse(&abf_bytes)?;
    let sqlitedb = abf.metadata_sqlitedb().ok_or(Error::NoMetadata)?;
    let db = bicraft_sqlite::Database::from_bytes(sqlitedb)
        .map_err(|e| Error::Structural(format!("metadata.sqlitedb: {e}")))?;
    let tables = bicraft_metadata::tables_from_sqlitedb(sqlitedb)?;
    let id_of = |name: &str| {
        tables
            .rows("Table")
            .iter()
            .find_map(|r| match (r.get("Name"), r.get("ID")) {
                (Some(MetaValue::Text(n)), Some(MetaValue::Integer(id))) if n == name => Some(*id),
                _ => None,
            })
    };
    let mut partitions = tables.rows("Partition").to_vec();
    for (table, query) in queries {
        let id = id_of(table)
            .ok_or_else(|| Error::Structural(format!("table '{table}' is not in the DataModel")))?;
        for r in partitions
            .iter_mut()
            .filter(|r| r.get("TableID") == Some(&MetaValue::Integer(id)))
        {
            r.insert("QueryDefinition".into(), MetaValue::Text(query.clone()));
        }
    }
    let mut edited = bicraft_metadata::MetadataTables::new();
    edited.insert_table("Partition", partitions);
    let info_table = db
        .table("Partition")
        .ok_or_else(|| Error::Structural("metadata.sqlitedb has no Partition table".into()))?
        .clone();
    let rows = bicraft_metadata::tables_to_rows(&edited).rows_for(&info_table)?;
    let new_sqlitedb = bicraft_sqlite::rewrite(&db, |name| {
        if name.eq_ignore_ascii_case("Partition") {
            rows.clone()
        } else {
            None
        }
    })
    .map_err(|e| Error::Structural(format!("metadata.sqlitedb: {e}")))?;
    let mut writer = abf.to_writer()?;
    writer.replace_metadata_sqlitedb(new_sqlitedb)?;
    let new_abf = writer.finish()?;
    Ok(bicraft_xpress9::compress_datamodel(&new_abf, Some(&info))?)
}

struct DatabaseXml<'a> {
    name: &'a str,
    id: &'a str,
    object_id: &'a str,
    unique_id: &'a str,
    time: &'a str,
    language: u32,
    compatibility_level: u32,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The database object document (`<id>.1.db.xml`): the ASSL `Load` of a tabular database as the
/// engine writes it for a new database (elements and namespaces listed in
/// `datamodel-fresh.md`; the database key file listed in `DataFileList`, no stored
/// impersonation password).
fn database_xml(d: &DatabaseXml<'_>) -> String {
    format!(
        "<Load xmlns=\"http://schemas.microsoft.com/analysisservices/2003/engine\" \
         xmlns:ddl200=\"http://schemas.microsoft.com/analysisservices/2010/engine/200\" \
         xmlns:ddl200_200=\"http://schemas.microsoft.com/analysisservices/2010/engine/200/200\" \
         xmlns:ddl400=\"http://schemas.microsoft.com/analysisservices/2012/engine/400\" \
         xmlns:ddl500_500=\"http://schemas.microsoft.com/analysisservices/2013/engine/500/500\" \
         xmlns:ddl800=\"http://schemas.microsoft.com/analysisservices/2018/engine/800\" \
         xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <ObjectDefinition><Database><Name>{name}</Name><ID>{id}</ID>\
         <CreatedTimestamp>{time}</CreatedTimestamp><LastSchemaUpdate>{time}</LastSchemaUpdate>\
         <ObjectVersion>1</ObjectVersion><ObjectID>{object_id}</ObjectID><Ordinal>0</Ordinal>\
         <PersistLocation>0</PersistLocation><System>false</System><DataFileList>0.CryptKey.bin</DataFileList><Description/>\
         <ddl800:DbUniqueId>{unique_id}</ddl800:DbUniqueId><AggregationPrefix/>\
         <Language>{language}</Language><Collation/>\
         <ddl400:DefaultCollationVersion>100</ddl400:DefaultCollationVersion><Visible>true</Visible>\
         <MasterDataSourceID/><ProcessingPriority>0</ProcessingPriority>\
         <ddl200_200:StorageEngineUsed valuens=\"ddl500_500\">TabularMetadata</ddl200_200:StorageEngineUsed>\
         <ddl200:CompatibilityLevel>{level}</ddl200:CompatibilityLevel><Translations/>\
         <DataSourceImpersonationInfo><ImpersonationMode>Default</ImpersonationMode><Account/>\
         </DataSourceImpersonationInfo><DataVersion>0</DataVersion></Database></ObjectDefinition></Load>",
        name = escape(d.name),
        id = escape(d.id),
        object_id = d.object_id,
        unique_id = d.unique_id,
        time = d.time,
        language = d.language,
        level = d.compatibility_level,
    )
}

/// The LCID of a culture name for the database language; 1033 when unknown.
fn language_of(culture: Option<&str>) -> u32 {
    const KNOWN: &[(&str, u32)] = &[
        ("en-US", 1033),
        ("en-GB", 2057),
        ("fr-FR", 1036),
        ("fr-CA", 3084),
        ("fr-BE", 2060),
        ("fr-CH", 4108),
        ("de-DE", 1031),
        ("de-CH", 2055),
        ("de-AT", 3079),
        ("es-ES", 3082),
        ("es-MX", 2058),
        ("it-IT", 1040),
        ("pt-BR", 1046),
        ("pt-PT", 2070),
        ("nl-NL", 1043),
        ("nl-BE", 2067),
        ("ja-JP", 1041),
        ("zh-CN", 2052),
        ("ko-KR", 1042),
        ("sv-SE", 1053),
        ("da-DK", 1030),
        ("nb-NO", 1044),
        ("fi-FI", 1035),
        ("pl-PL", 1045),
        ("ru-RU", 1049),
        ("tr-TR", 1055),
    ];
    let Some(c) = culture else {
        return DEFAULT_LANGUAGE;
    };
    KNOWN
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(c))
        .map_or(DEFAULT_LANGUAGE, |(_, l)| *l)
}

/// A seed from the model's name and the clock: the same inputs give the same IDs.
fn seed_of(model: &SemanticModel, now: i64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in model.name.bytes().chain(now.to_le_bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A version-4-shaped GUID from `seed`.
fn guid(seed: u64, upper: bool) -> String {
    let a = splitmix(seed);
    let b = splitmix(a);
    let s = format!(
        "{:08x}-{:04x}-4{:03x}-{:x}{:03x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xfff,
        8 | ((b >> 62) & 3),
        (b >> 48) & 0xfff,
        b & 0xffff_ffff_ffff
    );
    if upper { s.to_uppercase() } else { s }
}

/// FILETIME ticks as the `xs:dateTime` text of the database object (UTC, no zone).
fn timestamp(filetime: i64) -> String {
    let secs = filetime.div_euclid(10_000_000) - 11_644_473_600;
    let frac = filetime.rem_euclid(10_000_000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = bicraft_locale::calendar::civil_from_days(days);
    let mut text = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    );
    if frac != 0 {
        let f = format!("{frac:07}");
        text.push('.');
        text.push_str(f.trim_end_matches('0'));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 134_364_379_681_883_581;

    #[test]
    fn timestamps_and_ids() {
        assert_eq!(timestamp(NOW), "2026-10-14T07:52:48.1883581");
        assert_eq!(timestamp(116_444_736_000_000_000), "1970-01-01T00:00:00");
        let g = guid(42, false);
        assert_eq!(g.len(), 36);
        assert_eq!(g.as_bytes()[14], b'4');
        assert_eq!(guid(42, false), g);
        assert_ne!(guid(43, false), g);
        assert_eq!(language_of(Some("fr-fr")), 1036);
        assert_eq!(language_of(None), 1033);
    }

    #[test]
    fn empty_model_reads_back() {
        let mut m = SemanticModel {
            name: "Sales & Co".into(),
            culture: Some("en-US".into()),
            ..SemanticModel::default()
        };
        m.extra.insert(
            "defaultPowerBIDataSourceVersion".into(),
            serde_json::json!("powerBI_V3"),
        );
        let part = empty_data_model_at(&m, NOW).unwrap();
        let back = crate::read_data_model(&part).unwrap();
        assert!(back.tables.is_empty());
        assert_eq!(back.culture.as_deref(), Some("en-US"));
        assert_eq!(back.extra["defaultPowerBIDataSourceVersion"], "powerBI_V3");
        // Reproducible.
        assert_eq!(empty_data_model_at(&m, NOW).unwrap(), part);
        let abf = bicraft_xpress9::decompress_datamodel(&part, 1 << 30).unwrap();
        let parsed = bicraft_abf::Abf::parse(&abf).unwrap();
        let xml = parsed
            .files()
            .iter()
            .find(|f| f.path.ends_with(".db.xml"))
            .and_then(|f| parsed.data(f))
            .unwrap();
        let xml = std::str::from_utf8(xml).unwrap();
        assert!(xml.contains("<Name>Sales &amp; Co</Name>"), "{xml}");
        assert!(xml.contains("<ddl200:CompatibilityLevel>1606<"), "{xml}");
    }
}
