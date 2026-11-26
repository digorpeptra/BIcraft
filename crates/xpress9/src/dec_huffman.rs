// Ported from Microsoft XPress9 (MIT), dev/ese/src/_xpress9/Xpress9DecHuffman.c@7030fe7407615160e54d152e4ef704eede2fdd7e
// and the HUFFMAN_DECODE_SYMBOL macro of dev/ese/src/_xpress9/Xpress9Internal.h@7030fe7407615160e54d152e4ef704eede2fdd7e
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! Huffman decoding tables: `HuffmanCreateDecodeTables`, `HuffmanDecodeLengthTable` and the
//! `HUFFMAN_DECODE_SYMBOL` lookup.
//!
//! Translation notes: the linked lists of `HUFFMAN_DECODE_NODE` become indices into a vector;
//! every `RETAIL_ASSERT` becomes an `Error::Internal` return and every table access is
//! bounds-checked, so a hostile length table yields `Err` instead of memory corruption.

use crate::Error;
use crate::misc::{
    BitReader, HUFFMAN_CODEWORD_LENGTH_LIMIT, HUFFMAN_ENCODED_TABLE_FILL, HUFFMAN_ENCODED_TABLE_PREV,
    HUFFMAN_ENCODED_TABLE_ROW_0, HUFFMAN_ENCODED_TABLE_ROW_1, HUFFMAN_ENCODED_TABLE_SIZE,
    HUFFMAN_ENCODED_TABLE_ZERO_REPT, HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT, get_msb,
    huffman_reverse_mask,
};

/// `HUFFMAN_DECODE_TABLE_ENTRY`.
pub(crate) type DecodeEntry = i16;

/// `HUFFMAN_DECODE_NODE`.
#[derive(Clone, Copy, Default)]
struct DecodeNode {
    next: Option<usize>,
    symbol: u16,
    bits: u16,
}

/// One `Chain[i]` of the sort: head and last element (the upstream `m_ppLast`).
#[derive(Clone, Copy, Default)]
struct Chain {
    head: Option<usize>,
    last: Option<usize>,
}

fn node(nodes: &[DecodeNode], i: usize) -> Result<DecodeNode, Error> {
    nodes.get(i).copied().ok_or(Error::Internal("decode node index"))
}

fn set_next(nodes: &mut [DecodeNode], i: usize, next: Option<usize>) -> Result<(), Error> {
    nodes.get_mut(i).ok_or(Error::Internal("decode node index"))?.next = next;
    Ok(())
}

/// `HuffmanDecodeSortSymbols`: list of present symbols sorted descending by codeword length,
/// then descending by symbol. Returns the head of the list.
fn huffman_decode_sort_symbols(
    bit_length_table: &[u8],
    nodes: &mut Vec<DecodeNode>,
    bit_length_count: &mut [usize; HUFFMAN_CODEWORD_LENGTH_LIMIT],
    symbol_count: &mut usize,
) -> Result<usize, Error> {
    let mut chain = [Chain::default(); HUFFMAN_CODEWORD_LENGTH_LIMIT];
    *symbol_count = 0;
    *bit_length_count = [0; HUFFMAN_CODEWORD_LENGTH_LIMIT];

    for i in (0..bit_length_table.len()).rev() {
        let n = bit_length_table.get(i).copied().unwrap_or(0) as usize;
        if n == 0 {
            continue;
        }
        if n >= HUFFMAN_CODEWORD_LENGTH_LIMIT {
            return Err(Error::Internal("excessively large bit length"));
        }
        let idx = nodes.len();
        nodes.push(DecodeNode {
            next: None,
            symbol: i as u16,
            bits: n as u16,
        });
        if let Some(c) = bit_length_count.get_mut(n) {
            *c += 1;
        }
        let ch = chain.get_mut(n).ok_or(Error::Internal("chain index"))?;
        match ch.last {
            Some(last) => set_next(nodes, last, Some(idx))?,
            None => ch.head = Some(idx),
        }
        ch.last = Some(idx);
        *symbol_count += 1;
    }

    // build the list of symbols sorted in descending order by bit length and then by symbol code
    let mut leaf: Option<usize> = None;
    for ch in chain.iter() {
        if let Some(last) = ch.last {
            set_next(nodes, last, leaf)?;
            leaf = ch.head;
        }
    }
    leaf.ok_or(Error::Internal("no symbols in Huffman table"))
}

