//! Page allocation, b-tree construction (bulk loading) and page-level copies of b-trees.

use std::collections::BTreeMap;

use crate::btree::PageSource;
use crate::header::{HEADER_LEN, be_u16, be_u32};
use crate::record::read_varint;
use crate::write::record::put_varint;
use crate::{Error, Result, corrupt};

pub(crate) const INDEX_INTERIOR: u8 = 0x02;
pub(crate) const TABLE_INTERIOR: u8 = 0x05;
pub(crate) const INDEX_LEAF: u8 = 0x0a;
pub(crate) const TABLE_LEAF: u8 = 0x0d;

/// The page holding the lock byte at offset 2^30 must never be used; the writer stays below it,
/// which caps every output at 1 GiB.
const PENDING_BYTE: u64 = 0x4000_0000;

/// Deepest source b-tree a page copy follows.
const MAX_DEPTH: usize = 64;

/// Pointer-map entry types.
const PTRMAP_ROOT: u8 = 1;
const PTRMAP_OVERFLOW1: u8 = 3;
const PTRMAP_OVERFLOW2: u8 = 4;
const PTRMAP_BTREE: u8 = 5;

/// Copy `src` into `buf` at `at`, failing instead of panicking when it does not fit.
pub(crate) fn put_bytes(buf: &mut [u8], at: usize, src: &[u8]) -> Result<()> {
    at.checked_add(src.len())
        .and_then(|end| buf.get_mut(at..end))
        .ok_or_else(|| corrupt("write outside the page"))?
        .copy_from_slice(src);
    Ok(())
}

/// The output pages, numbered from 1.
pub(crate) struct Pager {
    pub page_size: usize,
    pub usable: usize,
    auto_vacuum: bool,
    max_pages: u32,
    /// Page contents; an empty vector is a page not written yet (zeros).
    pages: Vec<Vec<u8>>,
    /// Pointer-map entries (auto-vacuum only): page -> (type, parent).
    ptrmap: BTreeMap<u32, (u8, u32)>,
}

impl Pager {
    pub(crate) fn new(page_size: u32, reserved: u8, auto_vacuum: bool) -> Self {
        let max_pages = u32::try_from(PENDING_BYTE / u64::from(page_size)).unwrap_or(u32::MAX);
        Self {
            page_size: page_size as usize,
            usable: page_size as usize - usize::from(reserved),
            auto_vacuum,
            max_pages,
            // page 1 exists from the start
            pages: vec![Vec::new()],
            ptrmap: BTreeMap::new(),
        }
    }

    pub(crate) fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(u32::MAX)
    }

    /// Entries per pointer-map page plus the map page itself.
    fn ptrmap_stride(&self) -> u32 {
        u32::try_from(self.usable / 5 + 1).unwrap_or(u32::MAX)
    }

    fn is_ptrmap_page(&self, p: u32) -> bool {
        self.auto_vacuum && p >= 2 && (p - 2).is_multiple_of(self.ptrmap_stride())
    }

    /// Allocate the next free page number.
    pub(crate) fn alloc(&mut self) -> Result<u32> {
        loop {
            let next = self
                .page_count()
                .checked_add(1)
                .filter(|&n| n <= self.max_pages)
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "the database would exceed {} pages of {} bytes (1 GiB)",
                        self.max_pages, self.page_size
                    ))
                })?;
            self.pages.push(Vec::new());
            if !self.is_ptrmap_page(next) {
                return Ok(next);
            }
        }
    }

    fn slot(&mut self, no: u32) -> Result<&mut Vec<u8>> {
        let i = (no as usize).checked_sub(1).ok_or_else(|| corrupt("page 0"))?;
        self.pages
            .get_mut(i)
            .ok_or_else(|| corrupt(format!("page {no} was never allocated")))
    }

    pub(crate) fn set(&mut self, no: u32, data: Vec<u8>) -> Result<()> {
        *self.slot(no)? = data;
        Ok(())
    }

    pub(crate) fn page1_mut(&mut self) -> Result<&mut Vec<u8>> {
        self.slot(1)
    }

    pub(crate) fn note(&mut self, page: u32, kind: u8, parent: u32) {
        if self.auto_vacuum && page >= 3 {
            self.ptrmap.insert(page, (kind, parent));
        }
    }

    pub(crate) fn note_root(&mut self, page: u32) {
        self.note(page, PTRMAP_ROOT, 0);
    }

    /// Lay out every page, filling the pointer-map pages.
    pub(crate) fn into_bytes(mut self) -> Result<Vec<u8>> {
        if self.auto_vacuum {
            let stride = self.ptrmap_stride();
            let entries: Vec<(u32, (u8, u32))> = self.ptrmap.iter().map(|(k, v)| (*k, *v)).collect();
            for (page, (kind, parent)) in entries {
                let map_page = (page - 2) / stride * stride + 2;
                let at = 5 * (page - map_page - 1) as usize;
                let size = self.page_size;
                let slot = self.slot(map_page)?;
                if slot.is_empty() {
                    *slot = vec![0; size];
                }
                put_bytes(slot, at, &[kind])?;
                put_bytes(slot, at + 1, &parent.to_be_bytes())?;
            }
        }
        let total = self
            .pages
            .len()
            .checked_mul(self.page_size)
            .ok_or_else(|| Error::Invalid("database too large".into()))?;
        let mut out = Vec::with_capacity(total);
        for p in &self.pages {
            if p.is_empty() {
                out.resize(out.len() + self.page_size, 0);
            } else {
                out.extend_from_slice(p);
                out.resize(out.len() + self.page_size - p.len().min(self.page_size), 0);
            }
        }
        Ok(out)
    }
}

