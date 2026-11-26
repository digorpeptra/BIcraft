//! Pages, table b-tree traversal and overflow chains.

use crate::header::{HEADER_LEN, TextEncoding, be_u16, be_u32};
use crate::record::{Row, Value, decode_record, read_varint};
use crate::{Result, corrupt};

/// B-tree page type bytes.
const INDEX_INTERIOR: u8 = 0x02;
const TABLE_INTERIOR: u8 = 0x05;
const INDEX_LEAF: u8 = 0x0a;
const TABLE_LEAF: u8 = 0x0d;

/// Deepest b-tree we follow. Real trees stay far below this (SQLite itself caps cursors at 20
/// levels); anything deeper is treated as corruption.
const MAX_DEPTH: usize = 64;

/// Read access to the pages of a database image.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageSource<'a> {
    pub bytes: &'a [u8],
    pub page_size: u32,
    pub usable_size: u32,
    pub page_count: u32,
    pub encoding: TextEncoding,
}

impl<'a> PageSource<'a> {
    /// The usable part of page `no` (1-based).
    pub(crate) fn page(&self, no: u32) -> Result<&'a [u8]> {
        if no == 0 || no > self.page_count {
            return Err(corrupt(format!("page {no} out of range 1..={}", self.page_count)));
        }
        let start = u64::from(no - 1) * u64::from(self.page_size);
        let end = start + u64::from(self.usable_size);
        let (start, end) = match (usize::try_from(start), usize::try_from(end)) {
            (Ok(s), Ok(e)) => (s, e),
            _ => return Err(corrupt(format!("page {no} beyond the address space"))),
        };
        self.bytes
            .get(start..end)
            .ok_or_else(|| corrupt(format!("page {no} is past the end of the file")))
    }
}

/// Pages already reached during one traversal: a page reached twice means a cycle or pages
/// shared between cells, both of which are corruption. This bounds every traversal by the
/// number of pages in the file.
struct Visited {
    bits: Vec<u64>,
}

impl Visited {
    fn new(page_count: u32) -> Self {
        let words = (page_count as usize) / 64 + 1;
        Self { bits: vec![0; words] }
    }

