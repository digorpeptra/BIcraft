//! L0: reader and writer for the Analysis Services backup (ABF) container: the stream inside
//! a PBIX `DataModel` part (or `.pbi/cache.abf`) once XPress9 is decompressed.
//!
//! [`Abf::parse`] reads a backup; [`Abf::to_writer`] re-packs it with files replaced, removed
//! or added ([`AbfWriter`]). Writing an unchanged Power BI backup reproduces it byte for byte.
//!
//! Layout ([MS-XLDM] §2.1, see `docs/formats/abf.md`): a 4096-byte header page (BOM, stream
//! storage signature, `BackupLog` header XML), a files section with every file stored back to
//! back, and a page-aligned `VirtualDirectory` XML listing each file's storage name, offset and
//! size. The last file is the `BackupLog`, which maps storage names to the logical paths
//! (`metadata.sqlitedb`, VertiPaq `.idf`/`.idfmeta`/`.dictionary`/`.hidx` files, ...).
//!
//! ```no_run
//! # fn demo(decompressed: &[u8]) -> Result<(), bicraft_abf::Error> {
//! let abf = bicraft_abf::Abf::parse(decompressed)?;
//! for f in abf.files() {
//!     println!("{} {} {}", f.kind, f.size, f.path);
//! }
//! let sqlite: Option<&[u8]> = abf.metadata_sqlitedb();
//! # let _ = sqlite; Ok(()) }
//! ```
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod abf;
mod fresh;
mod header;
mod text;
mod writer;
mod xml;

#[cfg(test)]
mod synth;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_writer;

pub use abf::{
    Abf, BackupLog, FileEntry, FileGroup, FileKind, LogFile, StreamedIndex, Summary, SystemTable,
    streamed_header, streamed_index, streamed_log_range,
};
pub use fresh::{NewBackup, database_key, new_backup};
pub use header::Header;
pub use writer::AbfWriter;

/// ABF page size ([MS-XLDM] §2.1). The header occupies exactly one page.
pub const PAGE_SIZE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("input is still XPress9-compressed: decompress the DataModel before parsing the ABF")]
    Xpress9Compressed,
    #[error("not an ABF backup: {0}")]
    NotAbf(String),
    #[error("malformed {part}: {reason}")]
    Malformed { part: &'static str, reason: String },
    #[error("{what} at offset {offset} (+{len} bytes) lies outside the {stream_len}-byte stream")]
    OutOfBounds {
        what: &'static str,
        offset: u64,
        len: u64,
        stream_len: usize,
    },
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("no file in the backup matches the requested path")]
    FileNotFound,
    #[error("the requested path matches {0} files")]
    AmbiguousPath(usize),
    #[error("a file with this path is already in the backup")]
    PathExists,
    #[error("cannot write the backup: {0}")]
    Write(String),
}

pub type Result<T> = std::result::Result<T, Error>;