/// Local payload size rules (file format §1.6): table leaves and index pages differ in X.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TreeKind {
    Table,
    Index,
}

pub(crate) fn local_size(kind: TreeKind, p: usize, usable: usize) -> usize {
    let u = usable;
    let max_local = match kind {
        TreeKind::Table => u - 35,
        TreeKind::Index => (u - 12) * 64 / 255 - 23,
    };
    if p <= max_local {
        return p;
    }
    let min_local = (u - 12) * 32 / 255 - 23;
    let k = min_local + (p - min_local) % (u - 4);
    if k <= max_local { k } else { min_local }
}

/// A cell ready to be placed on a page; `overflow` is the first overflow page of its payload.
#[derive(Debug, Clone)]
pub(crate) struct Cell {
    pub bytes: Vec<u8>,
    pub overflow: Option<u32>,
}

/// Build a cell: `prefix` (payload-size varint and, for tables, the rowid varint) followed by
/// the local part of `payload`, spilling the rest to a fresh overflow chain.
pub(crate) fn payload_cell(
    pager: &mut Pager,
    kind: TreeKind,
    prefix: Vec<u8>,
    payload: &[u8],
) -> Result<Cell> {
    let local = local_size(kind, payload.len(), pager.usable);
    let mut bytes = prefix;
    let (head, mut rest) = payload.split_at(local.min(payload.len()));
    bytes.extend_from_slice(head);
    if rest.is_empty() {
        return Ok(Cell {
            bytes,
            overflow: None,
        });
    }
    let chunk = pager.usable - 4;
    let first = pager.alloc()?;
    bytes.extend_from_slice(&first.to_be_bytes());
    let mut current = first;
    loop {
        let (part, tail) = rest.split_at(chunk.min(rest.len()));
        rest = tail;
        let next = if rest.is_empty() { 0 } else { pager.alloc()? };
        let mut page = Vec::with_capacity(4 + part.len());
        page.extend_from_slice(&next.to_be_bytes());
        page.extend_from_slice(part);
        pager.set(current, page)?;
        if next == 0 {
            break;
        }
        pager.note(next, PTRMAP_OVERFLOW2, current);
        current = next;
    }
    Ok(Cell {
        bytes,
        overflow: Some(first),
    })
}

/// A table leaf cell.
pub(crate) fn table_leaf_cell(pager: &mut Pager, rowid: i64, payload: &[u8]) -> Result<Cell> {
    let mut prefix = Vec::with_capacity(18);
    put_varint(&mut prefix, payload.len() as u64);
    put_varint(&mut prefix, rowid as u64);
    payload_cell(pager, TreeKind::Table, prefix, payload)
}

