//! A new backup of a brand-new database (M5: a PBIX built without an existing `DataModel`), laid
//! out as the Analysis Services engine of Power BI Desktop 2.158 writes the image of a database
//! it has just created (see `docs/formats/datamodel-fresh.md`): the header page, the
//! `ADDITIONAL_LOG` and `PARTITIONS` bookkeeping files, the database object file
//! (`<id>.<version>.db.xml`, file group 100002), optionally `CryptKey.bin`, `metadata.sqlitedb`
//! (file group 100069), the `LOG` BackupLog and the virtual directory.
//!
//! Every other file (VertiPaq column data, system tables) is then added with
//! [`AbfWriter::add`](crate::AbfWriter::add), which inserts it before `metadata.sqlitedb`, the
//! way the engine lists them.

use crate::header::HEADER_XML_OFFSET;
use crate::writer::storage_name;
use crate::{Abf, Error, PAGE_SIZE};

/// What a new backup holds.
#[derive(Debug, Clone)]
pub struct NewBackup<'a> {
    /// The database ID and name (the engine names the database folder `<id>.0.db`).
    pub database_id: &'a str,
    /// The server data folder written in the BackupLog (`\\?\…\Data`). Only the paths relative
    /// to the database folder matter on restore.
    pub server_root: &'a str,
    /// The database object document (`<id>.<object version>.db.xml`), UTF-8.
    pub database_xml: &'a [u8],
    /// The database's `ObjectVersion` (the `.db.xml` file name carries it).
    pub object_version: u32,
    /// The database's `ObjectID` (a GUID, upper case).
    pub object_id: &'a str,
    /// `0.CryptKey.bin`, when the database has one.
    pub crypt_key: Option<&'a [u8]>,
    pub metadata_sqlitedb: &'a [u8],
    /// The database language (LCID), e.g. 1033.
    pub language: u32,
    /// Windows FILETIME ticks written as every file's time.
    pub file_time: u64,
    /// The header's `ObjectID` (a GUID, upper case).
    pub backup_id: &'a str,
    /// The BackupLog's `SvrEncryptPwdFlag`.
    pub encrypted_passwords: bool,
}

/// The identifier that starts and ends every database key file (`CryptKey.bin`), observed
/// identical in every engine-written backup (`datamodel-fresh.md`, "Database key").
const KEY_ID: [u8; 16] = [
    0x98, 0xbc, 0x21, 0x5d, 0x2d, 0x8d, 0xe6, 0x4e, 0xa8, 0xe5, 0xd0, 0x38, 0xaa, 0xc9, 0x44, 0x41,
];