/// `HuffmanDecodeVerifyTree`: the code must form a full binary tree.
fn huffman_decode_verify_tree(
    bit_length_count: &[usize; HUFFMAN_CODEWORD_LENGTH_LIMIT],
) -> Result<(), Error> {
    let mut n: usize = 0;
    for i in (1..HUFFMAN_CODEWORD_LENGTH_LIMIT).rev() {
        n = n.saturating_add(bit_length_count.get(i).copied().unwrap_or(0));
        if n & 1 != 0 {
            return Err(Error::CorruptedData("Huffman tree is not full"));
        }
        n >>= 1;
    }
    if n != 1 || bit_length_count.first().copied().unwrap_or(0) != 0 {
        return Err(Error::CorruptedData("Huffman tree is not full"));
    }
    Ok(())
}

/// `HuffmanDecodeFillTables`: returns the number of table entries used.
fn huffman_decode_fill_tables(
    decode_table: &mut [DecodeEntry],
    mut leaf: Option<usize>,
    root_bits: usize,
    tail_bits: usize,
    nodes: &mut Vec<DecodeNode>,
) -> Result<usize, Error> {
    if !(root_bits >= 4 && tail_bits >= 4 && root_bits <= 15 && tail_bits <= 15 && root_bits >= tail_bits) {
        return Err(Error::Internal("bad root/tail bits"));
    }

    let mut table_offset: usize = 0;
    let mut p_node: Option<usize> = None;
    let mut pp_link: Option<usize> = None;
    let mut root_level: usize;
    loop {
        // start next table
        let leaf_node = node(nodes, leaf.ok_or(Error::Internal("ran out of leaves"))?)?;
        let mut leaf_level = leaf_node.bits as usize;
        if let Some(pn) = p_node {
            let pn_bits = node(nodes, pn)?.bits as usize;
            if leaf_level < pn_bits {
                leaf_level = pn_bits;
            }
        }

        if leaf_level <= root_bits {
            root_level = 0;
            leaf_level = root_bits;
        } else {
            root_level = leaf_level - tail_bits;
        }

        let span = leaf_level - root_level;
        let mut i: usize = 1usize << span;
        if table_offset + i > decode_table.len() {
            return Err(Error::Internal("decode table overflow"));
        }

        // populate one table (the root table or one of the next-level tables)
        loop {
            let entry: i64;
            let bits: usize;
            let link = match p_node {
                Some(pn) => {
                    let n = node(nodes, pn)?;
                    if n.bits as usize > root_level {
                        Some(n)
                    } else {
                        None
                    }
                }
                None => None,
            };
            if let Some(n) = link {
                let symbol = (n.symbol as usize) << tail_bits;
                if symbol >= table_offset {
                    return Err(Error::Internal("table link points forward"));
                }
                bits = n.bits as usize;
                p_node = n.next;
                entry = symbol as i64 - table_offset as i64 + (bits as i64 - root_level as i64);
            } else {
                let l = node(nodes, leaf.ok_or(Error::Internal("ran out of leaves"))?)?;
                bits = l.bits as usize;
                leaf = l.next;
                entry = ((l.symbol as i64) << 4) + (bits as i64 - root_level as i64);
            }
            let entry = DecodeEntry::try_from(entry).map_err(|_| Error::Internal("decode entry overflow"))?;

            let fill = leaf_level
                .checked_sub(bits)
                .filter(|s| *s < usize::BITS as usize)
                .map(|s| 1usize << s)
                .ok_or(Error::Internal("codeword longer than table level"))?;
            if i < fill {
                return Err(Error::Internal("table entry count"));
            }
            let j = i - fill;
            loop {
                i -= 1;
                let slot = table_offset + huffman_reverse_mask(i as u64, span as u32) as usize;
                *decode_table
                    .get_mut(slot)
                    .ok_or(Error::Internal("decode table index"))? = entry;
                if i == j {
                    break;
                }
            }
            if i == 0 {
                break;
            }
        }

        let t = nodes.len();
        nodes.push(DecodeNode {
            next: None,
            symbol: (table_offset >> tail_bits) as u16,
            bits: root_level as u16,
        });
        if p_node.is_none() {
            p_node = Some(t);
        } else if let Some(link) = pp_link {
            set_next(nodes, link, Some(t))?;
        }
        pp_link = Some(t);
        table_offset += 1usize << span;

        if root_level == 0 {
            break;
        }
    }

    let root = node(nodes, p_node.ok_or(Error::Internal("no root table"))?)?;
    if root.next.is_some() {
        return Err(Error::Internal("dangling table node"));
    }
    Ok(table_offset)
}

