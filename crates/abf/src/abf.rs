//! The container: virtual directory ([MS-XLDM] §2.1.3), BackupLog ([MS-XLDM] §2.1.2.3) and the
//! file data regions they describe.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use crate::Error;
use crate::header::{self, Header, bool_field, num_field};
use crate::text::decode_xml;
use crate::xml::{self, Element};

/// Virtual-directory names of the bookkeeping entries Power BI writes around the real files.
const LOG_NAME: &str = "LOG";
const ADDITIONAL_LOG_NAME: &str = "ADDITIONAL_LOG";
const PARTITIONS_NAME: &str = "PARTITIONS";
pub(crate) const METADATA_NAME: &str = "metadata.sqlitedb";

/// What a file in the backup holds, judged from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    /// The files-section `BackupLog` XML (last file in the stream).
    BackupLog,
    /// Power BI's `ADDITIONAL_LOG` XML (first file in the stream; not in [MS-XLDM]).
    AdditionalLog,
    /// The `Partitions` XML ([MS-XLDM] §2.1.2.1).
    Partitions,
    /// `metadata.sqlitedb`: the tabular metadata as a SQLite 3 database.
    MetadataSqlite,
    /// `CryptKey.bin` ([MS-XLDM] §2.1.2.4).
    CryptKey,
    /// Any other XML file (Power BI: the database `.db.xml`; Excel: all object metadata).
    Xml,
    /// VertiPaq column segment data (`.idf`, [MS-XLDM] §2.3.1).
    Idf,
    /// Per-segment column statistics (`.idfmeta`, Power BI binary).
    IdfMeta,
    /// Column dictionary (`.dictionary`, [MS-XLDM] §2.3.2).
    Dictionary,
    /// Hash index of a value-encoded column (`.hidx`, [MS-XLDM] §2.3.3).
    HashIndex,
    /// Anything else.
    Other,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::BackupLog => "backup-log",
            FileKind::AdditionalLog => "additional-log",
            FileKind::Partitions => "partitions",
            FileKind::MetadataSqlite => "metadata-sqlitedb",
            FileKind::CryptKey => "cryptkey",
            FileKind::Xml => "xml",
            FileKind::Idf => "idf",
            FileKind::IdfMeta => "idfmeta",
            FileKind::Dictionary => "dictionary",
            FileKind::HashIndex => "hidx",
            FileKind::Other => "other",
        }
    }

    fn from_name(name: &str) -> FileKind {
        let lower = name.to_ascii_lowercase();
        if lower == METADATA_NAME {
            FileKind::MetadataSqlite
        } else if lower.ends_with("cryptkey.bin") {
            FileKind::CryptKey
        } else if lower.ends_with(".idfmeta") {
            FileKind::IdfMeta
        } else if lower.ends_with(".idf") {
            FileKind::Idf
        } else if lower.ends_with(".dictionary") {
            FileKind::Dictionary
        } else if lower.ends_with(".hidx") {
            FileKind::HashIndex
        } else if lower.ends_with(".xml") {
            FileKind::Xml
        } else {
            FileKind::Other
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The engine-generated system table a VertiPaq file belongs to, from the `H$`/`R$`/`U$`
/// prefix of its table folder or file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SystemTable {
    /// `H$`: attribute hierarchy (position ↔ data ID maps).
    AttributeHierarchy,
    /// `R$`: relationship index.
    Relationship,
    /// `U$`: user hierarchy.
    UserHierarchy,
}

impl SystemTable {
    pub fn prefix(self) -> &'static str {
        match self {
            SystemTable::AttributeHierarchy => "H$",
            SystemTable::Relationship => "R$",
            SystemTable::UserHierarchy => "U$",
        }
    }

    /// Detects the prefix on any path segment, also after an Excel-style `<n>.` ordinal.
    fn detect(path: &str) -> Option<SystemTable> {
        path.split('/').find_map(|seg| {
            let bare = seg.trim_start_matches(|c: char| c.is_ascii_digit());
            let bare = if bare.len() < seg.len() {
                bare.strip_prefix('.').unwrap_or(bare)
            } else {
                seg
            };
            [
                SystemTable::AttributeHierarchy,
                SystemTable::Relationship,
                SystemTable::UserHierarchy,
            ]
            .into_iter()
            .find(|t| bare.starts_with(t.prefix()))
        })
    }
}