/// The body of an index cell (the same on leaf and interior pages, after the child pointer).
pub(crate) fn index_cell(pager: &mut Pager, payload: &[u8]) -> Result<Cell> {
    let mut prefix = Vec::with_capacity(9);
    put_varint(&mut prefix, payload.len() as u64);
    payload_cell(pager, TreeKind::Index, prefix, payload)
}

/// Bytes a cell occupies on a page: its content (never less than 4, the smallest unit SQLite
/// can track) plus its 2-byte pointer.
fn cost(cell_len: usize) -> usize {
    cell_len.max(4) + 2
}

/// A child page and the key that follows it in the parent (`None` only for the last child of
/// an index level).
struct Child {
    page: u32,
    sep: Option<Cell>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Table leaves: every item of a group is a cell.
    Leaf,
    /// Index leaves: the item after a group is its separator, promoted to the parent.
    IndexLeaf,
    /// Interior pages: the last item of a group is the right child (its key goes up).
    Interior,
}

/// Prefix sums of item costs: `pre[i]` is the cost of items `0..i`.
fn prefix_sums(costs: &[usize]) -> Vec<usize> {
    let mut pre = Vec::with_capacity(costs.len() + 1);
    let mut acc = 0usize;
    pre.push(0);
    for c in costs {
        acc = acc.saturating_add(*c);
        pre.push(acc);
    }
    pre
}

/// Cost of the group `a..=b` (items `0..pre.len()-1`), or `None` when the group is too small
/// for `mode`.
fn group_cost(pre: &[usize], a: usize, b: usize, mode: Mode, final_group: bool) -> Option<usize> {
    let (end, min) = match (mode, final_group) {
        (Mode::Leaf, _) | (Mode::IndexLeaf, true) => (b + 1, 1),
        (Mode::IndexLeaf, false) | (Mode::Interior, _) => (b, 2),
    };
    if b + 1 < a + min {
        return None;
    }
    Some(pre.get(end)?.saturating_sub(*pre.get(a)?))
}

/// Find the most balanced split of `a..=last` into a non-final group and a final group, both
/// fitting `cap`.
fn best_split(pre: &[usize], a: usize, last: usize, mode: Mode, cap: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut best_diff = usize::MAX;
    for m in a..last {
        let (Some(l), Some(r)) = (
            group_cost(pre, a, m, mode, false),
            group_cost(pre, m + 1, last, mode, true),
        ) else {
            continue;
        };
        if l <= cap && r <= cap {
            let diff = l.abs_diff(r);
            if diff < best_diff {
                best_diff = diff;
                best = Some((m, m + 1));
            }
        }
    }
    best
}

/// Partition the items of one level into page groups (inclusive ranges): greedy filling, then
/// the last two pages are rebalanced (which also repairs a final page that would be empty).
fn partition(costs: &[usize], mode: Mode, cap: usize) -> Result<Vec<(usize, usize)>> {
    let n = costs.len();
    if n == 0 {
        return Err(corrupt("cannot partition an empty level"));
    }
    let pre = prefix_sums(costs);
    let fits = |a: usize, b: usize, fin: bool| group_cost(&pre, a, b, mode, fin).is_some_and(|c| c <= cap);
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut a = 0;
    while a < n {
        if fits(a, n - 1, true) {
            groups.push((a, n - 1));
            break;
        }
        let excl_last = mode != Mode::Leaf;
        if excl_last && a + 1 >= n {
            break; // a lone item cannot form a page; the rebalance below absorbs it
        }
        // Greedy: the largest group starting at `a` that fits.
        let mut b = a;
        if excl_last {
            // cells a..b fit; item b closes the group
            while b + 2 < n && fits(a, b + 1, false) {
                b += 1;
            }
            if b == a {
                b = a + 1;
                if !fits(a, b, false) {
                    return Err(corrupt("a cell does not fit in a page"));
                }
            }
        } else {
            if !fits(a, a, false) {
                return Err(corrupt("a cell does not fit in a page"));
            }
            while b + 1 < n && fits(a, b + 1, false) {
                b += 1;
            }
        }
        groups.push((a, b));
        a = b + 1;
    }
    let tail_valid = groups
        .last()
        .is_some_and(|&(x, y)| y == n - 1 && fits(x, y, true));
    if groups.len() >= 2 || !tail_valid {
        let start = if tail_valid {
            groups.len() - 2
        } else {
            groups.len().saturating_sub(1)
        };
        let a0 = groups.get(start).map_or(0, |g| g.0);
        groups.truncate(start);
        match best_split(&pre, a0, n - 1, mode, cap) {
            Some((m, m1)) => {
                groups.push((a0, m));
                groups.push((m1, n - 1));
            }
            None if fits(a0, n - 1, true) => groups.push((a0, n - 1)),
            None => return Err(corrupt("cannot balance the last b-tree pages")),
        }
    }
    Ok(groups)
}

