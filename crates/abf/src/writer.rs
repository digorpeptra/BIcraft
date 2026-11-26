//! The writer: re-packs a parsed backup with files replaced, removed or added
//! (see `docs/formats/abf.md`, "Writer").
//!
//! The three XML documents (header `BackupLog`, files-section `BackupLog`, `VirtualDirectory`)
//! are not re-serialized from a model: the writer patches the text of the few numeric
//! elements that change (sizes, offsets, counts), cuts out the entries of removed files, inserts
//! entries for added ones, and leaves every other byte as it was. An
//! unchanged document is copied verbatim, so writing a canonical Power BI backup without
//! changes reproduces it byte for byte.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use crate::abf::{Abf, METADATA_NAME};
use crate::header::HEADER_XML_OFFSET;
use crate::text::{TextEncoding, decode_xml, detect_encoding, encode, utf16_len_before_nul};
use crate::xml::{self, Element};
use crate::{Error, FileKind, PAGE_SIZE};

/// A number stored as the text of an element, with the byte range of that text.
#[derive(Debug, Clone)]
struct NumField {
    span: Range<usize>,
    value: u64,
}

/// Replacement text for a byte range of a document.
type Patch = (Range<usize>, String);

/// An XML document of the original stream, kept for patching.
#[derive(Debug, Clone)]
struct XmlDoc<'a> {
    original: &'a [u8],
    text: String,
    encoding: TextEncoding,
    /// Re-encoding the decoded text gives back `original`: patching changes nothing else.
    faithful: bool,
}

impl<'a> XmlDoc<'a> {
    fn new(original: &'a [u8], part: &'static str) -> Result<(XmlDoc<'a>, Element), Error> {
        let text = decode_xml(original, part)?;
        let encoding = detect_encoding(original);
        let faithful = encode(&text, encoding) == original;
        let root = xml::parse(&text, part)?;
        Ok((
            XmlDoc {
                original,
                text,
                encoding,
                faithful,
            },
            root,
        ))
    }

    /// The document with `patches` applied: the original bytes when there are none.
    fn render(&self, mut patches: Vec<Patch>, part: &'static str) -> Result<Cow<'a, [u8]>, Error> {
        if patches.is_empty() {
            return Ok(Cow::Borrowed(self.original));
        }
        if !self.faithful {
            return Err(Error::Unsupported(format!(
                "the {part} does not re-encode byte for byte, so it cannot be patched"
            )));
        }
        // Insertions (empty ranges) go before a replacement starting at the same place.
        patches.sort_by_key(|(r, _)| (r.start, r.end));
        let bad = || Error::Write(format!("overlapping or out-of-range patches in the {part}"));
        let mut out = String::with_capacity(self.text.len().saturating_add(32 * patches.len()));
        let mut pos = 0usize;
        for (range, value) in &patches {
            if range.start < pos {
                return Err(bad());
            }
            out.push_str(self.text.get(pos..range.start).ok_or_else(bad)?);
            out.push_str(value);
            pos = range.end;
        }
        out.push_str(self.text.get(pos..).ok_or_else(bad)?);
        Ok(Cow::Owned(encode(&out, self.encoding)))
    }
}

/// Reads a numeric child element and the range of its text. `None` when absent.
fn num_child(e: &Element, name: &str, part: &'static str) -> Result<Option<NumField>, Error> {
    let Some(child) = e.child(name) else {
        return Ok(None);
    };
    let bad = |reason: &str| Error::Malformed {
        part,
        reason: format!("{name} {reason}"),
    };
    let span = child.content.clone().ok_or_else(|| bad("has no text"))?;
    let value = child.text.trim().parse().map_err(|_| bad("is not a number"))?;
    Ok(Some(NumField { span, value }))
}

fn required_num(e: &Element, name: &str, part: &'static str) -> Result<NumField, Error> {
    num_child(e, name, part)?.ok_or_else(|| Error::Malformed {
        part,
        reason: format!("no {name}"),
    })
}

/// Queues a patch when the stored value differs from `value`.
fn set(patches: &mut Vec<Patch>, field: &NumField, value: u64) {
    if field.value != value {
        patches.push((field.span.clone(), value.to_string()));
    }
}

/// The numeric fields of the header `BackupLog` that depend on the layout.
#[derive(Debug, Clone)]
struct HeaderFields {
    directory_offset: NumField,
    directory_size: NumField,
    file_count: Option<NumField>,
    data_offset: Option<NumField>,
}

/// The virtual-directory entry of an original file.
#[derive(Debug, Clone)]
struct DirEntry {
    size: NumField,
    offset: NumField,
    /// The whole `<BackupFile>` element.
    span: Range<usize>,
    /// Everything after `m_cbOffsetHeader` inside the element (`Delete`, timestamps), when the
    /// element starts with `Path`, `Size`, `m_cbOffsetHeader`. Copied into added entries.
    tail: Option<Range<usize>>,
}

/// One `BackupFile` record of the files-section BackupLog naming an original file.
#[derive(Debug, Clone)]
struct LogRecord {
    size: Option<NumField>,
    /// The whole `<BackupFile>` element.
    span: Range<usize>,
    /// Index into [`AbfWriter::groups`].
    group: usize,
}

/// A `FileGroup` of the files-section BackupLog.
#[derive(Debug, Clone)]
struct LogGroup {
    /// `PersistLocationPath`, unescaped: the absolute folder of the database files.
    persist_root: Option<String>,
    /// Where the content of its (last) `FileList` ends: records appended to the group go here.
    list_end: Option<usize>,
    /// The largest `LastWriteTime` of its records.
    max_write_time: Option<u64>,
}

#[derive(Debug, Clone)]
struct WriterFile<'a> {
    path: String,
    storage_path: String,
    kind: FileKind,
    /// The bytes stored in the files section (with any CRC end marker, when not replaced).
    data: Cow<'a, [u8]>,
    replaced: bool,
    removed: bool,
    dir: DirEntry,
    /// Every BackupLog record that names this file.
    log: Vec<LogRecord>,
}