/// One file stored in the backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Logical path, `/`-separated, relative to the database folder recorded in the BackupLog
    /// (e.g. `Sales (12).tbl/13.prt/...0.idf`). Entries the BackupLog does not describe use
    /// their virtual-directory name.
    pub path: String,
    /// The virtual-directory key (`StoragePath` in the BackupLog): a 20-hex-digit name in
    /// Power BI files.
    pub storage_path: String,
    /// The absolute path recorded by the server that wrote the backup, if any. It can contain
    /// a local user directory: do not log it.
    pub original_path: Option<String>,
    /// Absolute offset of the file data in the stream.
    pub offset: usize,
    /// Payload length (BackupLog `Size`, which excludes any CRC end marker).
    pub size: usize,
    /// Bytes reserved in the stream (virtual directory `Size`, including any CRC end marker).
    pub stored_size: usize,
    pub kind: FileKind,
    pub system_table: Option<SystemTable>,
    /// FILETIME-style timestamp (100 ns ticks since 1601) from the BackupLog or directory.
    pub last_write_time: Option<u64>,
    /// `Class` of the BackupLog file group this file belongs to.
    pub file_group_class: Option<u32>,
    /// Stored Xpress-compressed (`ApplyCompression`, Excel Power Pivot backups): `size` is the
    /// decompressed size, the stored bytes ([`Abf::stored`]) are Plain LZ77 chunks (plus the CRC
    /// marker when `ErrorCode` is set), and [`Abf::data`] returns `None`. The BackupLog and the
    /// entries it does not describe are never compressed (`docs/formats/powerpivot.md`).
    pub compressed: bool,
}

impl FileEntry {
    /// The last path segment.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// One `BackupFile` record of the files-section BackupLog
/// (`SdfFileListBackupFileType`, [MS-XLDM] §2.1.2.3.1.3.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    pub path: String,
    pub storage_path: String,
    pub last_write_time: Option<u64>,
    pub size: Option<u64>,
}

/// A `FileGroup` of the BackupLog ([MS-XLDM] §2.1.2.3.1.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGroup {
    pub class: Option<u32>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub persist_location_path: Option<String>,
    pub files: Vec<LogFile>,
}

/// The fields of the files-section BackupLog (`SdfBackupLogType`, [MS-XLDM] §2.1.2.3.1)
/// that the reader keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupLog {
    /// `BackupRestoreSyncVersion`. A dotted string in Power BI files, an integer in Excel's.
    pub sync_version: Option<String>,
    pub object_name: Option<String>,
    pub object_id: Option<String>,
    pub is_tabular: Option<bool>,
    pub file_groups: Vec<FileGroup>,
}

/// A parsed ABF backup. File data are zero-copy slices of the input.
#[derive(Debug, Clone)]
pub struct Abf<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) header: Header,
    log: BackupLog,
    pub(crate) files: Vec<FileEntry>,
    unmatched_log_records: usize,
}

struct DirEntry {
    path: String,
    offset: usize,
    size: usize,
    last_write_time: Option<u64>,
}