/// Write one b-tree page.
fn write_page(
    pager: &mut Pager,
    no: u32,
    kind: u8,
    cells: &[&Cell],
    prefix: &[Vec<u8>],
    right: Option<u32>,
) -> Result<()> {
    let hdr_off = if no == 1 { HEADER_LEN } else { 0 };
    let hdr_len = if right.is_some() { 12 } else { 8 };
    let ptrs = hdr_off + hdr_len;
    let mut page = vec![0u8; pager.page_size];
    let mut content = pager.usable;
    for (i, cell) in cells.iter().enumerate() {
        let pre = prefix.get(i).map_or(&[][..], Vec::as_slice);
        let len = pre.len() + cell.bytes.len();
        let start = content
            .checked_sub(len.max(4))
            .filter(|&s| s >= ptrs + 2 * cells.len())
            .ok_or_else(|| corrupt("page overfull"))?;
        content = start;
        put_bytes(&mut page, start, pre)?;
        put_bytes(&mut page, start + pre.len(), &cell.bytes)?;
        put_bytes(&mut page, ptrs + 2 * i, &(start as u16).to_be_bytes())?;
        if let Some(ov) = cell.overflow {
            pager.note(ov, PTRMAP_OVERFLOW1, no);
        }
    }
    let n = u16::try_from(cells.len()).map_err(|_| corrupt("too many cells"))?;
    put_bytes(&mut page, hdr_off, &[kind])?;
    put_bytes(&mut page, hdr_off + 3, &n.to_be_bytes())?;
    // 65536 is stored as 0
    put_bytes(&mut page, hdr_off + 5, &((content % 65536) as u16).to_be_bytes())?;
    if let Some(r) = right {
        put_bytes(&mut page, hdr_off + 8, &r.to_be_bytes())?;
    }
    pager.set(no, page)
}

/// Leaf items of a table b-tree: cells in rowid order with their rowids.
pub(crate) enum Leaves {
    Table(Vec<(i64, Cell)>),
    Index(Vec<Cell>),
}

