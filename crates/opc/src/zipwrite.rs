//! The ZIP writer behind [`crate::Package::write`], laid out the way Power BI Desktop's packages
//! are (System.IO.Packaging on .NET Framework, `docs/formats/pbix-package.md` §1, "Zip layout"):
//!
//! - local headers without data descriptors (sizes and CRC known up front), flags 0 (bit 11 only
//!   for a non-ASCII name), no extra field unless the entry needs Zip64;
//! - "version made by" 4.5 on MS-DOS, "version needed" 1.0 for stored, 2.0 for deflated and 4.5
//!   for a Zip64 entry; external attributes 0; a fixed 1980-01-01 00:00 timestamp, so the output
//!   is byte-stable;
//! - Zip64 (APPNOTE 4.5.3) only when a value does not fit, or when forced
//!   ([`WriteOptions::force_zip64`]): the local Zip64 extra field carries **both** sizes, the
//!   central one the fields whose 32-bit slot holds `0xFFFFFFFF`, in APPNOTE order; the Zip64 end
//!   of central directory record and its locator precede the classic record, whose overflowing
//!   fields hold `0xFFFF` / `0xFFFFFFFF`.

use std::io::{Read, Write};

use flate2::{Compression, write::DeflateEncoder};

use crate::{Deferred, Error, Part, Result};

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_TAG: u16 = 0x0001;

const VERSION_STORED: u16 = 10;
const VERSION_DEFLATED: u16 = 20;
const VERSION_ZIP64: u16 = 45;
/// 4.5, host 0 (MS-DOS), as System.IO.Packaging writes on every entry.
const VERSION_MADE_BY: u16 = VERSION_ZIP64;
const FLAG_UTF8: u16 = 1 << 11;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATED: u16 = 8;
/// DOS date 1980-01-01, time 00:00:00.
const DOS_DATE: u16 = (1 << 5) | 1;
const DOS_TIME: u16 = 0;

const U32_MAX: u64 = 0xFFFF_FFFF;
/// Piece size for copying a deferred part.
const COPY_CHUNK: usize = 1 << 20;
const U16_MAX: u64 = 0xFFFF;

/// How [`crate::Package::write_with`] lays out the archive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// Writes every entry, the central directory and the end records in their Zip64 form even
    /// when the values fit in 32 bits (for tests and reader checks; Desktop writes Zip64 only
    /// when needed).
    pub force_zip64: bool,
}

/// One entry as written, kept for the central directory.
struct Entry {
    name: Vec<u8>,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
    uncompressed: u64,
    offset: u64,
    /// Sizes carried in Zip64 extra fields.
    zip64: bool,
    /// Offset carried in the central Zip64 extra field.
    zip64_offset: bool,
}

impl Entry {
    /// The same in the local and the central header.
    fn version_needed(&self) -> u16 {
        if self.zip64 || self.zip64_offset {
            VERSION_ZIP64
        } else if self.method == METHOD_DEFLATED {
            VERSION_DEFLATED
        } else {
            VERSION_STORED
        }
    }
}

/// Counts the bytes written so offsets are known without seeking.
struct Counted<W> {
    inner: W,
    at: u64,
}

impl<W: Write> Counted<W> {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.write_all(bytes)?;
        let len = u64::try_from(bytes.len()).map_err(|_| too_large("a write"))?;
        self.at = self.at.checked_add(len).ok_or_else(|| too_large("the archive"))?;
        Ok(())
    }
}

fn too_large(what: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("{what} is too large for a ZIP archive"),
    ))
}

fn u16_of(n: usize, what: &str) -> Result<u16> {
    u16::try_from(n).map_err(|_| too_large(what))
}

fn u64_of(n: usize) -> Result<u64> {
    u64::try_from(n).map_err(|_| too_large("a part"))
}

/// The value for a 32-bit slot: the value itself, or `0xFFFFFFFF` when Zip64 carries it.
fn slot32(v: u64, in_zip64: bool) -> u32 {
    if in_zip64 {
        u32::MAX
    } else {
        u32::try_from(v).unwrap_or(u32::MAX)
    }
}

fn deflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::with_capacity(data.len() / 2 + 64), Compression::default());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