    fn insert(&mut self, page: u32) -> Result<()> {
        let word = (page / 64) as usize;
        let bit = 1u64 << (page % 64);
        let slot = self
            .bits
            .get_mut(word)
            .ok_or_else(|| corrupt(format!("page {page} out of range")))?;
        if *slot & bit != 0 {
            return Err(corrupt(format!(
                "page {page} is reached twice (cycle or shared page)"
            )));
        }
        *slot |= bit;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PageHeader {
    kind: u8,
    n_cells: usize,
    right_child: u32,
    /// Offset of the cell pointer array within the page.
    cell_ptrs: usize,
}

fn page_header(page: &[u8], page_no: u32) -> Result<PageHeader> {
    let off = if page_no == 1 { HEADER_LEN } else { 0 };
    let kind = *page
        .get(off)
        .ok_or_else(|| corrupt(format!("page {page_no} has no b-tree header")))?;
    let interior = match kind {
        TABLE_INTERIOR | INDEX_INTERIOR => true,
        TABLE_LEAF | INDEX_LEAF => false,
        k => {
            return Err(corrupt(format!(
                "page {page_no} has invalid b-tree page type {k:#04x}"
            )));
        }
    };
    let n_cells = usize::from(be_u16(page, off + 3)?);
    let (right_child, hdr_len) = if interior {
        (be_u32(page, off + 8)?, 12)
    } else {
        (0, 8)
    };
    let cell_ptrs = off + hdr_len;
    if cell_ptrs + 2 * n_cells > page.len() {
        return Err(corrupt(format!(
            "page {page_no}: {n_cells} cells do not fit in the page"
        )));
    }
    Ok(PageHeader {
        kind,
        n_cells,
        right_child,
        cell_ptrs,
    })
}

fn cell_offset(page: &[u8], hdr: &PageHeader, i: usize, page_no: u32) -> Result<usize> {
    let off = usize::from(be_u16(page, hdr.cell_ptrs + 2 * i)?);
    if off < hdr.cell_ptrs + 2 * hdr.n_cells || off >= page.len() {
        return Err(corrupt(format!(
            "page {page_no}: cell pointer {off} outside the cell content area"
        )));
    }
    Ok(off)
}

/// How many payload bytes of a table-leaf cell are stored on the b-tree page itself.
fn local_payload(p: u64, usable: u32) -> u64 {
    let u = u64::from(usable);
    let max_local = u - 35;
    if p <= max_local {
        return p;
    }
    let min_local = (u - 12) * 32 / 255 - 23;
    let k = min_local + (p - min_local) % (u - 4);
    if k <= max_local { k } else { min_local }
}

/// Assemble a cell payload of `total` bytes whose first bytes start at `start` on `page`.
fn read_payload(
    src: &PageSource<'_>,
    visited: &mut Visited,
    page: &[u8],
    page_no: u32,
    start: usize,
    total: u64,
) -> Result<Vec<u8>> {
    let usable = u64::from(src.usable_size);
    // An honest payload never needs more pages than the file has, which also caps allocation.
    if total > u64::from(src.page_count) * usable {
        return Err(corrupt(format!(
            "page {page_no}: payload of {total} bytes is larger than the file"
        )));
    }
    let local = local_payload(total, src.usable_size);
    let local_usize = usize::try_from(local).map_err(|_| corrupt("payload size overflow"))?;
    let total_usize = usize::try_from(total).map_err(|_| corrupt("payload size overflow"))?;
    let local_end = start
        .checked_add(local_usize)
        .ok_or_else(|| corrupt("payload offset overflow"))?;
    let local_bytes = page
        .get(start..local_end)
        .ok_or_else(|| corrupt(format!("page {page_no}: cell payload runs past the page")))?;
    if local == total {
        return Ok(local_bytes.to_vec());
    }
    let mut out = Vec::with_capacity(total_usize);
    out.extend_from_slice(local_bytes);
    let mut next = be_u32(page, local_end)?;
    let chunk = src.usable_size as usize - 4;
    while out.len() < total_usize {
        if next == 0 {
            return Err(corrupt(format!("page {page_no}: overflow chain ends early")));
        }
        visited.insert(next)?;
        let ov = src.page(next)?;
        let take = chunk.min(total_usize - out.len());
        let data = ov
            .get(4..4 + take)
            .ok_or_else(|| corrupt(format!("overflow page {next} too short")))?;
        out.extend_from_slice(data);
        next = be_u32(ov, 0)?;
    }
    Ok(out)
}

/// How the values of a stored record map onto a table's declared columns.
#[derive(Debug, Clone, Default)]
pub(crate) struct RowLayout {
    /// For each declared column: `Some(index in the record)`, or `None` for a VIRTUAL generated
    /// column, which is not stored.
    pub storage: Vec<Option<usize>>,
    /// The `INTEGER PRIMARY KEY` column, which reads as the rowid.
    pub rowid_alias: Option<usize>,
    /// Columns with REAL affinity, whose stored integers read back as reals.
    pub real: Vec<bool>,
    /// Value of each declared column when a record is shorter than the table (columns added by
    /// `ALTER TABLE ... ADD COLUMN` after the row was written).
    pub defaults: Vec<Value>,
}

impl RowLayout {
    /// A layout for a table with `n` plain columns and no rowid alias.
    pub fn plain(n: usize) -> Self {
        Self {
            storage: (0..n).map(Some).collect(),
            rowid_alias: None,
            real: vec![false; n],
            defaults: vec![Value::Null; n],
        }
    }

    fn build(&self, rowid: i64, record: Vec<Value>) -> Vec<Value> {
        let stored = self.storage.iter().filter(|s| s.is_some()).count();
        let mut record: Vec<Option<Value>> = record.into_iter().map(Some).collect();
        let mut values = Vec::with_capacity(self.storage.len().max(record.len()));
        for (ci, slot) in self.storage.iter().enumerate() {
            let v = if self.rowid_alias == Some(ci) {
                Value::Integer(rowid)
            } else {
                match slot {
                    None => Value::Null,
                    Some(s) => match record.get_mut(*s).and_then(Option::take) {
                        Some(v) => v,
                        None => self.defaults.get(ci).cloned().unwrap_or(Value::Null),
                    },
                }
            };
            let v = match v {
                Value::Integer(i) if self.real.get(ci).copied().unwrap_or(false) => Value::Real(i as f64),
                v => v,
            };
            values.push(v);
        }
        values.extend(record.into_iter().skip(stored).flatten());
        values
    }
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    page_no: u32,
    hdr: PageHeader,
    /// Next cell to visit; for interior pages `n_cells` means "the right child".
    next: usize,
}

/// Iterator over the rows of a rowid table, in rowid order.
///
/// Yields `Err` once and then stops if the file turns out to be malformed.
pub struct Rows<'a> {
    src: PageSource<'a>,
    layout: RowLayout,
    stack: Vec<Frame>,
    visited: Visited,
    failed: bool,
}

impl std::fmt::Debug for Rows<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rows")
            .field("depth", &self.stack.len())
            .field("failed", &self.failed)
            .finish()
    }
}