/// Bulk-load a b-tree whose root page number is already reserved.
pub(crate) fn build_tree(pager: &mut Pager, root: u32, leaves: Leaves) -> Result<()> {
    pager.note_root(root);
    let full_leaf = pager.usable - 8;
    let full_interior = pager.usable - 12;
    let root_extra = if root == 1 { HEADER_LEN } else { 0 };
    let (leaf_kind, interior_kind) = match leaves {
        Leaves::Table(_) => (TABLE_LEAF, TABLE_INTERIOR),
        Leaves::Index(_) => (INDEX_LEAF, INDEX_INTERIOR),
    };

    // Leaf level.
    let mut children: Vec<Child> = match leaves {
        Leaves::Table(items) => {
            let costs: Vec<usize> = items.iter().map(|(_, c)| cost(c.bytes.len())).collect();
            let total: usize = costs.iter().sum();
            if total <= full_leaf - root_extra {
                let cells: Vec<&Cell> = items.iter().map(|(_, c)| c).collect();
                return write_page(pager, root, leaf_kind, &cells, &[], None);
            }
            let mut groups = partition(&costs, Mode::Leaf, full_leaf)?;
            if groups.len() == 1 && items.len() >= 2 {
                // fits a normal page but not page 1: split in two
                let (m, m1) = best_split(&prefix_sums(&costs), 0, items.len() - 1, Mode::Leaf, full_leaf)
                    .ok_or_else(|| corrupt("cannot split the root page"))?;
                groups = vec![(0, m), (m1, items.len() - 1)];
            }
            let mut out = Vec::with_capacity(groups.len());
            for (a, b) in groups {
                let page = pager.alloc()?;
                let slice = items.get(a..=b).ok_or_else(|| corrupt("group out of range"))?;
                let cells: Vec<&Cell> = slice.iter().map(|(_, c)| c).collect();
                write_page(pager, page, leaf_kind, &cells, &[], None)?;
                let key = slice.last().map_or(0, |(k, _)| *k);
                let mut sep = Vec::with_capacity(9);
                put_varint(&mut sep, key as u64);
                out.push(Child {
                    page,
                    sep: Some(Cell {
                        bytes: sep,
                        overflow: None,
                    }),
                });
            }
            out
        }
        Leaves::Index(items) => {
            let costs: Vec<usize> = items.iter().map(|c| cost(c.bytes.len())).collect();
            let total: usize = costs.iter().sum();
            if total <= full_leaf - root_extra {
                let cells: Vec<&Cell> = items.iter().collect();
                return write_page(pager, root, leaf_kind, &cells, &[], None);
            }
            let groups = partition(&costs, Mode::IndexLeaf, full_leaf)?;
            let n = items.len();
            let mut items: Vec<Option<Cell>> = items.into_iter().map(Some).collect();
            let mut out = Vec::with_capacity(groups.len());
            for (a, b) in groups {
                let page = pager.alloc()?;
                let last = b + 1 == n;
                let cell_end = if last { b + 1 } else { b };
                let cells: Vec<Cell> = items
                    .get_mut(a..cell_end)
                    .ok_or_else(|| corrupt("group out of range"))?
                    .iter_mut()
                    .filter_map(Option::take)
                    .collect();
                let refs: Vec<&Cell> = cells.iter().collect();
                write_page(pager, page, leaf_kind, &refs, &[], None)?;
                let sep = if last {
                    None
                } else {
                    items.get_mut(b).and_then(Option::take)
                };
                out.push(Child { page, sep });
            }
            out
        }
    };

    // Interior levels.
    loop {
        let costs: Vec<usize> = children
            .iter()
            .map(|c| cost(4 + c.sep.as_ref().map_or(0, |s| s.bytes.len())))
            .collect();
        let n = children.len();
        let fits_root = group_cost(&prefix_sums(&costs), 0, n.saturating_sub(1), Mode::Interior, true)
            .is_some_and(|c| c <= full_interior - root_extra);
        if fits_root || n == 1 {
            // n == 1 only happens on page 1 when a single cell is too large for it: SQLite then
            // keeps page 1 as an interior page with no cells and one right child.
            return write_interior(pager, root, interior_kind, children);
        }
        let groups = partition(&costs, Mode::Interior, full_interior)?;
        let mut slots: Vec<Option<Child>> = children.into_iter().map(Some).collect();
        let mut next = Vec::with_capacity(groups.len());
        for (a, b) in groups {
            let page = pager.alloc()?;
            let group: Vec<Child> = slots
                .get_mut(a..=b)
                .ok_or_else(|| corrupt("group out of range"))?
                .iter_mut()
                .filter_map(Option::take)
                .collect();
            let up = write_interior_take_sep(pager, page, interior_kind, group)?;
            next.push(Child { page, sep: up });
        }
        children = next;
    }
}

/// Write an interior page from `group` (the last child is the right child); returns the key of
/// the right child, which the parent uses as this page's key.
fn write_interior_take_sep(
    pager: &mut Pager,
    no: u32,
    kind: u8,
    mut group: Vec<Child>,
) -> Result<Option<Cell>> {
    let last = group.pop().ok_or_else(|| corrupt("empty interior page"))?;
    let mut prefixes = Vec::with_capacity(group.len());
    let mut cells = Vec::with_capacity(group.len());
    for c in &group {
        prefixes.push(c.page.to_be_bytes().to_vec());
        let sep = c
            .sep
            .as_ref()
            .ok_or_else(|| corrupt("interior cell without a key"))?;
        cells.push(sep);
    }
    write_page(pager, no, kind, &cells, &prefixes, Some(last.page))?;
    for c in &group {
        pager.note(c.page, PTRMAP_BTREE, no);
    }
    pager.note(last.page, PTRMAP_BTREE, no);
    Ok(last.sep)
}