impl<'a> Abf<'a> {
    /// Parses a decompressed ABF stream.
    ///
    /// Pass the output of XPress9 decompression for a PBIX `DataModel` or `.pbi/cache.abf`;
    /// passing the compressed part returns [`Error::Xpress9Compressed`].
    pub fn parse(bytes: &'a [u8]) -> Result<Abf<'a>, Error> {
        let header = header::parse(bytes)?;
        if header.encrypted {
            return Err(Error::Unsupported("encrypted backups".into()));
        }
        let dir_bytes = region(
            bytes,
            header.directory_offset,
            header.directory_size,
            "virtual directory",
        )?;
        let stream_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let dir = parse_directory(stream_len, dir_bytes).map_err(|e| match e {
            Error::Malformed { .. } if header.apply_compression => {
                Error::Unsupported("whole-stream compressed (ApplyCompression) backups".into())
            }
            e => e,
        })?;

        let log_index = log_index(&dir).ok_or_else(|| malformed("virtual directory", "no entries"))?;
        let log_entry = dir
            .get(log_index)
            .ok_or_else(|| malformed("virtual directory", "no BackupLog"))?;
        let log_bytes = slice(bytes, log_entry.offset, log_entry.size, "BackupLog")?;
        let log = parse_log(log_bytes)?;

        let (files, unmatched_log_records) = join(&dir, log_index, &log, header.apply_compression)?;
        Ok(Abf {
            bytes,
            header,
            log,
            files,
            unmatched_log_records,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn backup_log(&self) -> &BackupLog {
        &self.log
    }

    /// All files, in stream order (virtual directory order).
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    /// Finds a file by logical path (exact, then ASCII case-insensitive with `\` treated as
    /// `/`), then by virtual-directory name.
    pub fn entry(&self, path: &str) -> Option<&FileEntry> {
        let wanted = path.replace('\\', "/");
        self.files
            .iter()
            .find(|f| f.path == wanted)
            .or_else(|| self.files.iter().find(|f| f.path.eq_ignore_ascii_case(&wanted)))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|f| f.storage_path.eq_ignore_ascii_case(path))
            })
    }

    /// The payload of a file, by path as in [`Abf::entry`].
    pub fn read(&self, path: &str) -> Option<&'a [u8]> {
        self.entry(path).and_then(|f| self.data(f))
    }

    /// The payload of an entry returned by [`Abf::files`]; `None` for a
    /// [compressed](FileEntry::compressed) entry, whose content is [`Abf::stored`] decompressed.
    pub fn data(&self, entry: &FileEntry) -> Option<&'a [u8]> {
        if entry.compressed {
            return None;
        }
        self.bytes
            .get(entry.offset..entry.offset.checked_add(entry.size)?)
    }

    /// The bytes an entry occupies in the stream (virtual directory `Size`): the payload, any
    /// CRC end marker, and for a [compressed](FileEntry::compressed) entry its chunks.
    pub fn stored(&self, entry: &FileEntry) -> Option<&'a [u8]> {
        self.bytes
            .get(entry.offset..entry.offset.checked_add(entry.stored_size)?)
    }

    /// The `metadata.sqlitedb` entry, found by file name (case-insensitive).
    pub fn metadata_entry(&self) -> Option<&FileEntry> {
        self.files
            .iter()
            .find(|f| f.name().eq_ignore_ascii_case(METADATA_NAME))
    }

    /// The `metadata.sqlitedb` bytes.
    pub fn metadata_sqlitedb(&self) -> Option<&'a [u8]> {
        self.metadata_entry().and_then(|f| self.data(f))
    }

    /// Counts for diagnostics. Contains no names or paths.
    pub fn summary(&self) -> Summary {
        let mut kinds = BTreeMap::new();
        let mut system_tables = BTreeMap::new();
        let mut payload_bytes = 0u64;
        for f in &self.files {
            *kinds.entry(f.kind).or_insert(0) += 1;
            if let Some(t) = f.system_table {
                *system_tables.entry(t).or_insert(0) += 1;
            }
            payload_bytes = payload_bytes.saturating_add(f.size as u64);
        }
        Summary {
            stream_bytes: self.bytes.len(),
            file_count: self.files.len(),
            payload_bytes,
            kinds,
            system_tables,
            header_sync_version: self.header.sync_version,
            log_sync_version: self.log.sync_version.clone(),
            crc_markers: self.header.crc_markers,
            apply_compression: self.header.apply_compression,
            is_tabular: self.log.is_tabular,
            file_groups: self.log.file_groups.len(),
            has_metadata_sqlitedb: self.metadata_entry().is_some(),
            unmatched_log_records: self.unmatched_log_records,
        }
    }
}

