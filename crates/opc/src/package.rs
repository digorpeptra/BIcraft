//! The ZIP container. Parts are kept in archive order so a read → write round trip preserves
//! layout; each part remembers whether it was stored or deflated.

use std::io::{Cursor, Read, Seek, Write};
use std::sync::Arc;

use zip::{CompressionMethod, ZipArchive};

use crate::deferred::{ArchiveSource, Deferred, FileSource, ReadSeek};
use crate::{Error, Result, WriteOptions};

/// Default per-part size cap. Input files are hostile: a ZIP bomb must not exhaust memory.
/// Reading from a file, a part declared larger is deferred instead (left in the file, see
/// [`Part::deferred`]).
pub const MAX_PART_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Default cap on the decompressed bytes of all parts of one package read into memory: 8 GiB.
/// Many parts that each stay under [`MAX_PART_BYTES`] (or entries that share compressed data)
/// must not add up to more (docs/fuzzing.md, F4). Deferred parts do not count.
pub const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Decompression caps for [`Package::read_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
    /// Largest decompressed part read into memory.
    pub max_part_bytes: u64,
    /// Largest sum of decompressed parts, checked part by part as the package is read.
    pub max_total_bytes: u64,
}

impl ReadLimits {
    /// [`MAX_PART_BYTES`] per part, [`MAX_PACKAGE_BYTES`] in total.
    pub const DEFAULT: ReadLimits = ReadLimits {
        max_part_bytes: MAX_PART_BYTES,
        max_total_bytes: MAX_PACKAGE_BYTES,
    };
}

impl Default for ReadLimits {
    fn default() -> Self {
        ReadLimits::DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// Path inside the package, `/`-separated, without a leading slash (e.g. `Report/Layout`).
    pub name: String,
    /// The decompressed bytes; empty for a deferred part.
    pub data: Vec<u8>,
    /// Whether the part is stored uncompressed.
    pub stored: bool,
    /// Set when the part was too large to read into memory: where its bytes are in the source
    /// archive. Writing the package copies them from there.
    pub deferred: Option<Deferred>,
}

impl Part {
    pub fn new(name: impl Into<String>, data: Vec<u8>, stored: bool) -> Part {
        Part {
            name: name.into(),
            data,
            stored,
            deferred: None,
        }
    }

    /// Decompressed size.
    pub fn len(&self) -> u64 {
        match &self.deferred {
            Some(d) => d.size,
            None => u64::try_from(self.data.len()).unwrap_or(u64::MAX),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_deferred(&self) -> bool {
        self.deferred.is_some()
    }

    /// The decompressed bytes as a stream (from memory, or from the source of a deferred part).
    pub fn reader(&self) -> Result<Box<dyn Read + Send + '_>> {
        match &self.deferred {
            None => Ok(Box::new(self.data.as_slice())),
            Some(d) => {
                let raw = d.raw_reader()?;
                if self.stored {
                    Ok(Box::new(raw))
                } else {
                    Ok(Box::new(flate2::read::DeflateDecoder::new(raw).take(d.size)))
                }
            }
        }
    }

    /// The decompressed bytes: borrowed when the part is in memory, read from the source for a
    /// deferred part of at most [`MAX_PART_BYTES`] (a part left in the file by choice, see
    /// [`Package::open_deferring`]), checked against its CRC-32. A larger deferred part is
    /// [`Error::PartTooLarge`].
    pub fn bytes(&self) -> Result<std::borrow::Cow<'_, [u8]>> {
        let Some(d) = &self.deferred else {
            return Ok(std::borrow::Cow::Borrowed(self.data.as_slice()));
        };
        let too_large = || Error::PartTooLarge {
            part: self.name.clone(),
            limit: MAX_PART_BYTES,
        };
        if d.size > MAX_PART_BYTES {
            return Err(too_large());
        }
        let n = usize::try_from(d.size).map_err(|_| too_large())?;
        let mut data: Vec<u8> = Vec::new();
        data.try_reserve_exact(n).map_err(|_| too_large())?;
        self.reader()?
            .take(d.size.saturating_add(1))
            .read_to_end(&mut data)?;
        if data.len() != n || crc32fast::hash(&data) != d.crc32 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("part `{}`: size or CRC-32 mismatch", self.name),
            )));
        }
        Ok(std::borrow::Cow::Owned(data))
    }

    /// The decompressed bytes with random access: any part in memory, or a stored deferred part
    /// (`None` for a deflated deferred part).
    pub fn seekable(&self) -> Result<Option<Box<dyn ReadSeek + '_>>> {
        match &self.deferred {
            None => Ok(Some(Box::new(Cursor::new(self.data.as_slice())))),
            Some(d) if self.stored => Ok(Some(Box::new(d.raw_reader()?))),
            Some(_) => Ok(None),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    pub parts: Vec<Part>,
}

impl Package {
    /// Reads a package with the default [`ReadLimits`].
    pub fn read<R: Read + Seek>(reader: R) -> Result<Self> {
        Self::read_with(reader, &ReadLimits::default())
    }

    /// Reads a package, refusing any part over `limits.max_part_bytes` and stopping as soon as
    /// the parts read so far exceed `limits.max_total_bytes`.
    pub fn read_with<R: Read + Seek>(reader: R, limits: &ReadLimits) -> Result<Self> {
        read_parts(reader, limits, None, &[])
    }

    /// Reads a package from a source that can be opened again: a part declared larger than
    /// `limits.max_part_bytes` (stored or deflated) is deferred, not read ([`Part::deferred`]).
    pub fn read_from(source: Arc<dyn ArchiveSource>, limits: &ReadLimits) -> Result<Self> {
        let reader = source.open()?;
        read_parts(reader, limits, Some(source), &[])
    }

    /// [`Package::open_with`], also leaving in the file the parts named in `defer` (compared
    /// case-insensitively, stored or deflated) whatever their size: their bytes are read when
    /// needed ([`Part::reader`], [`Part::bytes`]) and copied from the file on write. For a large
    /// `DataModel` under a memory budget (`docs/beyond-ram.md`).
    pub fn open_deferring(path: &std::path::Path, limits: &ReadLimits, defer: &[&str]) -> Result<Self> {
        let source: Arc<dyn ArchiveSource> = Arc::new(FileSource::new(path)?);
        let reader = source.open()?;
        read_parts(reader, limits, Some(source), defer)
    }

    pub fn read_bytes(bytes: &[u8]) -> Result<Self> {
        Self::read(Cursor::new(bytes))
    }

    /// [`Package::read_bytes`] with explicit [`ReadLimits`].
    pub fn read_bytes_with(bytes: &[u8], limits: &ReadLimits) -> Result<Self> {
        Self::read_with(Cursor::new(bytes), limits)
    }

    pub fn open(path: &std::path::Path) -> Result<Self> {
        Self::open_with(path, &ReadLimits::default())
    }

    /// [`Package::open`] with explicit [`ReadLimits`]. Parts over `limits.max_part_bytes` are
    /// deferred ([`Package::read_from`]): they stay in the file and are copied from it on write.
    pub fn open_with(path: &std::path::Path, limits: &ReadLimits) -> Result<Self> {
        Self::read_from(Arc::new(FileSource::new(path)?), limits)
    }

    /// Whether any part was left in the source archive ([`Part::deferred`]).
    pub fn has_deferred(&self) -> bool {
        self.parts.iter().any(Part::is_deferred)
    }

    /// Points the deferred parts at `source`, an archive this package was just written to
    /// (after a save replaced the file they came from). Each deferred part must be in `source`
    /// with the same sizes and CRC.
    pub fn rebind_deferred(&mut self, source: Arc<dyn ArchiveSource>) -> Result<()> {
        if !self.has_deferred() {
            return Ok(());
        }
        let mut zip = ZipArchive::new(source.open()?)?;
        let mut found = std::collections::HashMap::new();
        for i in 0..zip.len() {
            let file = zip.by_index_raw(i)?;
            let name = file.name()?.trim_start_matches('/').to_ascii_lowercase();
            found.insert(
                name,
                (
                    data_start(&file)?,
                    file.compressed_size(),
                    file.size(),
                    file.crc32(),
                ),
            );
        }
        for part in &mut self.parts {
            let Some(d) = part.deferred.as_mut() else {
                continue;
            };
            let at = found.get(&part.name.to_ascii_lowercase());
            match at {
                Some(&(offset, compressed, size, crc))
                    if compressed == d.compressed_size && size == d.size && crc == d.crc32 =>
                {
                    d.source = Arc::clone(&source);
                    d.data_offset = offset;
                }
                _ => return Err(Error::MissingPart(part.name.clone())),
            }
        }
        Ok(())
    }

    /// Writes the package as a ZIP archive laid out like Desktop's (see [`WriteOptions`]): parts
    /// in order, deflated or stored as each part says, Zip64 only where a value needs it.
    pub fn write<W: Write + Seek>(&self, writer: W) -> Result<W> {
        self.write_with(writer, &WriteOptions::default())
    }

    /// [`Package::write`] with explicit [`WriteOptions`].
    pub fn write_with<W: Write>(&self, writer: W, opts: &WriteOptions) -> Result<W> {
        crate::zipwrite::write_parts(&self.parts, writer, opts)
    }

    /// [`Package::to_bytes`] with explicit [`WriteOptions`].
    pub fn to_bytes_with(&self, opts: &WriteOptions) -> Result<Vec<u8>> {
        self.write_with(Vec::new(), opts)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with(&WriteOptions::default())
    }

    pub fn get(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn require(&self, name: &str) -> Result<&Part> {
        self.get(name).ok_or_else(|| Error::MissingPart(name.to_string()))
    }

    /// Adds a part, or replaces the data of an existing part with the same name.
    pub fn put(&mut self, name: &str, data: Vec<u8>, stored: bool) {
        match self.parts.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(p) => {
                p.data = data;
                p.stored = stored;
                p.deferred = None;
            }
            None => self.parts.push(Part::new(name, data, stored)),
        }
    }
}

/// Where the bytes of `file` start in the archive: known once its local header was read, which
/// `by_index` and `by_index_raw` do.
fn data_start<R: Read>(file: &zip::read::ZipFile<'_, R>) -> Result<u64> {
    file.data_start().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a ZIP entry's data offset is unknown",
        ))
    })
}

/// A stored entry of at least [`crate::parread::PARALLEL_MIN`] bytes, within `cap`, read from
/// `source` by [`ArchiveSource::read_at`] and checked against the entry's CRC-32. `None`: not
/// such an entry, or a source that cannot (the caller reads it through the ZIP reader).
fn read_stored_parallel<R: Read>(
    source: &dyn ArchiveSource,
    file: &zip::read::ZipFile<'_, R>,
    stored: bool,
    cap: u64,
) -> Result<Option<Vec<u8>>> {
    let size = file.size();
    if !stored || size != file.compressed_size() || size < crate::parread::PARALLEL_MIN || size > cap {
        return Ok(None);
    }
    let Ok(len) = usize::try_from(size) else {
        return Ok(None);
    };
    let Some(mut data) = crate::parread::zeroed(len) else {
        return Ok(None);
    };
    match source.read_at(data_start(file)?, &mut data) {
        None => Ok(None),
        Some(Err(e)) => Err(Error::Io(e)),
        Some(Ok(crc)) if crc != file.crc32() => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("part `{}`: CRC-32 mismatch", file.name().unwrap_or_default()),
        ))),
        Some(Ok(_)) => Ok(Some(data)),
    }
}

/// Reads every entry; with a `source`, entries over the part cap are deferred instead.
fn read_parts<R: Read + Seek>(
    reader: R,
    limits: &ReadLimits,
    source: Option<Arc<dyn ArchiveSource>>,
    defer: &[&str],
) -> Result<Package> {
    let mut zip = ZipArchive::new(reader)?;
    let mut parts = Vec::with_capacity(zip.len().min(4096));
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.is_dir() {
            continue;
        }
        let name = file.name()?.trim_start_matches('/').to_string();
        let part_too_large = |name: String| Error::PartTooLarge {
            part: name,
            limit: limits.max_part_bytes,
        };
        let package_too_large = || Error::PackageTooLarge {
            limit: limits.max_total_bytes,
        };
        let method = file.compression();
        let stored = method == CompressionMethod::Stored;
        // The declared size is checked first; it may lie, so the read is bounded too.
        let chosen = defer.iter().any(|d| d.eq_ignore_ascii_case(&name));
        if file.size() > limits.max_part_bytes || (chosen && source.is_some()) {
            let deferrable = stored || method == CompressionMethod::Deflated;
            match &source {
                Some(src) if deferrable => {
                    let deferred = Deferred {
                        source: Arc::clone(src),
                        data_offset: data_start(&file)?,
                        compressed_size: file.compressed_size(),
                        size: file.size(),
                        crc32: file.crc32(),
                    };
                    parts.push(Part {
                        name,
                        data: Vec::new(),
                        stored,
                        deferred: Some(deferred),
                    });
                    continue;
                }
                _ => return Err(part_too_large(name)),
            }
        }
        let room = limits.max_total_bytes.saturating_sub(total);
        if file.size() > room {
            return Err(package_too_large());
        }
        let cap = limits.max_part_bytes.min(room);
        // A large stored part (the `DataModel`) of a file: read on several threads, CRC checked.
        if let Some(src) = &source
            && let Some(data) = read_stored_parallel(src.as_ref(), &file, stored, cap)?
        {
            total = total.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
            parts.push(Part::new(name, data, stored));
            continue;
        }
        let mut data = Vec::new();
        // A stored part (the `DataModel`): one allocation of its declared size, already checked
        // against the caps, instead of a doubling series and its copies. Its bytes are in the
        // archive as they are, so the declared size cannot exceed what the entry holds by more
        // than a lying header; a short read only leaves capacity unused.
        if stored && let Ok(declared) = usize::try_from(file.size().min(file.compressed_size()).min(cap)) {
            let _ = data.try_reserve_exact(declared);
        }
        (&mut file).take(cap.saturating_add(1)).read_to_end(&mut data)?;
        let len = u64::try_from(data.len()).unwrap_or(u64::MAX);
        if len > limits.max_part_bytes {
            return Err(part_too_large(name));
        }
        if len > room {
            return Err(package_too_large());
        }
        total = total.saturating_add(len);
        parts.push(Part::new(name, data, stored));
    }
    Ok(Package { parts })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_parts_order_and_storage() {
        let mut pkg = Package::default();
        pkg.put("Version", b"1.28".to_vec(), true);
        pkg.put("Report/Layout", vec![1, 2, 3, 4], false);
        let back = Package::read_bytes(&pkg.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pkg);
    }

    #[test]
    fn lookup_is_case_insensitive_like_opc() {
        let mut pkg = Package::default();
        pkg.put("DataModelSchema", vec![], false);
        assert!(pkg.get("datamodelschema").is_some());
        assert!(pkg.require("Missing").is_err());
    }

    /// F4 (docs/fuzzing.md): the decompressed size of all parts together is capped, part by
    /// part, on top of the per-part cap.
    #[test]
    fn total_decompressed_size_is_capped() {
        const MIB: u64 = 1 << 20;
        let mut pkg = Package::default();
        for name in ["a", "b", "c"] {
            pkg.put(name, vec![0; 1 << 20], false);
        }
        let bytes = pkg.to_bytes().unwrap();
        assert!(bytes.len() < 32 * 1024, "deflated zeros: {} bytes", bytes.len());
        let limits = |part: u64, total: u64| ReadLimits {
            max_part_bytes: part,
            max_total_bytes: total,
        };
        assert_eq!(
            Package::read_bytes_with(&bytes, &limits(MIB, 3 * MIB)).unwrap(),
            pkg
        );
        let err = Package::read_bytes_with(&bytes, &limits(MIB, 3 * MIB - 1)).unwrap_err();
        assert!(
            matches!(err, Error::PackageTooLarge { limit } if limit == 3 * MIB - 1),
            "{err}"
        );
        let err = Package::read_bytes_with(&bytes, &limits(MIB - 1, 3 * MIB)).unwrap_err();
        assert!(
            matches!(&err, Error::PartTooLarge { part, .. } if part == "a"),
            "{err}"
        );
        assert_eq!(ReadLimits::default().max_total_bytes, 8 << 30);
    }

    /// Headers that understate the sizes do not get past the caps: the ZIP reader stops at the
    /// declared size (zip 9 and later), and our own reads are bounded too.
    #[test]
    fn lying_sizes_are_caught_while_reading() {
        const MIB: u64 = 1 << 20;
        let mut pkg = Package::default();
        for name in ["a", "b", "c"] {
            pkg.put(name, vec![0; 1 << 20], false);
        }
        let mut bytes = pkg.to_bytes().unwrap();
        // Central directory entries: uncompressed size at offset 24; local headers: offset 22.
        let mut patched = 0;
        for (sig, at) in [(b"PK\x01\x02", 24usize), (b"PK\x03\x04", 22)] {
            let starts: Vec<usize> = bytes
                .windows(4)
                .enumerate()
                .filter(|(_, w)| w == sig)
                .map(|(i, _)| i)
                .collect();
            for i in starts {
                bytes[i + at..i + at + 4].copy_from_slice(&1u32.to_le_bytes());
                patched += 1;
            }
        }
        assert_eq!(patched, 6);
        let limits = |part: u64, total: u64| ReadLimits {
            max_part_bytes: part,
            max_total_bytes: total,
        };
        let refused = |err: &Error| match err {
            Error::PackageTooLarge { .. } | Error::PartTooLarge { .. } => true,
            Error::Io(e) => e.to_string().contains("larger than its declared"),
            _ => false,
        };
        for l in [
            limits(MIB, 2 * MIB),
            limits(MIB / 2, 8 * MIB),
            ReadLimits::default(),
        ] {
            let err = Package::read_bytes_with(&bytes, &l).unwrap_err();
            assert!(refused(&err), "{err}");
        }
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    /// Offsets of the central directory headers, from the classic end record.
    fn central_headers(b: &[u8], count: usize) -> Vec<usize> {
        let eocd = b.len() - 22;
        assert_eq!(u32_at(b, eocd), 0x0605_4b50);
        let mut at = if u32_at(b, eocd + 16) == u32::MAX {
            let locator = eocd - 20;
            assert_eq!(u32_at(b, locator), 0x0706_4b50);
            let rec = u64_at(b, locator + 8) as usize;
            assert_eq!(u32_at(b, rec), 0x0606_4b50);
            u64_at(b, rec + 48) as usize
        } else {
            u32_at(b, eocd + 16) as usize
        };
        let mut out = Vec::new();
        for _ in 0..count {
            assert_eq!(u32_at(b, at), 0x0201_4b50);
            out.push(at);
            at += 46 + u16_at(b, at + 28) as usize + u16_at(b, at + 30) as usize;
        }
        out
    }

    fn sample() -> Package {
        let mut pkg = Package::default();
        let version = "1.28".encode_utf16().flat_map(u16::to_le_bytes).collect();
        pkg.put("Version", version, false);
        pkg.put("[Content_Types].xml", b"<Types/>".to_vec(), false);
        pkg.put("DataModel", vec![7; 1000], true);
        pkg.put("Report/StaticResources/My%20Logo.png", vec![1, 2, 3], false);
        pkg
    }

    /// The layout System.IO.Packaging gives Desktop's files (pbix-package.md §1, "Zip layout"):
    /// no data descriptors, no extra fields, made by 4.5 on MS-DOS, needed 2.0 / 1.0.
    #[test]
    fn layout_matches_desktop_packages() {
        let pkg = sample();
        let b = pkg.to_bytes().unwrap();
        let central = central_headers(&b, 4);
        for (i, &c) in central.iter().enumerate() {
            assert_eq!(u16_at(&b, c + 4), 45, "made by");
            let stored = pkg.parts[i].stored;
            assert_eq!(u16_at(&b, c + 6), if stored { 10 } else { 20 }, "needed");
            assert_eq!(u16_at(&b, c + 8), 0, "flags");
            assert_eq!(u16_at(&b, c + 10), if stored { 0 } else { 8 });
            assert_eq!(u16_at(&b, c + 30), 0, "no extra field");
            assert_eq!(u32_at(&b, c + 38), 0, "external attributes");
            let local = u32_at(&b, c + 42) as usize;
            assert_eq!(u32_at(&b, local), 0x0403_4b50);
            assert_eq!(
                b[local + 4..local + 26],
                b[c + 6..c + 28],
                "local = central fields"
            );
            assert_eq!(u16_at(&b, local + 28), 0);
        }
        assert!(!b.windows(4).any(|w| w == b"PK\x07\x08"), "no data descriptor");
        assert!(!b.windows(4).any(|w| w == b"PK\x06\x06"), "no Zip64 record");
        // Byte-stable: the same package gives the same bytes.
        assert_eq!(pkg.to_bytes().unwrap(), b);
    }

    /// Forced Zip64 (the shape a > 4 GiB `DataModel` needs) reads back identically, and every
    /// record carries its Zip64 form.
    #[test]
    fn forced_zip64_round_trips() {
        let pkg = sample();
        let b = pkg.to_bytes_with(&WriteOptions { force_zip64: true }).unwrap();
        assert_eq!(Package::read_bytes(&b).unwrap(), pkg);
        let central = central_headers(&b, 4);
        for (i, &c) in central.iter().enumerate() {
            assert_eq!(u16_at(&b, c + 6), 45, "needed 4.5");
            assert_eq!(u32_at(&b, c + 20), u32::MAX);
            assert_eq!(u32_at(&b, c + 24), u32::MAX);
            assert_eq!(u32_at(&b, c + 42), u32::MAX);
            let name_len = u16_at(&b, c + 28) as usize;
            let extra = c + 46 + name_len;
            assert_eq!(u16_at(&b, extra), 1, "Zip64 tag");
            assert_eq!(u16_at(&b, extra + 2), 24);
            let size = pkg.parts[i].data.len() as u64;
            assert_eq!(u64_at(&b, extra + 4), size);
            let local = u64_at(&b, extra + 20) as usize;
            assert_eq!(u32_at(&b, local), 0x0403_4b50);
            assert_eq!(u16_at(&b, local + 4), 45);
            // The local Zip64 field holds both sizes.
            assert_eq!(u16_at(&b, local + 28), 20);
            let lextra = local + 30 + u16_at(&b, local + 26) as usize;
            assert_eq!(u16_at(&b, lextra), 1);
            assert_eq!(u16_at(&b, lextra + 2), 16);
            assert_eq!(u64_at(&b, lextra + 4), size);
            assert_eq!(u64_at(&b, lextra + 12), u64_at(&b, extra + 12));
        }
        let eocd = b.len() - 22;
        assert_eq!(u16_at(&b, eocd + 8), u16::MAX);
        assert_eq!(u32_at(&b, eocd + 12), u32::MAX);
    }

    /// More than 65 535 entries need the Zip64 end record; the classic one says 0xFFFF.
    #[test]
    fn more_than_65535_parts_use_the_zip64_end_record() {
        let mut pkg = Package::default();
        for i in 0..70_000u32 {
            pkg.parts.push(Part::new(format!("p/{i}"), vec![], true));
        }
        let b = pkg.to_bytes().unwrap();
        let eocd = b.len() - 22;
        assert_eq!(u16_at(&b, eocd + 10), u16::MAX);
        let rec = u64_at(&b, eocd - 20 + 8) as usize;
        assert_eq!(u64_at(&b, rec + 32), 70_000);
        let back = Package::read_bytes(&b).unwrap();
        assert_eq!(back.parts.len(), 70_000);
        assert_eq!(back.parts[69_999].name, "p/69999");
    }

    /// A hand-made archive: one stored entry written with a data descriptor (flag bit 3, sizes
    /// zero in the local header), as streaming writers do. Desktop never writes one; reading it
    /// must still work (the central directory is authoritative).
    #[test]
    fn data_descriptors_are_read() {
        let data = b"hello";
        let crc = crc32fast::hash(data);
        let mut b = Vec::new();
        b.extend_from_slice(b"PK\x03\x04");
        b.extend_from_slice(&[20, 0, 8, 0, 0, 0, 0, 0, 0x21, 0]);
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.push(b'a');
        b.extend_from_slice(data);
        b.extend_from_slice(b"PK\x07\x08");
        b.extend_from_slice(&crc.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        let cd = b.len() as u32;
        b.extend_from_slice(b"PK\x01\x02");
        b.extend_from_slice(&[45, 0, 20, 0, 8, 0, 0, 0, 0, 0, 0x21, 0]);
        b.extend_from_slice(&crc.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&0u32.to_le_bytes());
        b.push(b'a');
        let cd_size = b.len() as u32 - cd;
        b.extend_from_slice(b"PK\x05\x06");
        b.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        b.extend_from_slice(&cd_size.to_le_bytes());
        b.extend_from_slice(&cd.to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        let pkg = Package::read_bytes(&b).unwrap();
        assert_eq!(pkg.parts.len(), 1);
        assert_eq!(pkg.parts[0].data, data);
        assert!(pkg.parts[0].stored);
    }

    /// Zip64 records that lie (entry counts, offsets and sizes far beyond the file) are errors,
    /// not allocations or panics.
    #[test]
    fn hostile_zip64_records_are_errors() {
        let good = sample()
            .to_bytes_with(&WriteOptions { force_zip64: true })
            .unwrap();
        let eocd = good.len() - 22;
        let rec = u64_at(&good, eocd - 20 + 8) as usize;
        let patches: [(usize, u64); 5] = [
            (rec + 24, u64::MAX),      // entries on this disk
            (rec + 32, u64::MAX),      // entries
            (rec + 40, u64::MAX / 2),  // central directory size
            (rec + 48, u64::MAX / 2),  // central directory offset
            (eocd - 20 + 8, u64::MAX), // locator: record offset
        ];
        for (at, v) in patches {
            let mut b = good.clone();
            b[at..at + 8].copy_from_slice(&v.to_le_bytes());
            let _ = Package::read_bytes(&b); // must return, Ok or Err
        }
        // A central Zip64 size field that lies is caught by the size caps while reading.
        let c = central_headers(&good, 4)[2];
        let extra = c + 46 + u16_at(&good, c + 28) as usize;
        let mut b = good.clone();
        b[extra + 4..extra + 12].copy_from_slice(&(1u64 << 40).to_le_bytes());
        let limits = ReadLimits {
            max_part_bytes: 1 << 20,
            max_total_bytes: 1 << 20,
        };
        assert!(Package::read_bytes_with(&b, &limits).is_err());
    }

    /// A part left in the file by choice reads back the same bytes, and the package writes the
    /// same archive.
    #[test]
    fn parts_deferred_by_choice_read_and_write_the_same() {
        let dir = std::env::temp_dir().join(format!("bicraft-opc-defer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.pbix");
        let mut pkg = sample();
        pkg.put("Report/Layout", vec![9; 5000], false);
        std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
        let open =
            Package::open_deferring(&path, &ReadLimits::default(), &["datamodel", "Report/Layout"]).unwrap();
        for name in ["DataModel", "Report/Layout"] {
            let p = open.get(name).unwrap();
            assert!(p.is_deferred(), "{name}");
            assert!(p.data.is_empty());
            assert_eq!(
                p.bytes().unwrap().as_ref(),
                pkg.get(name).unwrap().data.as_slice()
            );
        }
        assert!(!open.get("Version").unwrap().is_deferred());
        assert!(matches!(
            open.get("Version").unwrap().bytes().unwrap(),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(open.to_bytes().unwrap(), pkg.to_bytes().unwrap());
        // the file changed under the package: an error, not other bytes
        std::fs::write(&path, b"changed").unwrap();
        assert!(open.get("DataModel").unwrap().bytes().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(Package::read_bytes(b"not a zip").is_err());
        assert!(Package::read_bytes(&[]).is_err());
    }
}