/// A file added by [`AbfWriter::add`].
#[derive(Debug, Clone)]
struct AddedFile<'a> {
    path: String,
    storage_path: String,
    data: Cow<'a, [u8]>,
}

/// Re-packs a parsed [`Abf`], with files replaced, removed or added.
///
/// ```no_run
/// # fn demo(decompressed: &[u8], new_sqlite: Vec<u8>) -> Result<Vec<u8>, bicraft_abf::Error> {
/// let abf = bicraft_abf::Abf::parse(decompressed)?;
/// let mut writer = abf.to_writer()?;
/// writer.replace_metadata_sqlitedb(new_sqlite)?;
/// writer.remove("Sales (12).tbl/13.prt/1.Sales (12).Old (14).0.idf")?;
/// writer.add("Sales (12).tbl/13.prt/1.Sales (12).New (40).0.idf", vec![0u8; 16])?;
/// writer.finish()
/// # }
/// ```
///
/// Adding or removing a file only changes the container (directory entry, BackupLog record,
/// layout). The `StorageFolder`/`StorageFile` rows of `metadata.sqlitedb` that name the file are
/// the caller's job (`bicraft-metadata` produces them together with the file list).
#[derive(Debug, Clone)]
pub struct AbfWriter<'a> {
    /// The original header page (BOM, signature, XML, padding), possibly short.
    header_page: &'a [u8],
    header: XmlDoc<'a>,
    header_fields: HeaderFields,
    directory: XmlDoc<'a>,
    log: XmlDoc<'a>,
    log_index: usize,
    crc_markers: bool,
    files: Vec<WriterFile<'a>>,
    groups: Vec<LogGroup>,
    added: Vec<AddedFile<'a>>,
}

impl<'a> Abf<'a> {
    /// A writer holding every file of this backup, ready to [`AbfWriter::replace`],
    /// [`AbfWriter::remove`] or [`AbfWriter::add`] files and [`AbfWriter::finish`].
    pub fn to_writer(&self) -> Result<AbfWriter<'a>, Error> {
        AbfWriter::new(self)
    }
}