/// Diagnostic counts for an [`Abf`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub stream_bytes: usize,
    pub file_count: usize,
    pub payload_bytes: u64,
    pub kinds: BTreeMap<FileKind, usize>,
    pub system_tables: BTreeMap<SystemTable, usize>,
    pub header_sync_version: Option<u32>,
    pub log_sync_version: Option<String>,
    pub crc_markers: bool,
    pub apply_compression: bool,
    pub is_tabular: Option<bool>,
    pub file_groups: usize,
    pub has_metadata_sqlitedb: bool,
    /// BackupLog records whose `StoragePath` is missing from the virtual directory (or
    /// repeated). They are skipped.
    pub unmatched_log_records: usize,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "ABF: {} bytes, {} files ({} payload bytes), {} file groups",
            self.stream_bytes, self.file_count, self.payload_bytes, self.file_groups
        )?;
        writeln!(
            f,
            "sync version: header {}, log {}; crc markers: {}; compression: {}; tabular: {}",
            self.header_sync_version
                .map_or_else(|| "?".to_string(), |v| v.to_string()),
            self.log_sync_version.as_deref().unwrap_or("?"),
            self.crc_markers,
            self.apply_compression,
            self.is_tabular.map_or_else(|| "?".to_string(), |v| v.to_string()),
        )?;
        let kinds: Vec<String> = self.kinds.iter().map(|(k, n)| format!("{k}={n}")).collect();
        writeln!(f, "kinds: {}", kinds.join(", "))?;
        let sys: Vec<String> = self
            .system_tables
            .iter()
            .map(|(t, n)| format!("{}={n}", t.prefix()))
            .collect();
        writeln!(
            f,
            "system-table files: {}",
            if sys.is_empty() {
                "none".to_string()
            } else {
                sys.join(", ")
            }
        )?;
        write!(
            f,
            "metadata.sqlitedb: {}; unmatched log records: {}",
            if self.has_metadata_sqlitedb {
                "present"
            } else {
                "absent"
            },
            self.unmatched_log_records
        )
    }
}

fn malformed(part: &'static str, reason: &str) -> Error {
    Error::Malformed {
        part,
        reason: reason.to_string(),
    }
}

/// Bounds-checked `offset..offset+len` with u64 inputs.
fn region<'b>(bytes: &'b [u8], offset: u64, len: u64, what: &'static str) -> Result<&'b [u8], Error> {
    let oob = || Error::OutOfBounds {
        what,
        offset,
        len,
        stream_len: bytes.len(),
    };
    let start = usize::try_from(offset).map_err(|_| oob())?;
    let size = usize::try_from(len).map_err(|_| oob())?;
    slice(bytes, start, size, what)
}

fn slice<'b>(bytes: &'b [u8], start: usize, len: usize, what: &'static str) -> Result<&'b [u8], Error> {
    start
        .checked_add(len)
        .and_then(|end| bytes.get(start..end))
        .ok_or(Error::OutOfBounds {
            what,
            offset: start as u64,
            len: len as u64,
            stream_len: bytes.len(),
        })
}

fn required_text(e: &Element, name: &str, part: &'static str) -> Result<String, Error> {
    e.child_text(name)
        .map(str::to_string)
        .ok_or_else(|| Error::Malformed {
            part,
            reason: format!("BackupFile without {name}"),
        })
}

/// Parses the virtual directory; every entry must lie within the first `stream_len` bytes.
fn parse_directory(stream_len: u64, dir_bytes: &[u8]) -> Result<Vec<DirEntry>, Error> {
    const PART: &str = "virtual directory";
    let doc = decode_xml(dir_bytes, PART)?;
    let root = xml::parse(&doc, PART)?;
    if root.name != "VirtualDirectory" {
        return Err(malformed(PART, "root element is not VirtualDirectory"));
    }
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for bf in root.children_named("BackupFile") {
        let path = required_text(bf, "Path", PART)?;
        let size: u64 =
            num_field(bf, "Size", PART)?.ok_or_else(|| malformed(PART, "BackupFile without Size"))?;
        let offset: u64 = num_field(bf, "m_cbOffsetHeader", PART)?
            .ok_or_else(|| malformed(PART, "BackupFile without m_cbOffsetHeader"))?;
        let oob = || Error::OutOfBounds {
            what: "file data",
            offset,
            len: size,
            stream_len: usize::try_from(stream_len).unwrap_or(usize::MAX),
        };
        if offset.checked_add(size).is_none_or(|end| end > stream_len) {
            return Err(oob());
        }
        if !seen.insert(path.to_ascii_uppercase()) {
            return Err(malformed(PART, "duplicate Path"));
        }
        let offset = usize::try_from(offset).map_err(|_| oob())?;
        let size = usize::try_from(size).map_err(|_| oob())?;
        out.push(DirEntry {
            path,
            offset,
            size,
            last_write_time: num_field(bf, "LastWriteTime", PART)?,
        });
    }
    Ok(out)
}

