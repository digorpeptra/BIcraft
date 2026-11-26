//! L0 packaging primitives: reading and writing the ZIP container behind PBIX/PBIT files, and
//! decoding the mixed UTF-8 / UTF-16LE text parts they hold.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod deferred;
mod extract;
mod package;
mod parread;
mod text;
mod zipwrite;

pub use deferred::{ArchiveSource, Deferred, FileSource, ReadSeek, Window};
pub use extract::{
    Entry as ZipEntry, ExtractError, Extracted, ZipLimits, for_each_entry as for_each_zip_entry,
};
pub use package::{MAX_PACKAGE_BYTES, MAX_PART_BYTES, Package, Part, ReadLimits};
pub use parread::fill as read_exact_at;
pub use text::{
    Encoding, container_text_to_package, container_text_to_project, decode_text, encode_text,
    is_container_text,
};
pub use zipwrite::WriteOptions;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a valid package: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("part `{part}` is larger than the {limit}-byte limit")]
    PartTooLarge { part: String, limit: u64 },
    #[error("the package's parts decompress to more than the {limit}-byte limit")]
    PackageTooLarge { limit: u64 },
    #[error("part `{0}` not found")]
    MissingPart(String),
    #[error("part `{0}` is not valid {1} text")]
    BadText(String, &'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