impl<'a> AbfWriter<'a> {
    /// Same as [`Abf::to_writer`].
    pub fn new(abf: &Abf<'a>) -> Result<AbfWriter<'a>, Error> {
        let bytes = abf.bytes;
        let header_page = bytes.get(..bytes.len().min(PAGE_SIZE)).unwrap_or_default();
        let xml_area = header_page.get(HEADER_XML_OFFSET..).unwrap_or_default();
        let header_xml = xml_area.get(..utf16_len_before_nul(xml_area)).unwrap_or_default();
        let (header, root) = XmlDoc::new(header_xml, "header")?;
        let header_fields = HeaderFields {
            directory_offset: required_num(&root, "m_cbOffsetHeader", "header")?,
            directory_size: required_num(&root, "DataSize", "header")?,
            file_count: num_child(&root, "Files", "header")?,
            data_offset: num_child(&root, "m_cbOffsetData", "header")?,
        };

        let h = &abf.header;
        let dir_bytes = usize::try_from(h.directory_offset)
            .ok()
            .zip(usize::try_from(h.directory_size).ok())
            .and_then(|(o, n)| bytes.get(o..o.checked_add(n)?))
            .ok_or_else(|| malformed("virtual directory", "out of bounds"))?;
        let (directory, dir_root) = XmlDoc::new(dir_bytes, "virtual directory")?;
        let dir_entries: Vec<&Element> = dir_root.children_named("BackupFile").collect();
        if dir_entries.len() != abf.files.len() {
            return Err(malformed(
                "virtual directory",
                "entry count differs from the parse",
            ));
        }

        let log_index = abf
            .files
            .iter()
            .position(|f| f.kind == FileKind::BackupLog)
            .ok_or_else(|| malformed("virtual directory", "no BackupLog"))?;

        let mut files = Vec::with_capacity(abf.files.len());
        for (f, d) in abf.files.iter().zip(&dir_entries) {
            const PART: &str = "virtual directory";
            if d.child_text("Path") != Some(f.storage_path.as_str()) {
                return Err(malformed(PART, "entry order differs from the parse"));
            }
            let data = f
                .offset
                .checked_add(f.stored_size)
                .and_then(|end| bytes.get(f.offset..end))
                .ok_or_else(|| malformed(PART, "file data out of bounds"))?;
            files.push(WriterFile {
                path: f.path.clone(),
                storage_path: f.storage_path.clone(),
                kind: f.kind,
                data: Cow::Borrowed(data),
                replaced: false,
                removed: false,
                dir: DirEntry {
                    size: required_num(d, "Size", PART)?,
                    offset: required_num(d, "m_cbOffsetHeader", PART)?,
                    span: d.span.clone(),
                    tail: dir_tail(d),
                },
                log: Vec::new(),
            });
        }

        let log_bytes = match files.get(log_index).map(|f| &f.data) {
            Some(Cow::Borrowed(b)) => *b,
            _ => return Err(malformed("BackupLog", "missing")),
        };
        let (log, log_root) = XmlDoc::new(log_bytes, "BackupLog")?;
        let by_storage: HashMap<String, usize> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.storage_path.to_ascii_uppercase(), i))
            .collect();
        let mut groups = Vec::new();
        for list_of_groups in log_root.children_named("FileGroups") {
            for group in list_of_groups.children_named("FileGroup") {
                let gi = groups.len();
                let mut info = LogGroup {
                    persist_root: group.child_text("PersistLocationPath").map(str::to_string),
                    list_end: None,
                    max_write_time: None,
                };
                for list in group.children_named("FileList") {
                    info.list_end = list.content.as_ref().map(|c| c.end);
                    for rec in list.children_named("BackupFile") {
                        if let Ok(Some(t)) = num_child(rec, "LastWriteTime", "BackupLog") {
                            info.max_write_time =
                                Some(info.max_write_time.map_or(t.value, |m| m.max(t.value)));
                        }
                        let Some(storage) = rec.child_text("StoragePath") else {
                            continue;
                        };
                        if let Some(file) = by_storage
                            .get(&storage.to_ascii_uppercase())
                            .and_then(|&i| files.get_mut(i))
                        {
                            file.log.push(LogRecord {
                                size: num_child(rec, "Size", "BackupLog")?,
                                span: rec.span.clone(),
                                group: gi,
                            });
                        }
                    }
                }
                groups.push(info);
            }
        }

