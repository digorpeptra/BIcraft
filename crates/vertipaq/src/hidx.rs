//! `.hidx`: hash index of a value-encoded column's attribute hierarchy ([MS-XLDM] §2.3.3).
//!
//! Parsed for validation and inspection; decoding column values does not need it.

use crate::cursor::Cursor;
use crate::{Error, FileKind};

/// A parsed hash index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashIndex {
    pub algorithm: i32,
    pub entry_size: u32,
    pub bin_size: u32,
    pub local_entry_count: u32,
    pub bin_count: u64,
    pub record_count: i64,
    pub current_mask: i64,
    pub stats: Option<HashStats>,
    pub bins: Vec<HashBin>,
    pub overflow: Vec<HashEntry>,
}

/// Optional hash statistics block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashStats {
    pub element_count: u64,
    pub bin_count: u64,
    pub used_bins: u64,
    pub fast_access_elements: u64,
    pub locals_per_bin: u64,
    pub max_chain: u64,
    pub histogram_element_size: u32,
    pub histogram: Vec<u64>,
}

/// One hash bin: a chain link, the number of entries that hash to it (local ones first; when
/// it exceeds the local entry count, the rest are in [`HashIndex::overflow`]) and the local
/// entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashBin {
    pub chain: u64,
    pub count: u32,
    pub entries: Vec<HashEntry>,
}

/// A (hash, key) pair; the key is a data id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashEntry {
    pub hash: u32,
    pub key: u32,
}

impl HashIndex {
    /// Entries in use: the first `count` local entries of each bin (at most all of them), then
    /// the overflow list, which holds every bin's entries beyond its local ones, bin by bin.
    pub fn used_entries(&self) -> impl Iterator<Item = &HashEntry> + '_ {
        self.bins
            .iter()
            .flat_map(|b| b.entries.iter().take(b.count as usize))
            .chain(self.overflow.iter())
    }
}

/// Parses and validates a `.hidx` file.
pub fn parse_hash_index(bytes: &[u8]) -> Result<HashIndex, Error> {
    let mut c = Cursor::new(bytes, FileKind::Hidx);
    let algorithm = c.i32("hash algorithm")?;
    let entry_size = c.u32("hash entry size")?;
    let bin_size = c.u32("hash bin size")?;
    let local_entry_count = c.u32("local entry count")?;
    let bin_count = c.u64("bin count")?;
    let record_count = c.i64("record count")?;
    let current_mask = c.i64("current mask")?;
    if entry_size != 8 {
        return Err(Error::Unsupported {
            kind: FileKind::Hidx,
            message: format!("hash entry size {entry_size}"),
        });
    }
    let expected_bin = u64::from(local_entry_count)
        .checked_mul(8)
        .and_then(|n| n.checked_add(16));
    if expected_bin != Some(u64::from(bin_size)) {
        return Err(c.malformed(format!(
            "bin size {bin_size} does not fit {local_entry_count} local entries"
        )));
    }
    let has_stats = c.u8("hash statistics flag")?;
    let stats = if has_stats != 0 {
        let element_count = c.u64("element count")?;
        let stat_bins = c.u64("stat bin count")?;
        let used_bins = c.u64("used bins")?;
        let fast_access_elements = c.u64("fast access elements")?;
        let locals_per_bin = c.u64("locals per bin")?;
        let max_chain = c.u64("max chain")?;
        let histogram_len = c.u64("histogram length")?;
        let histogram_element_size = c.u32("histogram element size")?;
        let raw = c.take_array(
            histogram_len,
            usize::try_from(histogram_element_size).unwrap_or(usize::MAX),
            "histogram",
        )?;
        let histogram = match histogram_element_size {
            4 => raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u64::from(u32::from_le_bytes(*b)))
                .collect(),
            8 => crate::cursor::words(raw).collect(),
            _ if histogram_len == 0 => Vec::new(),
            other => return Err(c.malformed(format!("histogram element size {other}"))),
        };
        Some(HashStats {
            element_count,
            bin_count: stat_bins,
            used_bins,
            fast_access_elements,
            locals_per_bin,
            max_chain,
            histogram_element_size,
            histogram,
        })
    } else {
        None
    };
    let raw_bins = c.take_array(bin_count, bin_size as usize, "hash bins")?;
    let mut bins = Vec::new();
    // Entries of a full bin beyond its local ones sit in the overflow list.
    let mut spilled: u64 = 0;
    for raw in raw_bins.chunks_exact(bin_size as usize) {
        let mut b = Cursor::new(raw, FileKind::Hidx);
        let chain = b.u64("bin chain")?;
        let count = b.u32("bin count")?;
        spilled = spilled.saturating_add(u64::from(count.saturating_sub(local_entry_count)));
        let mut entries = Vec::with_capacity(local_entry_count as usize);
        for _ in 0..local_entry_count {
            entries.push(HashEntry {
                hash: b.u32("entry hash")?,
                key: b.u32("entry key")?,
            });
        }
        bins.push(HashBin {
            chain,
            count,
            entries,
        });
    }
    let overflow_count = c.u64("overflow count")?;
    if overflow_count != spilled {
        return Err(c.malformed(format!(
            "{overflow_count} overflow entries for {spilled} entries beyond the bins' local ones"
        )));
    }
    let raw = c.take_array(overflow_count, 8, "overflow entries")?;
    let overflow = raw
        .as_chunks::<8>()
        .0
        .iter()
        .map(|e| HashEntry {
            hash: u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
            key: u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
        })
        .collect();
    if !bytes.get(c.pos()..).unwrap_or(&[]).iter().all(|&b| b == 0) {
        return Err(c.malformed("trailing bytes after the hash index"));
    }
    let index = HashIndex {
        algorithm,
        entry_size,
        bin_size,
        local_entry_count,
        bin_count,
        record_count,
        current_mask,
        stats,
        bins,
        overflow,
    };
    let used = index.used_entries().count() as u64;
    if i64::try_from(used).ok() != Some(record_count) {
        return Err(Error::Malformed {
            kind: FileKind::Hidx,
            offset: 0,
            message: format!("{used} used entries for {record_count} records"),
        });
    }
    Ok(index)
}
