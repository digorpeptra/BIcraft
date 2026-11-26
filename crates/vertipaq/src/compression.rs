//! Compression class identifiers stored in `.idfmeta` (see `docs/formats/vertipaq.md`).

/// Base of the `XMRENoSplitCompressionInfo<N>` class ids: the id is `BASE + N`.
pub const NO_SPLIT_BASE: u32 = 703_030;
/// `XMHybridRLECompressionInfo<…>`: RLE runs plus a bit-packed sub-segment.
pub const HYBRID_RLE: u32 = 703_066;
/// `XM123CompressionInfo`: no stored values, the ids count up from the minimum.
pub const XM123: u32 = 703_067;
/// Class id written in the sub-class slot when there is no inner compression.
pub const NONE: u32 = 1;

/// Bit widths allowed for `XMRENoSplit` packing ([MS-XLDM] §2.7.1).
pub const BIT_WIDTHS: [u8; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 21, 32];

/// The compression of one segment (or of the bit-packed part of a hybrid segment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Values packed in `n` bits, `64 / n` per little-endian `u64` word, low bits first.
    NoSplit(u8),
    /// Hybrid RLE: run entries plus bit-packed runs using the inner compression.
    HybridRle(InnerCompression),
    /// Consecutive ids, nothing stored (row-number columns).
    Xm123,
}

/// The bit-packed half of a hybrid RLE segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InnerCompression {
    NoSplit(u8),
    Xm123,
}

impl Compression {
    /// Interprets a (class, sub-class) pair from `.idfmeta`; `None` if unknown.
    pub fn from_classes(class: u32, sub_class: u32) -> Option<Compression> {
        match class {
            HYBRID_RLE => {
                let inner = match sub_class {
                    XM123 => InnerCompression::Xm123,
                    other => InnerCompression::NoSplit(no_split_width(other)?),
                };
                Some(Compression::HybridRle(inner))
            }
            XM123 => Some(Compression::Xm123),
            other => no_split_width(other).map(Compression::NoSplit),
        }
    }

    /// The class id of this compression, as written in `.idfmeta`.
    pub fn class_id(self) -> u32 {
        match self {
            Compression::NoSplit(n) => NO_SPLIT_BASE + u32::from(n),
            Compression::HybridRle(_) => HYBRID_RLE,
            Compression::Xm123 => XM123,
        }
    }

    /// The sub-class id written next to [`Compression::class_id`].
    pub fn sub_class_id(self) -> u32 {
        match self {
            Compression::HybridRle(InnerCompression::NoSplit(n)) => NO_SPLIT_BASE + u32::from(n),
            Compression::HybridRle(InnerCompression::Xm123) => XM123,
            _ => NONE,
        }
    }
}

/// Bit width of an `XMRENoSplit` class id, if it is one.
pub fn no_split_width(class: u32) -> Option<u8> {
    let n = class.checked_sub(NO_SPLIT_BASE)?;
    let n = u8::try_from(n).ok()?;
    BIT_WIDTHS.contains(&n).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_ids_round_trip() {
        for n in BIT_WIDTHS {
            let c = Compression::NoSplit(n);
            assert_eq!(Compression::from_classes(c.class_id(), c.sub_class_id()), Some(c));
            let h = Compression::HybridRle(InnerCompression::NoSplit(n));
            assert_eq!(Compression::from_classes(h.class_id(), h.sub_class_id()), Some(h));
        }
        assert_eq!(
            Compression::from_classes(703_046, 1),
            Some(Compression::NoSplit(16))
        );
        assert_eq!(
            Compression::from_classes(703_066, 703_067),
            Some(Compression::HybridRle(InnerCompression::Xm123))
        );
        assert_eq!(Compression::from_classes(703_041, 1), None);
        assert_eq!(Compression::from_classes(703_066, 1), None);
        assert_eq!(Compression::from_classes(0, 0), None);
    }
}