fn parse_log(bytes: &[u8]) -> Result<BackupLog, Error> {
    const PART: &str = "BackupLog";
    let doc = decode_xml(bytes, PART)?;
    let root = xml::parse(&doc, PART)?;
    if root.name != "BackupLog" {
        return Err(malformed(PART, "root element is not BackupLog"));
    }
    let text = |e: &Element, n: &str| e.child_text(n).map(str::to_string);
    let mut file_groups = Vec::new();
    for groups in root.children_named("FileGroups") {
        for g in groups.children_named("FileGroup") {
            let mut files = Vec::new();
            for list in g.children_named("FileList") {
                for bf in list.children_named("BackupFile") {
                    files.push(LogFile {
                        path: required_text(bf, "Path", PART)?,
                        storage_path: required_text(bf, "StoragePath", PART)?,
                        last_write_time: num_field(bf, "LastWriteTime", PART)?,
                        size: num_field(bf, "Size", PART)?,
                    });
                }
            }
            file_groups.push(FileGroup {
                class: num_field(g, "Class", PART)?,
                id: text(g, "ID"),
                name: text(g, "Name"),
                persist_location_path: text(g, "PersistLocationPath"),
                files,
            });
        }
    }
    Ok(BackupLog {
        sync_version: root
            .child_text("BackupRestoreSyncVersion")
            .map(|s| s.trim().to_string()),
        object_name: text(&root, "ObjectName"),
        object_id: text(&root, "ObjectId"),
        is_tabular: bool_field(&root, "IsTabular", PART)?,
        file_groups,
    })
}

/// Strips the group's persist location from an absolute server path and normalizes
/// separators. Falls back to the last segment when the path is outside that folder (a sibling
/// whose name merely starts with the folder's, such as `a.0.db.xml` next to `a.0.db`, is outside).
fn logical_path(original: &str, persist_root: Option<&str>) -> String {
    let stripped = persist_root
        .map(|root| root.trim_end_matches(['\\', '/']))
        .filter(|root| !root.is_empty())
        .and_then(|root| {
            let head = original.get(..root.len())?;
            head.eq_ignore_ascii_case(root)
                .then(|| original.get(root.len()..))
                .flatten()
        })
        .filter(|rest| rest.starts_with(['\\', '/']))
        .map(|rest| rest.trim_start_matches(['\\', '/']))
        .filter(|rest| !rest.is_empty());
    let rel = match stripped {
        Some(rest) => rest,
        None => original.rsplit(['\\', '/']).next().unwrap_or(original),
    };
    rel.replace('\\', "/")
}