/// The data of one entry: bytes in memory, or the compressed bytes of a deferred part.
enum Body<'a> {
    Bytes(&'a [u8]),
    /// The part and whether it is stored.
    Deferred(&'a Deferred, bool),
}

/// Copies a deferred part's compressed bytes from its source, in pieces of [`COPY_CHUNK`]
/// bytes. The byte count must match; a stored part's CRC is checked too (a source that changed
/// is an error, not a corrupt archive the caller would keep).
fn copy_deferred<W: Write>(out: &mut Counted<W>, d: &Deferred, stored: bool, name: &str) -> Result<()> {
    let changed = |what: &str| {
        Error::Io(std::io::Error::other(format!(
            "part `{name}`: {what}: the source package changed since it was read"
        )))
    };
    let mut src = d.raw_reader()?;
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut left = d.compressed_size;
    let mut crc = crc32fast::Hasher::new();
    while left > 0 {
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(COPY_CHUNK);
        let piece = buf.get_mut(..want).ok_or_else(|| too_large("a part"))?;
        let n = src.read(piece)?;
        if n == 0 {
            return Err(changed("fewer bytes than recorded"));
        }
        let got = piece.get(..n).ok_or_else(|| too_large("a part"))?;
        if stored {
            crc.update(got);
        }
        out.put(got)?;
        left = left.saturating_sub(u64_of(n)?);
    }
    if stored && crc.finalize() != d.crc32 {
        return Err(changed("CRC mismatch"));
    }
    Ok(())
}

/// Writes `parts` as a ZIP archive into `writer`, in order.
pub(crate) fn write_parts<W: Write>(parts: &[Part], writer: W, opts: &WriteOptions) -> Result<W> {
    let mut out = Counted { inner: writer, at: 0 };
    let mut entries: Vec<Entry> = Vec::with_capacity(parts.len().min(1 << 16));
    for part in parts {
        let name = part.name.as_bytes().to_vec();
        let name_len = u16_of(name.len(), "a part name")?;
        let flags = if part.name.is_ascii() { 0 } else { FLAG_UTF8 };
        let deflated;
        let (method, crc, uncompressed, compressed, body) = match &part.deferred {
            Some(d) => (
                if part.stored {
                    METHOD_STORED
                } else {
                    METHOD_DEFLATED
                },
                d.crc32,
                d.size,
                d.compressed_size,
                Body::Deferred(d, part.stored),
            ),
            None => {
                let crc = crc32fast::hash(&part.data);
                let (method, body): (u16, &[u8]) = if part.stored {
                    (METHOD_STORED, &part.data)
                } else {
                    deflated = deflate(&part.data)?;
                    (METHOD_DEFLATED, &deflated)
                };
                (
                    method,
                    crc,
                    u64_of(part.data.len())?,
                    u64_of(body.len())?,
                    Body::Bytes(body),
                )
            }
        };
        let offset = out.at;
        let zip64 = opts.force_zip64 || uncompressed >= U32_MAX || compressed >= U32_MAX;
        let zip64_offset = opts.force_zip64 || offset >= U32_MAX;
        let entry = Entry {
            name,
            flags,
            method,
            crc,
            compressed,
            uncompressed,
            offset,
            zip64,
            zip64_offset,
        };

        // Local header. A Zip64 local extra field holds both sizes (APPNOTE 4.5.3).
        let mut extra = Vec::new();
        if zip64 {
            extra.extend_from_slice(&ZIP64_TAG.to_le_bytes());
            extra.extend_from_slice(&16u16.to_le_bytes());
            extra.extend_from_slice(&uncompressed.to_le_bytes());
            extra.extend_from_slice(&compressed.to_le_bytes());
        }
        let mut h = Vec::with_capacity(30 + entry.name.len() + extra.len());
        h.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        h.extend_from_slice(&entry.version_needed().to_le_bytes());
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&method.to_le_bytes());
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&slot32(compressed, zip64).to_le_bytes());
        h.extend_from_slice(&slot32(uncompressed, zip64).to_le_bytes());
        h.extend_from_slice(&name_len.to_le_bytes());
        h.extend_from_slice(&u16_of(extra.len(), "an extra field")?.to_le_bytes());
        h.extend_from_slice(&entry.name);
        h.extend_from_slice(&extra);
        out.put(&h)?;
        match body {
            Body::Bytes(b) => out.put(b)?,
            Body::Deferred(d, stored) => copy_deferred(&mut out, d, stored, &part.name)?,
        }
        entries.push(entry);
    }

    let cd_start = out.at;
    for e in &entries {
        // Central Zip64 fields: those whose 32-bit slot holds 0xFFFFFFFF, in APPNOTE order
        // (both sizes of a Zip64 entry, as in its local header; the offset when it overflows).
        let mut extra = Vec::new();
        if e.zip64 || e.zip64_offset {
            let mut body = Vec::with_capacity(24);
            if e.zip64 {
                body.extend_from_slice(&e.uncompressed.to_le_bytes());
                body.extend_from_slice(&e.compressed.to_le_bytes());
            }
            if e.zip64_offset {
                body.extend_from_slice(&e.offset.to_le_bytes());
            }
            extra.extend_from_slice(&ZIP64_TAG.to_le_bytes());
            extra.extend_from_slice(&u16_of(body.len(), "an extra field")?.to_le_bytes());
            extra.extend_from_slice(&body);
        }
        let mut h = Vec::with_capacity(46 + e.name.len() + extra.len());
        h.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        h.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
        h.extend_from_slice(&e.version_needed().to_le_bytes());
        h.extend_from_slice(&e.flags.to_le_bytes());
        h.extend_from_slice(&e.method.to_le_bytes());
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&e.crc.to_le_bytes());
        h.extend_from_slice(&slot32(e.compressed, e.zip64).to_le_bytes());
        h.extend_from_slice(&slot32(e.uncompressed, e.zip64).to_le_bytes());
        h.extend_from_slice(&u16_of(e.name.len(), "a part name")?.to_le_bytes());
        h.extend_from_slice(&u16_of(extra.len(), "an extra field")?.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // comment length
        h.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        h.extend_from_slice(&slot32(e.offset, e.zip64_offset).to_le_bytes());
        h.extend_from_slice(&e.name);
        h.extend_from_slice(&extra);
        out.put(&h)?;
    }
    let cd_end = out.at;
    let cd_size = cd_end
        .checked_sub(cd_start)
        .ok_or_else(|| too_large("the central directory"))?;
    let count = u64_of(entries.len())?;
    let zip64_end = opts.force_zip64 || count >= U16_MAX || cd_size >= U32_MAX || cd_start >= U32_MAX;
    if zip64_end {
        let mut r = Vec::with_capacity(56 + 20);
        r.extend_from_slice(&EOCD64_SIG.to_le_bytes());
        r.extend_from_slice(&44u64.to_le_bytes()); // size of the remaining record
        r.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
        r.extend_from_slice(&VERSION_ZIP64.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes()); // this disk
        r.extend_from_slice(&0u32.to_le_bytes()); // disk with the central directory
        r.extend_from_slice(&count.to_le_bytes());
        r.extend_from_slice(&count.to_le_bytes());
        r.extend_from_slice(&cd_size.to_le_bytes());
        r.extend_from_slice(&cd_start.to_le_bytes());
        // Locator.
        r.extend_from_slice(&LOCATOR_SIG.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.extend_from_slice(&cd_end.to_le_bytes());
        r.extend_from_slice(&1u32.to_le_bytes());
        out.put(&r)?;
    }
    let count16 = if zip64_end && (opts.force_zip64 || count >= U16_MAX) {
        u16::MAX
    } else {
        u16::try_from(count).unwrap_or(u16::MAX)
    };
    let mut r = Vec::with_capacity(22);
    r.extend_from_slice(&EOCD_SIG.to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes());
    r.extend_from_slice(&count16.to_le_bytes());
    r.extend_from_slice(&count16.to_le_bytes());
    r.extend_from_slice(&slot32(cd_size, opts.force_zip64 || cd_size >= U32_MAX).to_le_bytes());
    r.extend_from_slice(&slot32(cd_start, opts.force_zip64 || cd_start >= U32_MAX).to_le_bytes());
    r.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out.put(&r)?;
    Ok(out.inner)
}
