//! L0: the Plain LZ77 compression of [MS-XCA] §2.3/§2.4 (the "Xpress" or "xpress8" format) and
//! the chunk framing of the files an Excel Power Pivot backup stores compressed
//! (`ApplyCompression`, `docs/formats/powerpivot.md`).
//!
//! The decoder ([`decompress_block`], [`decompress_chunked`]) is ported from DashCraft (MIT OR
//! Apache-2.0, `crates/pbi-vertipaq/src/xpress8.rs` at commit
//! `0958fd82b04656ee1797101a2b4df8d114dfebd1`, see `THIRD_PARTY_NOTICES.md`) and checked against
//! the [MS-XCA] pseudocode. The encoder ([`compress_block`], [`compress_chunked`]) is BIcraft's
//! own; it builds synthetic test backups (BIcraft never writes Power Pivot workbooks).
//!
//! ```
//! let data = b"to be or not to be, to be or not to be".repeat(20);
//! let chunks = bicraft_xpress8::compress_chunked(&data, bicraft_xpress8::DEFAULT_CHUNK_SIZE);
//! let back = bicraft_xpress8::decompress_chunked(&chunks, Some(data.len() as u64), 1 << 20)?;
//! assert_eq!(back, data);
//! # Ok::<(), bicraft_xpress8::Error>(())
//! ```
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod decode;
mod encode;

pub use decode::{CHUNK_HEADER_LEN, Error, MAX_CHUNK_SIZE, decompress_block, decompress_chunked, fuzz};
pub use encode::{DEFAULT_CHUNK_SIZE, compress_block, compress_chunked};