        Ok(AbfWriter {
            header_page,
            header,
            header_fields,
            directory,
            log,
            log_index,
            crc_markers: h.crc_markers,
            files,
            groups,
            added: Vec::new(),
        })
    }

    /// Number of files the output will hold (virtual directory entries, the BackupLog
    /// included): the original ones minus the removed ones plus the added ones.
    pub fn len(&self) -> usize {
        self.files
            .iter()
            .filter(|f| !f.removed)
            .count()
            .saturating_add(self.added.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Replaces the content of one file. `path` is resolved like [`Abf::entry`] (logical
    /// path, exact then ASCII case-insensitive with `\` as `/`, then storage name), except
    /// that a path matching several files is an error instead of picking the first. A file
    /// added by [`add`](Self::add) can be replaced by its path too.
    ///
    /// The BackupLog cannot be replaced (the writer generates it), and nothing can be
    /// replaced in a backup with CRC end markers (the marker format is unknown).
    /// Timestamps (`LastWriteTime`, `CreatedTimestamp`, `Access`) keep their original
    /// values, so the output depends only on the input and the replacements.
    pub fn replace(&mut self, path: &str, data: impl Into<Cow<'a, [u8]>>) -> Result<(), Error> {
        let data = data.into();
        match self.resolve(path)? {
            Target::Original(index) => self.replace_at(index, data),
            Target::Added(index) => {
                let added = self.added.get_mut(index).ok_or(Error::FileNotFound)?;
                added.data = data;
                Ok(())
            }
        }
    }

    /// Replaces `metadata.sqlitedb` (found by file name, case-insensitive).
    pub fn replace_metadata_sqlitedb(&mut self, data: impl Into<Cow<'a, [u8]>>) -> Result<(), Error> {
        let index = unique(
            self.files
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.removed && file_name(&f.path).eq_ignore_ascii_case(METADATA_NAME))
                .map(|(i, _)| i),
        )
        .unwrap_or(Err(Error::FileNotFound))?;
        self.replace_at(index, data.into())
    }

    /// Removes one file: its data, its virtual-directory entry and every BackupLog record
    /// naming it. `path` is resolved as in [`replace`](Self::replace); removing a file added
    /// by [`add`](Self::add) cancels the addition.
    ///
    /// The BackupLog cannot be removed, nor anything in a backup with CRC end markers.
    pub fn remove(&mut self, path: &str) -> Result<(), Error> {
        self.check_editable("removing files")?;
        match self.resolve(path)? {
            Target::Original(index) => {
                let file = self.files.get_mut(index).ok_or(Error::FileNotFound)?;
                if file.kind == FileKind::BackupLog {
                    return Err(Error::Unsupported(
                        "removing the BackupLog: the writer generates it".into(),
                    ));
                }
                file.removed = true;
            }
            Target::Added(index) => {
                if index < self.added.len() {
                    self.added.remove(index);
                }
            }
        }
        Ok(())
    }

    /// Adds a file at logical `path` (relative to the database folder, `/` or `\` separated,
    /// e.g. `Sales (12).tbl/13.prt/1.Sales (12).Amount (14).0.idf`).
    ///
    /// The file gets a fresh storage name (20 upper-case hex digits shaped like the start of a
    /// version-4 GUID, as Power BI's are; derived from the path, so the output is
    /// deterministic), a virtual-directory entry and a BackupLog record in the file group that
    /// holds `metadata.sqlitedb` (else the last group), with the absolute path built from the
    /// group's `PersistLocationPath`. New files are stored just before `metadata.sqlitedb`
    /// (else just before the BackupLog), in the order they were added. Timestamps are copied:
    /// the directory ones from that neighbor's entry, the log `LastWriteTime` is the group's
    /// latest.
    ///
    /// Errors: [`Error::PathExists`] when a file (not removed) already has the path (ASCII
    /// case-insensitive); [`Error::Unsupported`] for an empty, absolute or `..` path, for a
    /// backup with CRC end markers or without a BackupLog file group.
    pub fn add(&mut self, path: &str, data: impl Into<Cow<'a, [u8]>>) -> Result<(), Error> {
        self.check_editable("adding files")?;
        let path = normalize_new_path(path)?;
        let taken = self
            .files
            .iter()
            .filter(|f| !f.removed)
            .map(|f| f.path.as_str())
            .chain(self.added.iter().map(|a| a.path.as_str()))
            .any(|p| p.eq_ignore_ascii_case(&path));
        if taken {
            return Err(Error::PathExists);
        }
        if self.groups.is_empty() {
            return Err(Error::Unsupported(
                "adding a file to a backup whose BackupLog has no file group".into(),
            ));
        }
        let storage_path = self.fresh_storage_name(&path);
        self.added.push(AddedFile {
            path,
            storage_path,
            data: data.into(),
        });
        Ok(())
    }

    fn check_editable(&self, what: &str) -> Result<(), Error> {
        if self.crc_markers {
            return Err(Error::Unsupported(format!(
                "{what} in a backup with CRC end markers"
            )));
        }
        Ok(())
    }

    fn fresh_storage_name(&self, path: &str) -> String {
        let used = |name: &str| {
            self.files
                .iter()
                .map(|f| f.storage_path.as_str())
                .chain(self.added.iter().map(|a| a.storage_path.as_str()))
                .chain([LOG_STORAGE_NAME])
                .any(|s| s.eq_ignore_ascii_case(name))
        };
        let mut salt = 0u64;
        loop {
            let name = storage_name(path, salt);
            if !used(&name) {
                return name;
            }
            salt = salt.wrapping_add(1);
        }
    }

    fn replace_at(&mut self, index: usize, data: Cow<'a, [u8]>) -> Result<(), Error> {
        self.check_editable("replacing files")?;
        let file = self.files.get_mut(index).ok_or(Error::FileNotFound)?;
        if file.kind == FileKind::BackupLog {
            return Err(Error::Unsupported(
                "replacing the BackupLog: the writer generates it".into(),
            ));
        }
        file.data = data;
        file.replaced = true;
        Ok(())
    }

    fn resolve(&self, path: &str) -> Result<Target, Error> {
        let wanted = path.replace('\\', "/");
        let pick = |pred: &dyn Fn(&str, &str) -> bool| -> Option<Result<Target, Error>> {
            let originals = self
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.removed && pred(&f.path, &f.storage_path))
                .map(|(i, _)| Target::Original(i));
            let added = self
                .added
                .iter()
                .enumerate()
                .filter(|(_, a)| pred(&a.path, &a.storage_path))
                .map(|(i, _)| Target::Added(i));
            let mut all = originals.chain(added);
            let first = all.next()?;
            let more = all.count();
            Some(if more == 0 {
                Ok(first)
            } else {
                Err(Error::AmbiguousPath(more.saturating_add(1)))
            })
        };
        pick(&|p, _| p == wanted)
            .or_else(|| pick(&|p, _| p.eq_ignore_ascii_case(&wanted)))
            .or_else(|| pick(&|_, s| s.eq_ignore_ascii_case(path)))
            .unwrap_or(Err(Error::FileNotFound))
    }

    /// Where added files go: before the last `metadata.sqlitedb` that is kept, else before the
    /// BackupLog. Returns the index in `files`.
    fn anchor(&self) -> usize {
        self.files
            .iter()
            .enumerate()
            .rev()
            .find(|(_, f)| !f.removed && f.kind == FileKind::MetadataSqlite)
            .map_or(self.log_index, |(i, _)| i)
    }

    /// Writes the backup: header page, files back to back from offset 4096 (the original ones
    /// in their order, minus the removed ones, with the added ones before the anchor), zero
    /// padding to the next page, the virtual directory, zero padding to the next page. Every
    /// size, offset and count is updated; the output is parsed back before it is returned.
    pub fn finish(&self) -> Result<Vec<u8>, Error> {
        let anchor = self.anchor();
        let anchor_file = self
            .files
            .get(anchor)
            .ok_or_else(|| Error::Write("no anchor for added files".into()))?;

        // 1. The BackupLog: sizes of replaced files, records of removed and added files.
        let mut log_patches = Vec::new();
        for f in &self.files {
            for rec in &f.log {
                if f.removed {
                    log_patches.push((rec.span.clone(), String::new()));
                } else if f.replaced
                    && let Some(size) = &rec.size
                {
                    set(&mut log_patches, size, f.data.len() as u64);
                }
            }
        }
        if !self.added.is_empty() {
            let (group, at) = self.log_insertion(anchor_file)?;
            let root = group.persist_root.as_deref().unwrap_or_default();
            let mut records = String::new();
            for a in &self.added {
                records.push_str("<BackupFile><Path>");
                records.push_str(&escape(&absolute_path(root, &a.path)));
                records.push_str("</Path><StoragePath>");
                records.push_str(&a.storage_path);
                records.push_str("</StoragePath>");
                if let Some(t) = group.max_write_time {
                    records.push_str(&format!("<LastWriteTime>{t}</LastWriteTime>"));
                }
                records.push_str(&format!("<Size>{}</Size></BackupFile>", a.data.len()));
            }
            log_patches.push((at..at, records));
        }
        let log = self.log.render(log_patches, "BackupLog")?;

        // 2. Layout of the files section, in output order.
        let order = self.output_order(anchor);
        let sizes: Vec<u64> = order
            .iter()
            .map(|item| match item {
                Item::Original(i) if *i == self.log_index => log.len() as u64,
                Item::Original(i) => self.files.get(*i).map_or(0, |f| f.data.len() as u64),
                Item::Added(i) => self.added.get(*i).map_or(0, |a| a.data.len() as u64),
            })
            .collect();
        let (offsets, files_end) = file_offsets(&sizes)?;

        // 3. The virtual directory.
        let mut dir_patches = Vec::new();
        let mut inserted = String::new();
        let tail = anchor_file
            .dir
            .tail
            .clone()
            .and_then(|r| self.directory.text.get(r))
            .unwrap_or("<Delete>false</Delete>");
        for ((item, &offset), &size) in order.iter().zip(&offsets).zip(&sizes) {
            match item {
                Item::Original(i) => {
                    if let Some(f) = self.files.get(*i) {
                        set(&mut dir_patches, &f.dir.size, size);
                        set(&mut dir_patches, &f.dir.offset, offset);
                    }
                }
                Item::Added(i) => {
                    if let Some(a) = self.added.get(*i) {
                        inserted.push_str(&format!(
                            "<BackupFile><Path>{}</Path><Size>{size}</Size>\
                             <m_cbOffsetHeader>{offset}</m_cbOffsetHeader>{tail}</BackupFile>",
                            a.storage_path
                        ));
                    }
                }
            }
        }
        for f in self.files.iter().filter(|f| f.removed) {
            dir_patches.push((f.dir.span.clone(), String::new()));
        }
        if !inserted.is_empty() {
            let at = anchor_file.dir.span.start;
            dir_patches.push((at..at, inserted));
        }
        let directory = self.directory.render(dir_patches, "virtual directory")?;
        let dir_offset = align_page(files_end)?;
        let end = dir_offset
            .checked_add(directory.len() as u64)
            .and_then(|e| align_page(e).ok())
            .ok_or_else(too_large)?;

        // 4. The header.
        let hf = &self.header_fields;
        let mut header_patches = Vec::new();
        set(&mut header_patches, &hf.directory_offset, dir_offset);
        set(&mut header_patches, &hf.directory_size, directory.len() as u64);
        if let Some(n) = &hf.file_count {
            set(&mut header_patches, n, order.len() as u64);
        }
        if let Some(n) = &hf.data_offset {
            set(&mut header_patches, n, PAGE_SIZE as u64);
        }
        let header_unchanged = header_patches.is_empty();
        let header_xml = self.header.render(header_patches, "header")?;

        // 5. Assemble.
        let total = usize::try_from(end).map_err(|_| too_large())?;
        let mut out = Vec::with_capacity(total);
        if header_unchanged {
            out.extend_from_slice(self.header_page);
        } else {
            out.extend_from_slice(self.header_page.get(..HEADER_XML_OFFSET).unwrap_or_default());
            out.extend_from_slice(&header_xml);
        }
        if out.len() > PAGE_SIZE {
            return Err(Error::Write(
                "the header XML does not fit in the header page".into(),
            ));
        }
        out.resize(PAGE_SIZE, 0);
        for item in &order {
            match item {
                Item::Original(i) if *i == self.log_index => out.extend_from_slice(&log),
                Item::Original(i) => {
                    if let Some(f) = self.files.get(*i) {
                        out.extend_from_slice(&f.data);
                    }
                }
                Item::Added(i) => {
                    if let Some(a) = self.added.get(*i) {
                        out.extend_from_slice(&a.data);
                    }
                }
            }
        }
        let dir_start = usize::try_from(dir_offset).map_err(|_| too_large())?;
        out.resize(dir_start, 0);
        out.extend_from_slice(&directory);
        out.resize(total, 0);

        // 6. Self-check.
        let back = Abf::parse(&out).map_err(|e| Error::Write(format!("the output does not parse: {e}")))?;
        if back.files.len() != order.len() {
            return Err(Error::Write("the output lost files".into()));
        }
        for a in &self.added {
            let found = back
                .files
                .iter()
                .find(|f| f.storage_path == a.storage_path)
                .is_some_and(|f| f.path == a.path && f.size == a.data.len());
            if !found {
                return Err(Error::Write(
                    "an added file does not read back at its path".into(),
                ));
            }
        }
        Ok(out)
    }

    /// The file group and text position where BackupLog records of added files go: before the
    /// anchor's record (`metadata.sqlitedb`), else at the end of the last group's file list.
    fn log_insertion(&self, anchor: &WriterFile<'a>) -> Result<(&LogGroup, usize), Error> {
        if anchor.kind != FileKind::BackupLog
            && let Some(rec) = anchor.log.first()
            && let Some(group) = self.groups.get(rec.group)
        {
            return Ok((group, rec.span.start));
        }
        self.groups
            .iter()
            .rev()
            .find_map(|g| g.list_end.map(|end| (g, end)))
            .ok_or_else(|| {
                Error::Unsupported("adding a file to a backup whose BackupLog has no file list".into())
            })
    }

    /// The files of the output, in stream order.
    fn output_order(&self, anchor: usize) -> Vec<Item> {
        let mut order = Vec::with_capacity(self.len());
        for (i, f) in self.files.iter().enumerate() {
            if i == anchor {
                order.extend((0..self.added.len()).map(Item::Added));
            }
            if !f.removed {
                order.push(Item::Original(i));
            }
        }
        order
    }
}

