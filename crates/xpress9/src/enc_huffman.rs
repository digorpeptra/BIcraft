// Ported from Microsoft XPress9 (MIT), dev/ese/src/_xpress9/Xpress9EncHuffman.c@7030fe7407615160e54d152e4ef704eede2fdd7e
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! Length-limited canonical Huffman codes and the encoding of their length tables
//! (`Xpress9HuffmanCreateTree`, `Xpress9HuffmanEncodeTable`, `Xpress9HuffmanCreateAndEncodeTable`).
//!
//! Translation notes: `HUFFMAN_NODE` pointers become indices into a vector. Upstream sorts the
//! symbols by count with a merge sort (fewer than 64 symbols) or an LSD radix sort; both are
//! stable and ascending, so a single stable sort yields the identical order. Packed masks are
//! `(codeword << 5) | length` as in `HUFFMAN_MASK`.

use crate::Error;
use crate::misc::{
    BitWriter, HUFFMAN_CODEWORD_LENGTH_BITS, HUFFMAN_CODEWORD_LENGTH_LIMIT, HUFFMAN_ENCODED_TABLE_FILL,
    HUFFMAN_ENCODED_TABLE_PREV, HUFFMAN_ENCODED_TABLE_ROW_0, HUFFMAN_ENCODED_TABLE_ROW_1,
    HUFFMAN_ENCODED_TABLE_SIZE, HUFFMAN_ENCODED_TABLE_ZERO_REPT, HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT,
    HUFFMAN_MAX_CODEWORD_LENGTH, get_msb, huffman_reverse_mask,
};

/// `HUFFMAN_NODE`.
#[derive(Clone, Copy, Default)]
struct Node {
    son: [Option<usize>; 2],
    next: Option<usize>,
    count: u32,
    symbol: u16,
    bits: u16,
}

fn nd(nodes: &[Node], i: usize) -> Result<&Node, Error> {
    nodes.get(i).ok_or(Error::Internal("Huffman node index"))
}

fn nd_mut(nodes: &mut [Node], i: usize) -> Result<&mut Node, Error> {
    nodes.get_mut(i).ok_or(Error::Internal("Huffman node index"))
}

#[inline]
fn codeword_length(mask: u32) -> usize {
    (mask & ((1 << HUFFMAN_CODEWORD_LENGTH_BITS) - 1)) as usize
}

/// `HuffmanCreateTemp`: one node per symbol with a non-zero count, linked in symbol order.
fn huffman_create_temp(nodes: &mut Vec<Node>, counts: &[u32]) -> usize {
    nodes.clear();
    for (i, &c) in counts.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let n = nodes.len();
        nodes.push(Node {
            son: [None, None],
            next: Some(n + 1),
            count: c,
            symbol: i as u16,
            bits: 0,
        });
    }
    if let Some(last) = nodes.last_mut() {
        last.next = None;
    }
    nodes.len()
}

/// `HuffmanMergeSort` / `HuffmanRadixSort`: stable ascending sort of the first `n` nodes by
/// count; returns the list head.
fn huffman_sort(nodes: &mut [Node], n: usize) -> Result<Option<usize>, Error> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut keys = Vec::with_capacity(n);
    for &i in &order {
        keys.push(nd(nodes, i)?.count);
    }
    order.sort_by_key(|&i| keys.get(i).copied().unwrap_or(u32::MAX));
    for w in order.windows(2) {
        if let [a, b] = *w {
            nd_mut(nodes, a)?.next = Some(b);
        }
    }
    if let Some(&last) = order.last() {
        nd_mut(nodes, last)?.next = None;
    }
    Ok(order.first().copied())
}