/// `HuffmanCreateDecodeTables`: returns the index of the root table inside `decode_table`.
pub(crate) fn huffman_create_decode_tables(
    bit_length_table: &[u8],
    decode_table: &mut [DecodeEntry],
    root_bits: usize,
    tail_bits: usize,
) -> Result<usize, Error> {
    if root_bits < tail_bits {
        return Err(Error::Internal("root bits < tail bits"));
    }
    let mut bit_length_count = [0usize; HUFFMAN_CODEWORD_LENGTH_LIMIT];
    let mut num_symbols = 0usize;
    let mut nodes: Vec<DecodeNode> = Vec::with_capacity(bit_length_table.len() + 80);
    let leaf = huffman_decode_sort_symbols(
        bit_length_table,
        &mut nodes,
        &mut bit_length_count,
        &mut num_symbols,
    )?;

    if num_symbols == 1 {
        // A tree with a single symbol: fill the root table with it.
        let l = node(&nodes, leaf)?;
        let entry = ((l.symbol as i64) << 4) + l.bits as i64;
        let entry = DecodeEntry::try_from(entry).map_err(|_| Error::Internal("decode entry overflow"))?;
        let mut i = 1usize << root_bits;
        loop {
            i -= 1;
            let slot = huffman_reverse_mask(i as u64, root_bits as u32) as usize;
            *decode_table
                .get_mut(slot)
                .ok_or(Error::Internal("decode table index"))? = entry;
            if i == 0 {
                break;
            }
        }
        return Ok(0);
    }

    huffman_decode_verify_tree(&bit_length_count)?;

    let n = huffman_decode_fill_tables(decode_table, Some(leaf), root_bits, tail_bits, &mut nodes)?;
    if n < (1usize << root_bits) {
        return Err(Error::Internal("decode tables too small"));
    }
    Ok(n - (1usize << root_bits))
}

/// `HUFFMAN_DECODE_SYMBOL (iSymbol, pDecodeTable, ROOT_LOOKUP_BITS, TAIL_LOOKUP_BITS)`.
#[inline]
pub(crate) fn huffman_decode_symbol(
    br: &mut BitReader<'_>,
    table: &[DecodeEntry],
    root: usize,
    root_lookup_bits: u32,
    tail_lookup_bits: u32,
) -> Result<usize, Error> {
    let lookup = |br: &BitReader<'_>, base: isize, bits: u32| -> Result<i32, Error> {
        let idx = base
            .checked_add(br.peek(bits) as isize)
            .ok_or(Error::CorruptedData("Huffman table walk"))?;
        let idx = usize::try_from(idx).map_err(|_| Error::CorruptedData("Huffman table walk"))?;
        table
            .get(idx)
            .map(|e| i32::from(*e))
            .ok_or(Error::CorruptedData("Huffman table walk"))
    };
    let mut symbol = lookup(br, root as isize, root_lookup_bits)?;
    if symbol < 0 {
        br.skip((symbol & 15) as u32);
        let mut next_table = (root as isize) + (symbol & !15) as isize;
        loop {
            symbol = lookup(br, next_table, tail_lookup_bits)?;
            if symbol >= 0 {
                break;
            }
            br.skip((symbol & 15) as u32);
            next_table += (symbol & !15) as isize;
        }
    }
    br.skip((symbol & 15) as u32);
    Ok((symbol >> 4) as usize)
}