fn write_interior(pager: &mut Pager, no: u32, kind: u8, group: Vec<Child>) -> Result<()> {
    write_interior_take_sep(pager, no, kind, group).map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Page-level copy of a b-tree from a source database

/// Copies b-trees page by page from a source image, renumbering child and overflow pointers.
pub(crate) struct Copier<'a> {
    src: PageSource<'a>,
    seen: Vec<bool>,
}

impl<'a> Copier<'a> {
    pub(crate) fn new(src: PageSource<'a>) -> Self {
        Self {
            seen: vec![false; src.page_count as usize + 1],
            src,
        }
    }

    fn visit(&mut self, page: u32) -> Result<()> {
        let slot = self
            .seen
            .get_mut(page as usize)
            .ok_or_else(|| corrupt(format!("page {page} out of range")))?;
        if *slot {
            return Err(corrupt(format!(
                "page {page} is reached twice (cycle or shared page)"
            )));
        }
        *slot = true;
        Ok(())
    }

    /// Copy the b-tree rooted at `src_root` into `dst_root` (already allocated).
    pub(crate) fn copy_tree(&mut self, pager: &mut Pager, src_root: u32, dst_root: u32) -> Result<()> {
        if src_root < 2 {
            return Err(corrupt(format!("cannot copy a b-tree rooted at page {src_root}")));
        }
        if pager.page_size != self.src.page_size as usize || pager.usable != self.src.usable_size as usize {
            return Err(Error::Invalid(
                "page copies need the source page and usable sizes".into(),
            ));
        }
        pager.note_root(dst_root);
        let mut stack: Vec<(u32, u32, usize)> = vec![(src_root, dst_root, 0)];
        let mut family: Option<bool> = None;
        while let Some((src_no, dst_no, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(corrupt(format!("b-tree deeper than {MAX_DEPTH} levels")));
            }
            self.visit(src_no)?;
            let src_page = self.src.page(src_no)?;
            let mut page = src_page.to_vec();
            page.resize(pager.page_size, 0);
            let kind = *page.first().ok_or_else(|| corrupt("empty page"))?;
            let (interior, table) = match kind {
                TABLE_INTERIOR => (true, true),
                TABLE_LEAF => (false, true),
                INDEX_INTERIOR => (true, false),
                INDEX_LEAF => (false, false),
                k => {
                    return Err(corrupt(format!(
                        "page {src_no} has invalid b-tree page type {k:#04x}"
                    )));
                }
            };
            if *family.get_or_insert(table) != table {
                return Err(corrupt(format!("page {src_no} mixes table and index b-trees")));
            }
            let n_cells = usize::from(be_u16(&page, 3)?);
            let hdr_len = if interior { 12 } else { 8 };
            let usable = pager.usable;
            if hdr_len + 2 * n_cells > usable {
                return Err(corrupt(format!("page {src_no}: {n_cells} cells do not fit")));
            }
            for i in 0..n_cells {
                let off = usize::from(be_u16(&page, hdr_len + 2 * i)?);
                if off < hdr_len + 2 * n_cells || off >= usable {
                    return Err(corrupt(format!("page {src_no}: cell pointer {off} out of range")));
                }
                let mut at = off;
                if interior {
                    let child = be_u32(&page, at)?;
                    let new_child = pager.alloc()?;
                    page.get_mut(at..at + 4)
                        .ok_or_else(|| corrupt("cell past the page"))?
                        .copy_from_slice(&new_child.to_be_bytes());
                    pager.note(new_child, PTRMAP_BTREE, dst_no);
                    stack.push((child, new_child, depth + 1));
                    at += 4;
                    if table {
                        continue; // a table interior cell is the child pointer and a rowid
                    }
                }
                let (p, n) = read_varint(&page, at)?;
                at += n;
                if table {
                    let (_, n) = read_varint(&page, at)?;
                    at += n;
                }
                let total = usize::try_from(p)
                    .ok()
                    .filter(|&t| t as u64 <= u64::from(self.src.page_count) * usable as u64)
                    .ok_or_else(|| corrupt(format!("page {src_no}: payload larger than the file")))?;
                let tree = if table { TreeKind::Table } else { TreeKind::Index };
                let local = local_size(tree, total, usable);
                if local < total {
                    let ptr_at = at + local;
                    let first = be_u32(&page, ptr_at)?;
                    let new_first = self.copy_overflow(pager, first, total - local, dst_no)?;
                    page.get_mut(ptr_at..ptr_at + 4)
                        .ok_or_else(|| corrupt("overflow pointer past the page"))?
                        .copy_from_slice(&new_first.to_be_bytes());
                } else if at + local > usable {
                    return Err(corrupt(format!("page {src_no}: cell runs past the page")));
                }
            }
            if interior {
                let right = be_u32(&page, 8)?;
                let new_right = pager.alloc()?;
                put_bytes(&mut page, 8, &new_right.to_be_bytes())?;
                pager.note(new_right, PTRMAP_BTREE, dst_no);
                stack.push((right, new_right, depth + 1));
            }
            // reserved bytes are not carried over
            if let Some(r) = page.get_mut(usable..) {
                r.fill(0);
            }
            pager.set(dst_no, page)?;
        }
        Ok(())
    }