impl<'a> Rows<'a> {
    pub(crate) fn new(src: PageSource<'a>, root: u32, layout: RowLayout) -> Result<Self> {
        let mut rows = Self {
            src,
            layout,
            stack: Vec::new(),
            visited: Visited::new(src.page_count),
            failed: false,
        };
        rows.push(root)?;
        Ok(rows)
    }

    fn push(&mut self, page_no: u32) -> Result<()> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(corrupt(format!("b-tree deeper than {MAX_DEPTH} levels")));
        }
        self.visited.insert(page_no)?;
        let page = self.src.page(page_no)?;
        let hdr = page_header(page, page_no)?;
        if hdr.kind != TABLE_INTERIOR && hdr.kind != TABLE_LEAF {
            return Err(corrupt(format!(
                "page {page_no}: index page inside a table b-tree"
            )));
        }
        self.stack.push(Frame {
            page_no,
            hdr,
            next: 0,
        });
        Ok(())
    }

    fn leaf_row(&mut self, page_no: u32, hdr: &PageHeader, i: usize) -> Result<Row> {
        let page = self.src.page(page_no)?;
        let off = cell_offset(page, hdr, i, page_no)?;
        let (payload_len, n1) = read_varint(page, off)?;
        let (rowid, n2) = read_varint(page, off + n1)?;
        let payload = read_payload(
            &self.src,
            &mut self.visited,
            page,
            page_no,
            off + n1 + n2,
            payload_len,
        )?;
        let record = decode_record(&payload, self.src.encoding)?;
        let rowid = rowid as i64;
        Ok(Row {
            rowid,
            values: self.layout.build(rowid, record),
        })
    }

    fn step(&mut self) -> Result<Option<Row>> {
        loop {
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            let Frame { page_no, hdr, next } = *frame;
            if hdr.kind == TABLE_LEAF {
                if next < hdr.n_cells {
                    frame.next += 1;
                    return self.leaf_row(page_no, &hdr, next).map(Some);
                }
                self.stack.pop();
                continue;
            }
            let child = if next < hdr.n_cells {
                let page = self.src.page(page_no)?;
                let off = cell_offset(page, &hdr, next, page_no)?;
                be_u32(page, off)?
            } else if next == hdr.n_cells {
                hdr.right_child
            } else {
                self.stack.pop();
                continue;
            };
            frame.next += 1;
            self.push(child)?;
        }
    }
}

impl Iterator for Rows<'_> {
    type Item = Result<Row>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.step() {
            Ok(Some(row)) => Some(Ok(row)),
            Ok(None) => None,
            Err(e) => {
                self.failed = true;
                self.stack.clear();
                Some(Err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_payload_formula() {
        // usable 512: max local 477, min local 39
        assert_eq!(local_payload(477, 512), 477);
        assert_eq!(local_payload(478, 512), 39);
        assert_eq!(local_payload(39 + 508 + 100, 512), 139);
        assert_eq!(local_payload(39 + 508, 512), 39);
        // usable 65536
        assert_eq!(local_payload(65501, 65536), 65501);
        assert!(local_payload(70000, 65536) <= 65501);
    }

    #[test]
    fn visited_detects_repeat() {
        let mut v = Visited::new(10);
        v.insert(3).unwrap();
        assert!(v.insert(3).is_err());
        assert!(v.insert(10).is_ok());
        assert!(v.insert(10_000).is_err());
    }
}