/// `HuffmanDecodeLengthTable`: restore the sequence of codeword lengths.
pub(crate) fn huffman_decode_length_table(
    br: &mut BitReader<'_>,
    bit_length_table: &mut [u8],
    fill_boundary: usize,
    max_codeword_length: usize,
) -> Result<(), Error> {
    let alphabet_size = bit_length_table.len();
    let mut k = br.load(3) as usize;
    if k == 0 {
        // "stored" encoding: the first N symbols use M = MSB(|A|) bits, the others M + 1.
        let bits = get_msb(alphabet_size as u64)? as usize;
        let threshold = (1usize << (bits + 1)).saturating_sub(alphabet_size);
        for (i, slot) in bit_length_table.iter_mut().enumerate() {
            *slot = if i < threshold {
                bits as u8
            } else {
                (bits + 1) as u8
            };
        }
        return Ok(());
    }
    if k != 1 {
        return Err(Error::CorruptedData("unknown Huffman table encoding"));
    }

    // read the small table
    let mut small_table_length = [0u8; HUFFMAN_ENCODED_TABLE_SIZE];
    let mut prev_symbol: usize = 4;
    for slot in small_table_length.iter_mut() {
        k = br.load(1) as usize;
        if k == 0 {
            *slot = prev_symbol as u8;
        } else {
            k = br.load(3) as usize;
            if k >= prev_symbol {
                k += 1;
                if k > 8 {
                    return Err(Error::CorruptedData("small Huffman table length"));
                }
            }
            *slot = k as u8;
            prev_symbol = k;
        }
    }

    let mut decode_table: [DecodeEntry; (1 << 8) + (2 << 4)] = [0; (1 << 8) + (2 << 4)];
    let root =
        huffman_create_decode_tables(&small_table_length, &mut decode_table, 8, 4).map_err(|e| match e {
            Error::Internal(_) => Error::CorruptedData("bad small Huffman table"),
            other => other,
        })?;

    let set = |table: &mut [u8], i: usize, v: u8| -> Result<(), Error> {
        *table
            .get_mut(i)
            .ok_or(Error::CorruptedData("length table overrun"))? = v;
        Ok(())
    };
    let get = |table: &[u8], i: usize| -> Result<u8, Error> {
        table
            .get(i)
            .copied()
            .ok_or(Error::CorruptedData("length table index"))
    };

    prev_symbol = 8;
    let mut i = 0usize;
    while i < alphabet_size {
        let symbol = huffman_decode_symbol(br, &decode_table, root, 8, 1)?;
        match symbol {
            HUFFMAN_ENCODED_TABLE_FILL => loop {
                set(bit_length_table, i, 0)?;
                i += 1;
                if !((i & (fill_boundary - 1)) != 0 && i < alphabet_size) {
                    break;
                }
            },
            HUFFMAN_ENCODED_TABLE_ZERO_REPT => {
                k = br.load(2) as usize;
                let mut count = k + HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT;
                let bad = |count: usize| i + count > alphabet_size || (i ^ (i + count)) >= fill_boundary;
                if k == 3 {
                    loop {
                        k = br.load(3) as usize;
                        count += k;
                        if bad(count) {
                            return Err(Error::CorruptedData("zero run crosses fill boundary"));
                        }
                        if k != 7 {
                            break;
                        }
                    }
                }
                if bad(count) {
                    return Err(Error::CorruptedData("zero run crosses fill boundary"));
                }
                loop {
                    set(bit_length_table, i, 0)?;
                    i += 1;
                    count -= 1;
                    if !(i < alphabet_size && count != 0) {
                        break;
                    }
                }
            }
            HUFFMAN_ENCODED_TABLE_PREV => {
                set(bit_length_table, i, prev_symbol as u8)?;
                i += 1;
            }
            HUFFMAN_ENCODED_TABLE_ROW_0 => {
                if i < fill_boundary {
                    return Err(Error::CorruptedData("ROW_0 in first row"));
                }
                let v = get(bit_length_table, i - fill_boundary)?;
                set(bit_length_table, i, v)?;
                if v == 0 {
                    return Err(Error::CorruptedData("ROW_0 copies zero length"));
                }
                prev_symbol = v as usize;
                i += 1;
            }
            HUFFMAN_ENCODED_TABLE_ROW_1 => {
                if i < fill_boundary {
                    return Err(Error::CorruptedData("ROW_1 in first row"));
                }
                let v = get(bit_length_table, i - fill_boundary)?.wrapping_add(1);
                set(bit_length_table, i, v)?;
                if v as usize > max_codeword_length || v == 0 {
                    return Err(Error::CorruptedData("ROW_1 length out of range"));
                }
                prev_symbol = v as usize;
                i += 1;
            }
            _ => {
                if symbol > max_codeword_length {
                    return Err(Error::CorruptedData("codeword length out of range"));
                }
                set(bit_length_table, i, symbol as u8)?;
                i += 1;
                if symbol != 0 {
                    prev_symbol = symbol;
                }
            }
        }
    }
    Ok(())
}