/// `HuffmanCreateTree`: two-queue Huffman construction; internal nodes are appended from
/// index `base`. Returns the root.
fn huffman_create_tree(nodes: &mut [Node], leaf: Option<usize>, base: usize) -> Result<usize, Error> {
    let count = |nodes: &[Node], i: usize| -> Result<u32, Error> { Ok(nd(nodes, i)?.count) };
    let next = |nodes: &[Node], i: usize| -> Result<Option<usize>, Error> { Ok(nd(nodes, i)?.next) };

    // at least 2 leaves shall be available; form the first intermediate node from two leaves
    let l0 = leaf.ok_or(Error::Internal("Huffman tree needs two leaves"))?;
    let l1 = next(nodes, l0)?.ok_or(Error::Internal("Huffman tree needs two leaves"))?;
    let sum = count(nodes, l0)?
        .checked_add(count(nodes, l1)?)
        .ok_or(Error::Internal("Huffman count overflow"))?;
    let mut p_leaf = next(nodes, l1)?;
    let mut t = base;
    *nd_mut(nodes, t)? = Node {
        son: [Some(l0), Some(l1)],
        next: None,
        count: sum,
        symbol: 0xffff,
        bits: 0,
    };
    let mut p_node = t;

    let add = |a: u32, b: u32| a.checked_add(b).ok_or(Error::Internal("Huffman count overflow"));

    while let Some(l) = p_leaf {
        t += 1;
        nd_mut(nodes, t)?.symbol = 0xffff;
        nd_mut(nodes, t)?.son = [None, None];

        // left son; `right_from_leaf` is the GetRightLeaf / GetRightNode decision
        let right_from_leaf: bool;
        if count(nodes, l)? <= count(nodes, p_node)? {
            let c = count(nodes, l)?;
            let n = nd_mut(nodes, t)?;
            n.son[0] = Some(l);
            n.count = c;
            p_leaf = next(nodes, l)?;
            match p_leaf {
                None => {
                    if p_node == t {
                        return Err(Error::Internal("Huffman tree construction"));
                    }
                    right_from_leaf = false;
                }
                Some(pl) => right_from_leaf = count(nodes, pl)? <= count(nodes, p_node)?,
            }
        } else {
            let c = count(nodes, p_node)?;
            let n = nd_mut(nodes, t)?;
            n.son[0] = Some(p_node);
            n.count = c;
            p_node += 1;
            if p_node == t {
                right_from_leaf = true;
            } else {
                let pl = p_leaf.ok_or(Error::Internal("Huffman tree construction"))?;
                right_from_leaf = count(nodes, pl)? <= count(nodes, p_node)?;
            }
        }

        // right son
        if right_from_leaf {
            let pl = p_leaf.ok_or(Error::Internal("Huffman tree construction"))?;
            let c = count(nodes, pl)?;
            let n = nd_mut(nodes, t)?;
            n.son[1] = Some(pl);
            n.count = add(n.count, c)?;
            p_leaf = next(nodes, pl)?;
        } else {
            let c = count(nodes, p_node)?;
            let n = nd_mut(nodes, t)?;
            n.son[1] = Some(p_node);
            n.count = add(n.count, c)?;
            p_node += 1;
        }
    }

    while p_node < t {
        t += 1;
        let c0 = count(nodes, p_node)?;
        let left = p_node;
        p_node += 1;
        if p_node == t {
            return Err(Error::Internal("Huffman tree construction"));
        }
        let c1 = count(nodes, p_node)?;
        let right = p_node;
        p_node += 1;
        *nd_mut(nodes, t)? = Node {
            son: [Some(left), Some(right)],
            next: None,
            count: add(c0, c1)?,
            symbol: 0xffff,
            bits: 0,
        };
    }

    Ok(p_node)
}

/// `HuffmanComputeBitLength`: number of leaves at each depth.
fn huffman_compute_bit_length(
    nodes: &mut [Node],
    root: usize,
    bit_length_count: &mut [usize; HUFFMAN_CODEWORD_LENGTH_LIMIT],
    alphabet_size: usize,
) -> Result<(), Error> {
    *bit_length_count = [0; HUFFMAN_CODEWORD_LENGTH_LIMIT];
    nd_mut(nodes, root)?.bits = 0;
    let mut stack: Vec<usize> = Vec::with_capacity(HUFFMAN_CODEWORD_LENGTH_LIMIT);
    stack.push(root);
    while let Some(mut r) = stack.pop() {
        let mut bits = nd(nodes, r)?.bits as usize;
        // traverse the tree (it should be right-aligned)
        while let Some(left) = nd(nodes, r)?.son[0] {
            let n = *nd(nodes, r)?;
            let right = n.son[1].ok_or(Error::Internal("left son is not null but right is"))?;
            if n.symbol as usize <= alphabet_size {
                return Err(Error::Internal("intermediate node has a symbol"));
            }
            bits += 1;
            nd_mut(nodes, right)?.bits = bits as u16;
            stack.push(right);
            if stack.len() >= HUFFMAN_CODEWORD_LENGTH_LIMIT {
                return Err(Error::Internal("Huffman tree is way too deep"));
            }
            r = left;
            nd_mut(nodes, r)?.bits = bits as u16;
        }
        let n = nd(nodes, r)?;
        if n.son[1].is_some() {
            return Err(Error::Internal("left son is null but right is not"));
        }
        if n.symbol as usize >= alphabet_size {
            return Err(Error::Internal("leaf symbol out of range"));
        }
        *bit_length_count
            .get_mut(bits)
            .ok_or(Error::Internal("codeword too long"))? += 1;
    }
    Ok(())
}