    /// Copy an overflow chain holding `remaining` bytes; returns its new first page.
    fn copy_overflow(
        &mut self,
        pager: &mut Pager,
        first: u32,
        mut remaining: usize,
        cell_page: u32,
    ) -> Result<u32> {
        let chunk = pager.usable - 4;
        let new_first = pager.alloc()?;
        pager.note(new_first, PTRMAP_OVERFLOW1, cell_page);
        let (mut src_no, mut dst_no) = (first, new_first);
        loop {
            self.visit(src_no)?;
            let src = self.src.page(src_no)?;
            let take = chunk.min(remaining);
            remaining -= take;
            let next_src = be_u32(src, 0)?;
            let next_dst = if remaining == 0 { 0 } else { pager.alloc()? };
            if remaining > 0 && next_src == 0 {
                return Err(corrupt(format!("overflow chain ends early at page {src_no}")));
            }
            let mut page = Vec::with_capacity(4 + take);
            page.extend_from_slice(&next_dst.to_be_bytes());
            page.extend_from_slice(
                src.get(4..4 + take)
                    .ok_or_else(|| corrupt("overflow page too short"))?,
            );
            pager.set(dst_no, page)?;
            if next_dst == 0 {
                return Ok(new_first);
            }
            pager.note(next_dst, PTRMAP_OVERFLOW2, dst_no);
            src_no = next_src;
            dst_no = next_dst;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sizes() {
        assert_eq!(local_size(TreeKind::Table, 477, 512), 477);
        assert_eq!(local_size(TreeKind::Table, 478, 512), 39);
        assert_eq!(local_size(TreeKind::Table, 39 + 508 + 100, 512), 139);
        // index: X = ((512-12)*64/255)-23 = 102, M = 39
        assert_eq!(local_size(TreeKind::Index, 102, 512), 102);
        assert_eq!(local_size(TreeKind::Index, 103, 512), 39);
        assert_eq!(local_size(TreeKind::Index, 39 + 508 + 50, 512), 89);
    }

    fn check(costs: &[usize], mode: Mode, cap: usize) {
        let g = partition(costs, mode, cap).unwrap();
        let pre = prefix_sums(costs);
        let mut covered = 0;
        for (i, &(a, b)) in g.iter().enumerate() {
            assert_eq!(a, covered, "{g:?}");
            let last = i + 1 == g.len();
            let c = group_cost(&pre, a, b, mode, last).unwrap_or_else(|| panic!("{g:?}"));
            assert!(c <= cap, "{g:?}");
            covered = b + 1;
        }
        assert_eq!(covered, costs.len(), "{g:?}");
    }

    #[test]
    fn partition_rules() {
        for n in 1..40 {
            for cap in [10, 25, 33, 100, 250] {
                let costs: Vec<usize> = (0..n).map(|i| 5 + (i * 7) % 6).collect();
                check(&costs, Mode::Leaf, cap);
                if n >= 2 {
                    check(&costs, Mode::Interior, cap.max(30));
                }
                check(&costs, Mode::IndexLeaf, cap.max(30));
            }
        }
    }
}