/// A file resolved by path: an original one (index into `files`) or an added one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Original(usize),
    Added(usize),
}

/// A file of the output, in stream order.
#[derive(Debug, Clone, Copy)]
enum Item {
    Original(usize),
    Added(usize),
}

/// Virtual-directory name of the BackupLog; never handed out to an added file.
const LOG_STORAGE_NAME: &str = "LOG";

/// The range after `m_cbOffsetHeader` inside a directory `BackupFile` element, when the
/// element starts with `Path`, `Size`, `m_cbOffsetHeader` (the Power BI shape).
fn dir_tail(e: &Element) -> Option<Range<usize>> {
    let names: Vec<&str> = e.children.iter().take(3).map(|c| c.name.as_str()).collect();
    if names != ["Path", "Size", "m_cbOffsetHeader"] {
        return None;
    }
    let start = e.children.get(2)?.span.end;
    let end = e.content.as_ref()?.end;
    (start <= end).then_some(start..end)
}

/// Checks and normalizes the logical path of a file to add: `/` separators, relative, no
/// empty, `.` or `..` segment, no control character.
fn normalize_new_path(path: &str) -> Result<String, Error> {
    let p = path.replace('\\', "/");
    let bad = |why: &str| {
        Err(Error::Unsupported(format!(
            "cannot add a file at this path: {why}"
        )))
    };
    if p.is_empty() {
        return bad("empty path");
    }
    if p.chars().any(char::is_control) {
        return bad("control character");
    }
    if p.contains(':') {
        return bad("drive or stream separator");
    }
    if p.split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return bad("absolute path or empty, `.` or `..` segment");
    }
    if p.eq_ignore_ascii_case(LOG_STORAGE_NAME) {
        return bad("reserved name");
    }
    Ok(p)
}