/// Joins BackupLog records to virtual directory entries by storage path. `compressed`: the
/// header's `ApplyCompression`; the files the BackupLog describes are then stored compressed and
/// their log `Size` (the decompressed size) may exceed the stored size.
fn join(
    dir: &[DirEntry],
    log_index: usize,
    log: &BackupLog,
    compressed: bool,
) -> Result<(Vec<FileEntry>, usize), Error> {
    let by_storage: HashMap<String, usize> = dir
        .iter()
        .enumerate()
        .map(|(i, d)| (d.path.to_ascii_uppercase(), i))
        .collect();
    let mut described: Vec<Option<(&LogFile, Option<u32>, String)>> = vec![None; dir.len()];
    let mut unmatched = 0usize;
    for group in &log.file_groups {
        for rec in &group.files {
            let slot = by_storage
                .get(&rec.storage_path.to_ascii_uppercase())
                .and_then(|&i| described.get_mut(i));
            match slot {
                Some(slot @ None) => {
                    let path = logical_path(&rec.path, group.persist_location_path.as_deref());
                    *slot = Some((rec, group.class, path));
                }
                _ => unmatched += 1,
            }
        }
    }

    let mut files = Vec::with_capacity(dir.len());
    for (i, (d, desc)) in dir.iter().zip(described).enumerate() {
        let entry = match desc {
            Some((rec, class, path)) => {
                let size = match rec.size {
                    Some(s) => usize::try_from(s)
                        .ok()
                        .filter(|&s| compressed || s <= d.size)
                        .ok_or_else(|| {
                            malformed("BackupLog", "file Size exceeds its virtual directory Size")
                        })?,
                    None => d.size,
                };
                FileEntry {
                    kind: FileKind::from_name(path.rsplit('/').next().unwrap_or(&path)),
                    system_table: SystemTable::detect(&path),
                    path,
                    storage_path: d.path.clone(),
                    original_path: Some(rec.path.clone()),
                    offset: d.offset,
                    size,
                    stored_size: d.size,
                    last_write_time: rec.last_write_time.or(d.last_write_time),
                    file_group_class: class,
                    compressed,
                }
            }
            None => {
                let kind = if i == log_index {
                    FileKind::BackupLog
                } else if d.path == ADDITIONAL_LOG_NAME {
                    FileKind::AdditionalLog
                } else if d.path == PARTITIONS_NAME {
                    FileKind::Partitions
                } else {
                    FileKind::from_name(&d.path)
                };
                let path = d.path.replace('\\', "/");
                FileEntry {
                    system_table: SystemTable::detect(&path),
                    path,
                    storage_path: d.path.clone(),
                    original_path: None,
                    offset: d.offset,
                    size: d.size,
                    stored_size: d.size,
                    kind,
                    last_write_time: d.last_write_time,
                    file_group_class: None,
                    compressed: false,
                }
            }
        };
        files.push(entry);
    }
    Ok((files, unmatched))
}

/// The index of a backup read as a stream, too large to hold in memory
/// (`docs/formats/abf.md`, "Streamed backups"): the caller reads the header page, then the
/// virtual directory ([`Header::directory_offset`] / [`Header::directory_size`]), then the
/// BackupLog ([`streamed_log_range`]), and gets the same file list as [`Abf::files`] without
/// the file data. Offsets are checked against the end of the directory, the end of the stream.
#[derive(Debug, Clone)]
pub struct StreamedIndex {
    pub header: Header,
    pub log: BackupLog,
    pub files: Vec<FileEntry>,
}

impl StreamedIndex {
    /// Finds a file as [`Abf::entry`] does.
    pub fn entry(&self, path: &str) -> Option<&FileEntry> {
        let wanted = path.replace('\\', "/");
        self.files
            .iter()
            .find(|f| f.path == wanted)
            .or_else(|| self.files.iter().find(|f| f.path.eq_ignore_ascii_case(&wanted)))
            .or_else(|| {
                self.files
                    .iter()
                    .find(|f| f.storage_path.eq_ignore_ascii_case(path))
            })
    }

    /// The `metadata.sqlitedb` entry, found by file name (case-insensitive).
    pub fn metadata_entry(&self) -> Option<&FileEntry> {
        self.files
            .iter()
            .find(|f| f.name().eq_ignore_ascii_case(METADATA_NAME))
    }
}

/// Parses the header page of a streamed backup (the first [`crate::PAGE_SIZE`] bytes).
pub fn streamed_header(page: &[u8]) -> Result<Header, Error> {
    let header = header::parse(page)?;
    if header.encrypted {
        return Err(Error::Unsupported("encrypted backups".into()));
    }
    if header.apply_compression {
        return Err(Error::Unsupported(
            "whole-stream compressed (ApplyCompression) backups".into(),
        ));
    }
    Ok(header)
}

fn streamed_len(header: &Header) -> Result<u64, Error> {
    header
        .directory_offset
        .checked_add(header.directory_size)
        .ok_or_else(|| malformed("header", "directory range overflows"))
}