/// `HuffmanTruncateTree`: make every codeword at most `max_codeword_length` bits.
fn huffman_truncate_tree(
    count: &mut [usize; HUFFMAN_CODEWORD_LENGTH_LIMIT],
    max_codeword_length: usize,
) -> Result<(), Error> {
    let err = || Error::Internal("corrupt Huffman frequency table");
    let mut j = max_codeword_length;
    let mut i = HUFFMAN_CODEWORD_LENGTH_LIMIT - 1;
    while i > max_codeword_length {
        while count.get(i).copied().ok_or_else(err)? != 0 {
            if j == max_codeword_length {
                // find the deepest leaf with codeword length < max_codeword_length
                loop {
                    j = j.checked_sub(1).ok_or_else(err)?;
                    if j == 0 {
                        return Err(err());
                    }
                    if count.get(j).copied().ok_or_else(err)? != 0 {
                        break;
                    }
                }
            }
            if count.get(j).copied().ok_or_else(err)? == 0 {
                return Err(err());
            }
            let c = count.get_mut(j).ok_or_else(err)?;
            *c -= 1;
            *count.get_mut(j + 1).ok_or_else(err)? += 2;
            *count.get_mut(i - 1).ok_or_else(err)? += 1;
            let c = count.get_mut(i).ok_or_else(err)?;
            *c = c.checked_sub(2).ok_or_else(err)?;
            j += 1;
        }
        i -= 1;
    }
    Ok(())
}

/// `HuffmanCanonizeTree`: assign lengths to symbols (rarest get the longest), then return the
/// list of symbols sorted by (length, symbol). Writes the lengths into `masks`.
fn huffman_canonize_tree(
    nodes: &mut [Node],
    sorted_list: Option<usize>,
    bit_length_count: &[usize; HUFFMAN_CODEWORD_LENGTH_LIMIT],
    masks: &mut [u32],
    cost: Option<&mut usize>,
) -> Result<Option<usize>, Error> {
    let err = |m| Error::Internal(m);
    masks.fill(0);
    let mut cost_by_length = [0usize; HUFFMAN_CODEWORD_LENGTH_LIMIT];

    // assign number of bits to symbols
    let mut i = HUFFMAN_CODEWORD_LENGTH_LIMIT;
    let mut n = 0usize;
    let mut p = sorted_list;
    while let Some(node) = p {
        if n == 0 {
            loop {
                i = i.checked_sub(1).ok_or(err("bit length counts"))?;
                n = bit_length_count.get(i).copied().ok_or(err("bit length counts"))?;
                if n != 0 {
                    break;
                }
            }
        }
        let nn = *nd(nodes, node)?;
        *masks
            .get_mut(nn.symbol as usize)
            .ok_or(err("symbol out of range"))? = i as u32;
        *cost_by_length.get_mut(i).ok_or(err("bit length"))? += nn.count as usize;
        n -= 1;
        p = nn.next;
    }
    if n != 0 {
        return Err(err("bit length counts"));
    }

    // sort symbols by length of codeword in a stable manner, reusing the list nodes
    let mut head: [Option<usize>; HUFFMAN_CODEWORD_LENGTH_LIMIT] = [None; HUFFMAN_CODEWORD_LENGTH_LIMIT];
    let mut last: [Option<usize>; HUFFMAN_CODEWORD_LENGTH_LIMIT] = [None; HUFFMAN_CODEWORD_LENGTH_LIMIT];
    let mut p = sorted_list;
    for (symbol, &bits) in masks.iter().enumerate() {
        let bits = bits as usize;
        if bits == 0 {
            continue;
        }
        let node = p.ok_or(err("canonize ran out of nodes"))?;
        if bits >= HUFFMAN_CODEWORD_LENGTH_LIMIT {
            return Err(err("codeword too long"));
        }
        let next = nd(nodes, node)?.next;
        {
            let m = nd_mut(nodes, node)?;
            m.symbol = symbol as u16;
            m.bits = bits as u16;
        }
        match last.get(bits).copied().flatten() {
            Some(l) => nd_mut(nodes, l)?.next = Some(node),
            None => *head.get_mut(bits).ok_or(err("bit length"))? = Some(node),
        }
        *last.get_mut(bits).ok_or(err("bit length"))? = Some(node);
        p = next;
    }
    if p.is_some() {
        return Err(err("canonize left nodes over"));
    }

    // terminate and concatenate the chains
    let mut list: Option<usize> = None;
    for k in (0..HUFFMAN_CODEWORD_LENGTH_LIMIT).rev() {
        match last.get(k).copied().flatten() {
            Some(l) => nd_mut(nodes, l)?.next = list,
            None => *head.get_mut(k).ok_or(err("bit length"))? = list,
        }
        list = head.get(k).copied().flatten();
    }

    if let Some(cost) = cost {
        *cost = 0;
        let mut acc = 0usize;
        for k in (1..HUFFMAN_CODEWORD_LENGTH_LIMIT).rev() {
            acc += cost_by_length.get(k).copied().unwrap_or(0);
            *cost += acc;
        }
    }
    Ok(list)
}