/// `root\rel` with `\` separators (the absolute server path of a BackupLog record).
fn absolute_path(root: &str, rel: &str) -> String {
    let rel = rel.replace('/', "\\");
    if root.is_empty() {
        rel
    } else if root.ends_with('\\') {
        format!("{root}{rel}")
    } else {
        format!("{root}\\{rel}")
    }
}

/// Escapes text for an XML element.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A storage name for an added file: 20 upper-case hex digits, digit 13 = `4` and digit 17 in
/// `8`–`B`, like the first 80 bits of a version-4 GUID (the shape of every Power BI storage
/// name observed). Deterministic in `path` and `salt`.
pub(crate) fn storage_name(path: &str, salt: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for b in path.to_ascii_uppercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let a = splitmix(h);
    let b = splitmix(a);
    let variant = ['8', '9', 'A', 'B'];
    let mut digits: Vec<char> = format!("{a:016X}{:04X}", b & 0xffff).chars().collect();
    if let Some(d) = digits.get_mut(12) {
        *d = '4';
    }
    if let (Some(d), Some(&v)) = (digits.get_mut(16), variant.get(((b >> 16) & 3) as usize)) {
        *d = v;
    }
    digits.into_iter().collect()
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// `Some(Ok(i))` for exactly one index, `Some(Err)` for several, `None` for none.
fn unique(mut indices: impl Iterator<Item = usize>) -> Option<Result<usize, Error>> {
    let first = indices.next()?;
    let more = indices.count();
    Some(if more == 0 {
        Ok(first)
    } else {
        Err(Error::AmbiguousPath(more.saturating_add(1)))
    })
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn malformed(part: &'static str, reason: &str) -> Error {
    Error::Malformed {
        part,
        reason: reason.to_string(),
    }
}

fn too_large() -> Error {
    Error::Write("the backup would exceed the addressable size".into())
}

/// Offsets of files stored back to back from the end of the header page, and the end of
/// the last one. Checked: hostile sizes return an error.
pub(crate) fn file_offsets(sizes: &[u64]) -> Result<(Vec<u64>, u64), Error> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut pos = PAGE_SIZE as u64;
    for &size in sizes {
        offsets.push(pos);
        pos = pos.checked_add(size).ok_or_else(too_large)?;
    }
    Ok((offsets, pos))
}

/// Rounds up to a multiple of the page size.
pub(crate) fn align_page(n: u64) -> Result<u64, Error> {
    let page = PAGE_SIZE as u64;
    n.checked_next_multiple_of(page).ok_or_else(too_large)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn patches_only_the_given_ranges() {
        let bytes = utf16("<a><n> 7 </n><m>x</m></a>");
        let (doc, root) = XmlDoc::new(&bytes, "t").unwrap();
        assert!(doc.faithful);
        let n = required_num(&root, "n", "t").unwrap();
        assert_eq!(n.value, 7);
        let mut patches = Vec::new();
        set(&mut patches, &n, 7);
        assert!(patches.is_empty(), "an unchanged value is not rewritten");
        assert!(matches!(doc.render(patches, "t").unwrap(), Cow::Borrowed(_)));
        let mut patches = Vec::new();
        set(&mut patches, &n, 1234);
        assert_eq!(
            doc.render(patches, "t").unwrap().into_owned(),
            utf16("<a><n>1234</n><m>x</m></a>")
        );
        assert!(required_num(&root, "m", "t").is_err());
        assert!(required_num(&root, "z", "t").is_err());
        let selfclosing = utf16("<a><n/></a>");
        let (_, root) = XmlDoc::new(&selfclosing, "t").unwrap();
        assert!(num_child(&root, "n", "t").is_err());
    }

    #[test]
    fn unfaithful_documents_are_copied_but_not_patched() {
        // An unpaired surrogate decodes lossily, so re-encoding would change it.
        let mut bytes = utf16("<a><n>1</n><x>");
        bytes.extend_from_slice(&[0x00, 0xD8]);
        bytes.extend(utf16("</x></a>"));
        let (doc, root) = XmlDoc::new(&bytes, "t").unwrap();
        assert!(!doc.faithful);
        assert_eq!(doc.render(Vec::new(), "t").unwrap(), &bytes[..]);
        let n = required_num(&root, "n", "t").unwrap();
        let mut patches = Vec::new();
        set(&mut patches, &n, 2);
        assert!(matches!(doc.render(patches, "t"), Err(Error::Unsupported(_))));
    }

    #[test]
    fn overlapping_patches_are_rejected() {
        let (doc, _) = XmlDoc::new(b"<a>12345</a>", "t").unwrap();
        let patches = vec![(3..6, "x".to_string()), (4..7, "y".to_string())];
        assert!(matches!(doc.render(patches, "t"), Err(Error::Write(_))));
        let out_of_range = vec![(3..99, "x".to_string())];
        assert!(doc.render(out_of_range, "t").is_err());
    }
}