/// The (offset, size) of the BackupLog, from the virtual directory bytes.
pub fn streamed_log_range(header: &Header, directory: &[u8]) -> Result<(u64, u64), Error> {
    let dir = parse_directory(streamed_len(header)?, directory)?;
    let log = log_index(&dir)
        .and_then(|i| dir.get(i))
        .ok_or_else(|| malformed("virtual directory", "no entries"))?;
    let as_u64 = |n: usize| u64::try_from(n).map_err(|_| malformed("virtual directory", "offset"));
    Ok((as_u64(log.offset)?, as_u64(log.size)?))
}

/// The file list of a streamed backup, from its virtual directory and BackupLog bytes.
pub fn streamed_index(header: Header, directory: &[u8], log: &[u8]) -> Result<StreamedIndex, Error> {
    let dir = parse_directory(streamed_len(&header)?, directory)?;
    let log_at = log_index(&dir).ok_or_else(|| malformed("virtual directory", "no entries"))?;
    let parsed = parse_log(log)?;
    let (files, _) = join(&dir, log_at, &parsed, false)?;
    Ok(StreamedIndex {
        header,
        log: parsed,
        files,
    })
}

fn log_index(dir: &[DirEntry]) -> Option<usize> {
    dir.iter()
        .position(|d| d.path == LOG_NAME)
        .or_else(|| dir.len().checked_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_paths() {
        let root = Some(r"\\?\C:\X\Data\a.0.db");
        assert_eq!(
            logical_path(r"\\?\C:\X\Data\a.0.db\T (1).tbl\2.prt\f.idf", root),
            "T (1).tbl/2.prt/f.idf"
        );
        assert_eq!(
            logical_path(r"\\?\c:\x\data\A.0.DB\metadata.sqlitedb", root),
            "metadata.sqlitedb"
        );
        assert_eq!(logical_path(r"\\?\C:\Elsewhere\a.0.db.xml", root), "a.0.db.xml");
        assert_eq!(logical_path(r"\\?\C:\X\Data\a.0.db", root), "a.0.db");
        // A sibling whose name starts with the root's (Excel's `<db>.0.db.xml` next to the
        // `<db>.0.db` folder) is outside it, not the file ".xml".
        assert_eq!(logical_path(r"\\?\C:\X\Data\a.0.db.xml", root), "a.0.db.xml");
        assert_eq!(
            logical_path(r"\\?\C:\X\Data\a.0.db.xml", Some(r"\\?\C:\X\Data\a.0.db\")),
            "a.0.db.xml"
        );
        assert_eq!(logical_path("plain", None), "plain");
        // A multi-byte character straddling the root length must not panic.
        assert_eq!(logical_path("é", Some("x")), "é");
    }

    #[test]
    fn system_table_detection() {
        assert_eq!(
            SystemTable::detect("H$T (1)$C (2)$(3).tbl/4.prt/x.idf"),
            Some(SystemTable::AttributeHierarchy)
        );
        assert_eq!(
            SystemTable::detect("R$a.tbl/x.idf"),
            Some(SystemTable::Relationship)
        );
        assert_eq!(
            SystemTable::detect("1.U$Table1$H.idf"),
            Some(SystemTable::UserHierarchy)
        );
        assert_eq!(SystemTable::detect("T (1).tbl/2.prt/1.T (1).H$x.idf"), None);
        assert_eq!(SystemTable::detect("4.Table1.Cat.0.idf"), None);
    }

    #[test]
    fn kinds_from_names() {
        assert_eq!(FileKind::from_name("Metadata.SQLiteDB"), FileKind::MetadataSqlite);
        assert_eq!(FileKind::from_name("0.CryptKey.bin"), FileKind::CryptKey);
        assert_eq!(FileKind::from_name("a.0.idfmeta"), FileKind::IdfMeta);
        assert_eq!(FileKind::from_name("a.0.idf"), FileKind::Idf);
        assert_eq!(FileKind::from_name("a.dictionary"), FileKind::Dictionary);
        assert_eq!(FileKind::from_name("a.hidx"), FileKind::HashIndex);
        assert_eq!(FileKind::from_name("x.db.xml"), FileKind::Xml);
        assert_eq!(FileKind::from_name("x.bin"), FileKind::Other);
    }
}