/// A new database key file (`0.CryptKey.bin`, 144 bytes), laid out as the engine writes it
/// (`datamodel-fresh.md`, "Database key"): the identifier, eight 32-bit fields (format 4, the
/// key blob's offset 48 and length 80, the identifier's length 16, 1, 7, −1, 0), a CryptoAPI
/// `SIMPLEBLOB` header (3DES session key, `CALG_RSA_KEYX` exchange algorithm) and the 64-byte
/// PKCS #1 v1.5 block that holds the key in the clear (little-endian: the 24-byte key, a zero,
/// non-zero padding, `02 00`), four zero bytes, and the identifier again. The 3DES key (odd
/// parity, as the engine's) and the padding come from `seed`.
pub fn database_key(seed: u64) -> Vec<u8> {
    let mut state = seed ^ 0x6b65_795f_7365_6564;
    let mut next = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let mut out = Vec::with_capacity(144);
    out.extend_from_slice(&KEY_ID);
    for field in [4u32, 48, 80, 16, 1, 7, u32::MAX, 0] {
        out.extend_from_slice(&field.to_le_bytes());
    }
    // BLOBHEADER: SIMPLEBLOB, version 2, reserved, CALG_3DES; then ALG_ID CALG_RSA_KEYX.
    out.extend_from_slice(&[0x01, 0x02, 0x00, 0x00, 0x03, 0x66, 0x00, 0x00]);
    out.extend_from_slice(&[0x00, 0xa4, 0x00, 0x00]);
    for _ in 0..24 {
        // Odd parity in every key byte.
        let b = (next() & 0xfe) as u8;
        out.push(if b.count_ones().is_multiple_of(2) {
            b | 1
        } else {
            b
        });
    }
    out.push(0);
    for _ in 0..37 {
        let b = (next() % 255) as u8;
        out.push(b.saturating_add(1));
    }
    out.extend_from_slice(&[0x02, 0x00, 0, 0, 0, 0]);
    out.extend_from_slice(&KEY_ID);
    out
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn bom_utf16(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    v.extend(utf16(s));
    v
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn pad_to_page(v: &mut Vec<u8>) {
    let rem = v.len() % PAGE_SIZE;
    if rem != 0 {
        v.resize(v.len().saturating_add(PAGE_SIZE - rem), 0);
    }
}

/// One stored file: storage name, offset, size.
struct Stored {
    name: String,
    offset: usize,
    size: usize,
}

/// A BackupLog record: absolute path, storage name, time, size.
struct Record {
    path: String,
    storage: String,
    time: u64,
    size: usize,
}

fn records(list: &[Record]) -> String {
    let mut out = String::from("<FileList>");
    for r in list {
        out.push_str(&format!(
            "<BackupFile><Path>{}</Path><StoragePath>{}</StoragePath><LastWriteTime>{}</LastWriteTime><Size>{}</Size></BackupFile>",
            escape(&r.path),
            r.storage,
            r.time,
            r.size
        ));
    }
    out.push_str("</FileList>");
    out
}

/// Writes the backup (see the module documentation). The result is parsed back before it is
/// returned.
pub fn new_backup(b: &NewBackup<'_>) -> Result<Vec<u8>, Error> {
    let bad_name = |what: &str| Error::Unsupported(format!("new backup: invalid {what}"));
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    if !valid(b.database_id) {
        return Err(bad_name("database id"));
    }
    if !valid(b.object_id) || !valid(b.backup_id) {
        return Err(bad_name("object id"));
    }
    let root = b.server_root.trim_end_matches('\\');
    let db_folder = format!("{root}\\{}.0.db", b.database_id);
    let db_xml_path = format!("{root}\\{}.{}.db.xml", b.database_id, b.object_version);

    let mut stream = vec![0u8; PAGE_SIZE];
    let mut dir: Vec<Stored> = Vec::new();
    let mut push = |stream: &mut Vec<u8>, name: String, data: &[u8]| {
        dir.push(Stored {
            name,
            offset: stream.len(),
            size: data.len(),
        });
        stream.extend_from_slice(data);
    };
    push(
        &mut stream,
        "ADDITIONAL_LOG".into(),
        &bom_utf16("<Property><ProductName>Default</ProductName></Property>"),
    );
    push(&mut stream, "PARTITIONS".into(), &bom_utf16("<Partitions/>"));

    let mut salt = 0u64;
    let mut name_for = |path: &str| {
        salt = salt.wrapping_add(1);
        storage_name(path, salt)
    };
    let mut database_files = Vec::new();
    let s = name_for(&db_xml_path);
    push(&mut stream, s.clone(), b.database_xml);
    database_files.push(Record {
        path: db_xml_path,
        storage: s,
        time: b.file_time,
        size: b.database_xml.len(),
    });
    if let Some(key) = b.crypt_key {
        let path = format!("{db_folder}\\0.CryptKey.bin");
        let s = name_for(&path);
        push(&mut stream, s.clone(), key);
        database_files.push(Record {
            path,
            storage: s,
            time: b.file_time,
            size: key.len(),
        });
    }
    let path = format!("{db_folder}\\metadata.sqlitedb");
    let s = name_for(&path);
    push(&mut stream, s.clone(), b.metadata_sqlitedb);
    let data_files = [Record {
        path,
        storage: s,
        time: b.file_time,
        size: b.metadata_sqlitedb.len(),
    }];

    let id = escape(b.database_id);
    let group = |class: u32, version: i64, location: i64, object_id: &str, files: &[Record]| {
        format!(
            "<FileGroup><Class>{class}</Class><ID>{id}</ID><Name>{id}</Name><ObjectVersion>{version}</ObjectVersion>\
             <PersistLocation>{location}</PersistLocation><PersistLocationPath>{}</PersistLocationPath>\
             <StorageLocationPath/><ObjectID>{object_id}</ObjectID>{}</FileGroup>",
            escape(&db_folder),
            records(files)
        )
    };
    let log = format!(
        "<BackupLog><BackupRestoreSyncVersion>11.53</BackupRestoreSyncVersion><ServerRoot>{}</ServerRoot>\
         <SvrEncryptPwdFlag>{}</SvrEncryptPwdFlag><ServerEnableBinaryXML>false</ServerEnableBinaryXML>\
         <ServerEnableCompression>false</ServerEnableCompression><CompressionFlag>false</CompressionFlag>\
         <EncryptionFlag>false</EncryptionFlag><ObjectName>{id}</ObjectName><ObjectId>{id}</ObjectId>\
         <Write>ReadWrite</Write><OlapInfo>false</OlapInfo><IsTabular>true</IsTabular><Collations/>\
         <Languages><Language>{}</Language></Languages><FileGroups>{}{}</FileGroups></BackupLog>",
        escape(root),
        b.encrypted_passwords,
        b.language,
        group(
            100_002,
            i64::from(b.object_version),
            0,
            b.object_id,
            &database_files
        ),
        group(
            100_069,
            -1,
            -1,
            "00000000-0000-0000-0000-000000000000",
            &data_files
        ),
    );
    push(&mut stream, "LOG".into(), &bom_utf16(&log));
    pad_to_page(&mut stream);

    let dir_offset = stream.len();
    let mut dir_xml = String::from("<VirtualDirectory>");
    for f in &dir {
        dir_xml.push_str(&format!(
            "<BackupFile><Path>{}</Path><Size>{}</Size><m_cbOffsetHeader>{}</m_cbOffsetHeader><Delete>false</Delete>\
             <CreatedTimestamp>{t}</CreatedTimestamp><Access>{t}</Access><LastWriteTime>{t}</LastWriteTime></BackupFile>",
            f.name,
            f.size,
            f.offset,
            t = b.file_time
        ));
    }
    dir_xml.push_str("</VirtualDirectory>");
    stream.extend_from_slice(dir_xml.as_bytes());
    pad_to_page(&mut stream);

    let header = format!(
        "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault>\
         <faultcode>0</faultcode><ErrorCode>false</ErrorCode><EncryptionFlag>false</EncryptionFlag>\
         <EncryptionKey>0</EncryptionKey><ApplyCompression>false</ApplyCompression>\
         <m_cbOffsetHeader>{dir_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{}</Files>\
         <ObjectID>{}</ObjectID><m_cbOffsetData>{PAGE_SIZE}</m_cbOffsetData></BackupLog>",
        dir_xml.len(),
        dir.len(),
        b.backup_id
    );
    let mut page = vec![0xFF, 0xFE];
    page.extend(utf16("STREAM_STORAGE_SIGNATURE_)!@#$%^&*("));
    if page.len() != HEADER_XML_OFFSET {
        return Err(Error::Write("backup signature length".into()));
    }
    page.extend(utf16(&header));
    if page.len() > PAGE_SIZE {
        return Err(Error::Write("the backup header does not fit its page".into()));
    }
    if let Some(dst) = stream.get_mut(..page.len()) {
        dst.copy_from_slice(&page);
    }
    // Parse it back: the layout must read as written.
    let parsed = Abf::parse(&stream)?;
    if parsed.metadata_sqlitedb() != Some(b.metadata_sqlitedb) {
        return Err(Error::Write(
            "the new backup does not read back its metadata.sqlitedb".into(),
        ));
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NewBackup<'static> {
        NewBackup {
            database_id: "0f1e2d3c-4b5a-4968-8776-655443322110",
            server_root: r"\\?\C:\bicraft\Data",
            database_xml: b"<Load/>",
            object_version: 1,
            object_id: "6A0D2E7B-0C1F-4E4A-9B21-3D5C7E9F1A2B",
            crypt_key: None,
            metadata_sqlitedb: b"SQLite format 3\0synthetic",
            language: 1033,
            file_time: 134_000_000_000_000_000,
            backup_id: "2B1A9F7E-5C3D-4B21-9A4E-1F0C7B2E0D6A",
            encrypted_passwords: true,
        }
    }

    #[test]
    fn new_backup_reads_back() {
        let bytes = new_backup(&sample()).unwrap();
        let abf = Abf::parse(&bytes).unwrap();
        let names: Vec<&str> = abf.files().iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            names,
            [
                "ADDITIONAL_LOG",
                "PARTITIONS",
                "0f1e2d3c-4b5a-4968-8776-655443322110.1.db.xml",
                "metadata.sqlitedb",
                "LOG"
            ]
        );
        assert_eq!(bytes.len() % PAGE_SIZE, 0);
        assert_eq!(abf.backup_log().file_groups.len(), 2);
        assert_eq!(
            abf.read("metadata.sqlitedb").unwrap(),
            b"SQLite format 3\0synthetic"
        );
        // Files added afterwards go before metadata.sqlitedb, as the engine lists them.
        let mut w = abf.to_writer().unwrap();
        w.add("T (1).tbl/2.prt/1.T (1).C (3).0.idf", b"x".to_vec())
            .unwrap();
        let out = w.finish().unwrap();
        let back = Abf::parse(&out).unwrap();
        let names: Vec<&str> = back.files().iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names.get(3), Some(&"T (1).tbl/2.prt/1.T (1).C (3).0.idf"));
        assert_eq!(names.get(4), Some(&"metadata.sqlitedb"));
    }

    #[test]
    fn database_keys_have_the_engine_layout() {
        let k = database_key(7);
        assert_eq!(k.len(), 144);
        assert_eq!(&k[..16], &KEY_ID);
        assert_eq!(&k[128..], &KEY_ID);
        assert_eq!(&k[16..20], &[4, 0, 0, 0]);
        assert_eq!(&k[48..60], &[1, 2, 0, 0, 3, 0x66, 0, 0, 0, 0xa4, 0, 0]);
        let block = &k[60..124];
        assert!(block[..24].iter().all(|b| b.count_ones() % 2 == 1));
        assert_eq!(block[24], 0);
        assert!(block[25..62].iter().all(|&b| b != 0));
        assert_eq!(&block[62..], &[2, 0]);
        assert_eq!(&k[124..128], &[0, 0, 0, 0]);
        assert_eq!(database_key(7), k);
        assert_ne!(database_key(8)[60..84], k[60..84]);
    }

    #[test]
    fn crypt_key_and_bad_ids() {
        let mut s = sample();
        s.crypt_key = Some(b"key");
        let bytes = new_backup(&s).unwrap();
        let abf = Abf::parse(&bytes).unwrap();
        assert_eq!(abf.read("0.CryptKey.bin"), Some(&b"key"[..]));
        s.database_id = "a<b";
        assert!(new_backup(&s).is_err());
        s.database_id = "";
        assert!(new_backup(&s).is_err());
    }
}