/// `HuffmanCreateCodewords`: canonical codewords, bit-reversed for LSB-first output.
fn huffman_create_codewords(nodes: &[Node], mut p: Option<usize>, masks: &mut [u32]) -> Result<(), Error> {
    let alphabet_size = masks.len();
    let mut bits: usize = 1;
    let mut mask: u64 = 0;
    while let Some(i) = p {
        let n = nd(nodes, i)?;
        let nb = n.bits as usize;
        if !(nb < HUFFMAN_CODEWORD_LENGTH_LIMIT && nb >= bits && (n.symbol as usize) <= alphabet_size) {
            return Err(Error::Internal("codewords not sorted"));
        }
        mask <<= nb - bits;
        bits = nb;
        if bits >= (1 << HUFFMAN_CODEWORD_LENGTH_BITS) {
            return Err(Error::Internal("codeword too long"));
        }
        let packed = (huffman_reverse_mask(mask, bits as u32) << HUFFMAN_CODEWORD_LENGTH_BITS) + bits as u64;
        *masks
            .get_mut(n.symbol as usize)
            .ok_or(Error::Internal("symbol out of range"))? = packed as u32;
        mask += 1;
        if mask > (1u64 << bits) {
            return Err(Error::Internal("codeword overflow"));
        }
        p = n.next;
    }
    if mask != (1u64 << bits) {
        return Err(Error::Internal("incomplete code"));
    }
    Ok(())
}

/// `Xpress9HuffmanCreateTree`: returns the number of symbols with non-zero counts. With no
/// symbols the masks (and cost) are left untouched, exactly as upstream.
fn xpress9_huffman_create_tree(
    counts: &[u32],
    max_codeword_length: usize,
    nodes: &mut Vec<Node>,
    masks: &mut [u32],
    cost: Option<&mut usize>,
) -> Result<usize, Error> {
    if max_codeword_length > HUFFMAN_MAX_CODEWORD_LENGTH {
        return Err(Error::Internal("max codeword length"));
    }
    let alphabet_size = counts.len();
    let n = huffman_create_temp(nodes, counts);
    if n <= 1 {
        if n == 1 {
            masks.fill(0);
            let leaf = *nd(nodes, 0)?;
            *masks
                .get_mut(leaf.symbol as usize)
                .ok_or(Error::Internal("symbol out of range"))? = 1;
            if let Some(cost) = cost {
                *cost = leaf.count as usize;
            }
        }
        return Ok(n);
    }

    let sorted = huffman_sort(nodes, n)?;
    nodes.resize(2 * n, Node::default());
    let root = huffman_create_tree(nodes, sorted, n)?;
    let mut bit_length_count = [0usize; HUFFMAN_CODEWORD_LENGTH_LIMIT];
    huffman_compute_bit_length(nodes, root, &mut bit_length_count, alphabet_size)?;
    huffman_truncate_tree(&mut bit_length_count, max_codeword_length)?;
    let sorted = huffman_canonize_tree(nodes, sorted, &bit_length_count, masks, cost)?;
    huffman_create_codewords(nodes, sorted, masks)?;
    Ok(n)
}

/// Classify one non-zero length for the table encoding (shared by the counting and writing
/// passes of `Xpress9HuffmanEncodeTable`).
fn length_symbol(masks: &[u32], i: usize, k: usize, prev: &mut usize, fill_boundary: usize) -> usize {
    if k == *prev {
        return HUFFMAN_ENCODED_TABLE_PREV;
    }
    *prev = k;
    if i >= fill_boundary {
        let above = codeword_length(masks.get(i - fill_boundary).copied().unwrap_or(0));
        if k == above {
            return HUFFMAN_ENCODED_TABLE_ROW_0;
        } else if k == above + 1 {
            return HUFFMAN_ENCODED_TABLE_ROW_1;
        }
    }
    k
}

