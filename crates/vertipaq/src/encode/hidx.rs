//! `.hidx` writer ([MS-XLDM] §2.3.3), the inverse of [`crate::parse_hash_index`].
//!
//! Desktop's files (value-encoded columns) hold one entry per distinct data id, the blank id 2
//! included when the column has blanks: hash = the data id, key = its rank in ascending id
//! order (so the blank, when present, has key 0), bin = the [MS-XLDM] §2.3.3.1.3
//! multiplicative hash of the id (the top `log2(bins)` bits of `id × 0x12B9B6A5` mod 2^32),
//! entries in key order within a bin, bins of 6 local entries, the statistics flag set with
//! all statistics zero and an empty 8-byte histogram.
//!
//! Bin count (observed on engine-written files of 3 to 1 000 000 entries): the smallest power
//! of two, at least 16, with `entries ≤ 4.5 × bins` (6 locals at a 0.75 load factor). A bin's
//! `count` is the number of entries that hash to it; when it exceeds 6, the extra entries go
//! to the overflow list, bin by bin in bin order (key order within a bin).

use crate::Error;

const LOCAL_ENTRIES: usize = 6;
const MIN_BINS: usize = 16;
const BIN_SIZE: u32 = 16 + 8 * LOCAL_ENTRIES as u32;
/// Multiplier of the [MS-XLDM] §2.3.3.1.3 hash.
const MAGIC: u32 = 0x12B9_B6A5;

/// Bin of `key` in a table of `bins` bins (a power of two, at least 16).
fn bin_of(key: u32, bins: usize) -> usize {
    let bits = bins.trailing_zeros().min(32);
    key.wrapping_mul(MAGIC).checked_shr(32 - bits).unwrap_or(0) as usize
}

/// Bins of a hash index of `records` entries: grow (from 16) while the load is above 4.5.
fn bins_for(records: usize) -> Result<usize, Error> {
    let mut bins = MIN_BINS;
    while records.saturating_mul(2) > bins.saturating_mul(9) {
        if bins >= 1 << 31 {
            return Err(Error::LimitExceeded("hash index too large".into()));
        }
        bins *= 2;
    }
    Ok(bins)
}

/// Builds the hash index of the distinct ids of `ids` (blank included).
pub(super) fn write(ids: &[u32]) -> Result<Vec<u8>, Error> {
    let too_large = || Error::LimitExceeded("hash index too large".into());
    let mut keys: Vec<u32> = ids.to_vec();
    keys.sort_unstable();
    keys.dedup();
    let records = keys.len();
    let bins = bins_for(records)?;
    let mut table: Vec<Vec<(u32, u32)>> = vec![Vec::new(); bins];
    for (rank, &id) in keys.iter().enumerate() {
        let entry = (id, u32::try_from(rank).map_err(|_| too_large())?);
        if let Some(bin) = table.get_mut(bin_of(id, bins)) {
            bin.push(entry);
        }
    }
    let overflow: Vec<(u32, u32)> = table
        .iter()
        .flat_map(|bin| bin.iter().skip(LOCAL_ENTRIES).copied())
        .collect();
    let mut out = Vec::with_capacity(109 + bins * BIN_SIZE as usize + overflow.len() * 8);
    out.extend_from_slice(&(-1i32).to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&BIN_SIZE.to_le_bytes());
    out.extend_from_slice(&(LOCAL_ENTRIES as u32).to_le_bytes());
    out.extend_from_slice(&(bins as u64).to_le_bytes());
    out.extend_from_slice(&(records as i64).to_le_bytes());
    out.extend_from_slice(&((bins - 1) as i64).to_le_bytes());
    out.push(1); // statistics present, all zero
    out.extend_from_slice(&[0u8; 7 * 8]);
    out.extend_from_slice(&8u32.to_le_bytes());
    for bin in &table {
        out.extend_from_slice(&0u64.to_le_bytes()); // chain
        out.extend_from_slice(&u32::try_from(bin.len()).map_err(|_| too_large())?.to_le_bytes());
        for k in 0..LOCAL_ENTRIES {
            let (hash, key) = bin.get(k).copied().unwrap_or((0, 0));
            out.extend_from_slice(&hash.to_le_bytes());
            out.extend_from_slice(&key.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes()); // padding
    }
    out.extend_from_slice(&(overflow.len() as u64).to_le_bytes());
    for (hash, key) in overflow {
        out.extend_from_slice(&hash.to_le_bytes());
        out.extend_from_slice(&key.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_follow_the_spec_hash() {
        assert_eq!(bin_of(3, 16), 3);
        assert_eq!(bin_of(6, 16), 7);
        assert_eq!(bin_of(14, 16), 0);
        assert!(bin_of(u32::MAX, 1 << 20) < 1 << 20);
    }

    #[test]
    fn bins_grow_at_a_load_of_four_and_a_half() {
        // (entries, bins) pairs of engine-written files
        for (records, bins) in [
            (3, 16),
            (52, 16),
            (13_335, 4096),
            (17_144, 4096),
            (18_183, 4096),
            (18_668, 8192),
            (20_000, 8192),
            (99_991, 32_768),
            (500_001, 131_072),
            (1_000_000, 262_144),
        ] {
            assert_eq!(bins_for(records).unwrap(), bins, "{records} entries");
        }
    }

    #[test]
    fn full_bins_spill_to_the_overflow_list_in_bin_order() {
        // 18 183 ids at a load of 4.44: many bins hold more than 6 entries
        let ids: Vec<u32> = (3..18_186).collect();
        let bytes = write(&ids).unwrap();
        let h = crate::parse_hash_index(&bytes).unwrap();
        assert_eq!(h.bin_count, 4096);
        assert!(!h.overflow.is_empty());
        let spilled: u64 = h.bins.iter().map(|b| u64::from(b.count.saturating_sub(6))).sum();
        assert_eq!(spilled, h.overflow.len() as u64);
        let order: Vec<(usize, u32)> = h.overflow.iter().map(|e| (bin_of(e.hash, 4096), e.key)).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
        assert_eq!(h.used_entries().count(), 18_183);
    }

    #[test]
    fn parses_back() {
        let ids: Vec<u32> = (3..2000).chain([2, 2, 7]).collect();
        let bytes = write(&ids).unwrap();
        let h = crate::parse_hash_index(&bytes).unwrap();
        assert_eq!(h.record_count, 1998, "1997 values and the blank");
        let mut keys: Vec<u32> = h.used_entries().map(|e| e.key).collect();
        keys.sort_unstable();
        assert_eq!(keys, (0..1998).collect::<Vec<_>>());
        assert!(h.used_entries().any(|e| e.hash == 2 && e.key == 0), "blank first");
        assert!(h.bin_count.is_power_of_two() && h.bin_count >= 16);
    }
}