/// `Xpress9HuffmanEncodeTable`: write the codeword lengths using a small Huffman code.
fn xpress9_huffman_encode_table(
    w: &mut BitWriter,
    masks: &[u32],
    fill_boundary: usize,
    nodes: &mut Vec<Node>,
) -> Result<(), Error> {
    let alphabet_size = masks.len();
    if !(fill_boundary != 0 && fill_boundary.is_power_of_two() && alphabet_size & (fill_boundary - 1) == 0) {
        return Err(Error::Internal("bad fill boundary"));
    }
    let len_at = |i: usize| codeword_length(masks.get(i).copied().unwrap_or(0));

    // count the symbols of the small alphabet
    let mut count = [0u32; HUFFMAN_ENCODED_TABLE_SIZE];
    let bump = |count: &mut [u32; HUFFMAN_ENCODED_TABLE_SIZE], s: usize, by: u32| {
        if let Some(c) = count.get_mut(s) {
            *c = c.wrapping_add(by);
        }
    };
    let mut prev = 8usize;
    let mut i = 0usize;
    while i < alphabet_size {
        let k = len_at(i);
        if k > HUFFMAN_MAX_CODEWORD_LENGTH {
            return Err(Error::Internal("codeword too long"));
        }
        if k != 0 {
            let s = length_symbol(masks, i, k, &mut prev, fill_boundary);
            bump(&mut count, s, 1);
            i += 1;
        } else {
            // zeros get special treatment
            let mut k = i;
            loop {
                i += 1;
                if !(i < alphabet_size && len_at(i) == 0) {
                    break;
                }
            }
            while (k ^ i) >= fill_boundary {
                bump(&mut count, HUFFMAN_ENCODED_TABLE_FILL, 1);
                k = (k & !(fill_boundary - 1)) + fill_boundary;
                if k > i {
                    return Err(Error::Internal("fill boundary"));
                }
            }
            let k = i - k;
            if k > 0 {
                if k < HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT {
                    bump(&mut count, 0, k as u32);
                } else {
                    bump(&mut count, HUFFMAN_ENCODED_TABLE_ZERO_REPT, 1);
                }
            }
        }
    }

    let mut small_mask = [0u32; HUFFMAN_ENCODED_TABLE_SIZE];
    let k = xpress9_huffman_create_tree(&count, 8, nodes, &mut small_mask, None)?;
    if k == 0 || k > alphabet_size {
        return Err(Error::Internal("small Huffman table"));
    }

    // write the small table
    let mut prev = 4usize;
    for &m in small_mask.iter() {
        let k = codeword_length(m);
        if k == prev {
            w.write(0, 1);
        } else {
            w.write(1, 1);
            if k > prev {
                w.write((k - 1) as u64, 3);
            } else {
                w.write(k as u64, 3);
            }
            prev = k;
        }
    }

    // write the actual table
    let small = |s: usize| small_mask.get(s).copied().unwrap_or(0);
    let mut prev = 8usize;
    let mut i = 0usize;
    while i < alphabet_size {
        let k = len_at(i);
        if k != 0 {
            let s = length_symbol(masks, i, k, &mut prev, fill_boundary);
            w.write_symbol(small(s));
            i += 1;
        } else {
            let mut k = i;
            loop {
                i += 1;
                if !(i < alphabet_size && len_at(i) == 0) {
                    break;
                }
            }
            while (k ^ i) >= fill_boundary {
                w.write_symbol(small(HUFFMAN_ENCODED_TABLE_FILL));
                k = (k & !(fill_boundary - 1)) + fill_boundary;
                if k > i {
                    return Err(Error::Internal("fill boundary"));
                }
            }
            let mut k = i - k;
            if k > 0 {
                if k < HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT {
                    while k != 0 {
                        w.write_symbol(small(0));
                        k -= 1;
                    }
                } else {
                    w.write_symbol(small(HUFFMAN_ENCODED_TABLE_ZERO_REPT));
                    k -= HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT;
                    if k < 3 {
                        w.write(k as u64, 2);
                    } else {
                        w.write(3, 2);
                        k -= 3;
                        while k >= 7 {
                            w.write(7, 3);
                            k -= 7;
                        }
                        w.write(k as u64, 3);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Scratch space reused across tables (`m_Scratch.m_HuffmanNodeTemp`).
#[derive(Default)]
pub(crate) struct HuffmanScratch {
    nodes: Vec<Node>,
}

/// `Xpress9HuffmanCreateAndEncodeTable`: build the code for `counts`, write whichever of the
/// Huffman-coded table or the flat "stored" code is cheaper, fill `masks`, return the cost
/// (table bits plus data bits).
pub(crate) fn xpress9_huffman_create_and_encode_table(
    w: &mut BitWriter,
    counts: &[u32],
    max_codeword_length: usize,
    masks: &mut [u32],
    fill_boundary: usize,
    scratch: &mut HuffmanScratch,
) -> Result<usize, Error> {
    let alphabet_size = counts.len();
    if masks.len() != alphabet_size {
        return Err(Error::Internal("mask array size"));
    }
    // first N symbols may use M = MSB(|A|) bits, the remaining |A| - N use M + 1
    let bits = get_msb(alphabet_size as u64)? as usize;
    let threshold = (1usize << (bits + 1)).saturating_sub(alphabet_size);
    let count0: usize = counts.iter().take(threshold).map(|&c| c as usize).sum();
    let count1: usize = counts.iter().skip(threshold).map(|&c| c as usize).sum();
    let candidate_code_cost = (count0 + count1) * bits + count1 + 3;

    let mut huffman_code_cost = 0usize;
    xpress9_huffman_create_tree(
        counts,
        max_codeword_length,
        &mut scratch.nodes,
        masks,
        Some(&mut huffman_code_cost),
    )?;

    let start = w.mark();
    let start_bits = w.bit_position();
    w.write(1, 3);
    xpress9_huffman_encode_table(w, masks, fill_boundary, &mut scratch.nodes)?;
    huffman_code_cost += (w.bit_position() - start_bits) as usize;

    if candidate_code_cost <= huffman_code_cost {
        w.restore(start);
        w.write(0, 3);
        for (i, m) in masks.iter_mut().enumerate().take(threshold) {
            *m = ((huffman_reverse_mask(i as u64, bits as u32) << HUFFMAN_CODEWORD_LENGTH_BITS) + bits as u64)
                as u32;
        }
        let bits1 = bits + 1;
        for (code, m) in ((threshold as u64) << 1..).zip(masks.iter_mut().skip(threshold)) {
            *m = ((huffman_reverse_mask(code, bits1 as u32) << HUFFMAN_CODEWORD_LENGTH_BITS) + bits1 as u64)
                as u32;
        }
        return Ok(candidate_code_cost);
    }
    Ok(huffman_code_cost)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::dec_huffman::{
        huffman_create_decode_tables, huffman_decode_length_table, huffman_decode_symbol,
    };
    use crate::misc::BitReader;

    fn round_trip(counts: &[u32]) {
        let mut w = BitWriter::new();
        let mut masks = vec![0u32; counts.len()];
        let mut scratch = HuffmanScratch::default();
        xpress9_huffman_create_and_encode_table(&mut w, counts, 27, &mut masks, 16, &mut scratch).unwrap();
        // write every present symbol once
        for (s, &c) in counts.iter().enumerate() {
            if c != 0 {
                w.write_symbol(masks[s]);
            }
        }
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        let mut lengths = vec![0u8; counts.len()];
        huffman_decode_length_table(&mut r, &mut lengths, 16, 27).unwrap();
        let mut table = vec![0i16; 8192];
        let root = huffman_create_decode_tables(&lengths, &mut table, 12, 6).unwrap();
        for (s, &c) in counts.iter().enumerate() {
            if c != 0 {
                assert_eq!(huffman_decode_symbol(&mut r, &table, root, 12, 6).unwrap(), s);
            }
        }
    }

    #[test]
    fn huffman_round_trips() {
        let mut counts = vec![0u32; 704];
        counts[3] = 7;
        counts[100] = 1;
        round_trip(&counts);
        // skewed (Fibonacci-like) counts force codeword truncation to 27 bits
        let mut counts = vec![0u32; 704];
        let (mut a, mut b) = (1u32, 1u32);
        for c in counts.iter_mut().take(40) {
            *c = a;
            let t = a.saturating_add(b);
            a = b;
            b = t;
        }
        round_trip(&counts);
        let counts: Vec<u32> = (0..256u32).map(|i| (i * 7919) % 97 + 1).collect();
        round_trip(&counts);
    }
}
