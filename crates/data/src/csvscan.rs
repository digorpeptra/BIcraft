//! The fast CSV paths' byte-level tokenizer and parallel driver (`docs/formats/data-sources.md`,
//! *Fast CSV loading*).
//!
//! Two dialects are tokenized here, each exactly as its reference implementation splits text:
//!
//! - [`Quoting::Rfc`]: [`crate::csv::read_csv`]'s RFC 4180 rules (`table.import {csv}`). A quote
//!   opens a field only at its start, `""` is a quote, nothing but a delimiter or a line break
//!   may follow the closing quote, and an unterminated quote is an error.
//! - [`Quoting::MCsv`] and [`Quoting::MNone`]: `Csv.Document` with `QuoteStyle.Csv` (quoted
//!   fields may hold line breaks) or `QuoteStyle.None` (a line break always ends a record),
//!   where text after a closing quote is part of the field and nothing is an error.
//!
//! The input is valid UTF-8 (`&str`); every special character is ASCII and the delimiter is a
//! whole UTF-8 string, so field boundaries always fall on character boundaries.
//!
//! **Parallel parsing.** A region of text is cut into chunks at line feeds and each chunk is
//! parsed on its own thread as if it started a record. A line feed may sit inside a quoted
//! field, so each chunk's start is then checked, in order, against where the previous chunk's
//! last record really ended; a chunk that started in the wrong place is parsed again from the
//! right place. The result is always the sequential one.

use std::ops::Range;

/// The quoting rules of a dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Quoting {
    /// [`crate::csv::read_csv`].
    Rfc,
    /// `Csv.Document`, `QuoteStyle.Csv`.
    MCsv,
    /// `Csv.Document`, `QuoteStyle.None`.
    MNone,
}

/// A tokenizer configuration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dialect<'d> {
    pub quoting: Quoting,
    /// The delimiter: non-empty, without `"`, CR or LF.
    pub delim: &'d [u8],
    /// Longest field (bytes, after unescaping) before [`Problem::CellTooLong`].
    pub max_cell: usize,
    /// Most fields of one record before [`Problem::TooManyFields`] (`Rfc` only).
    pub max_fields: usize,
}

impl Dialect<'_> {
    /// Whether the delimiter is usable by this tokenizer.
    pub fn valid_delim(delim: &[u8]) -> bool {
        !delim.is_empty() && !delim.iter().any(|b| matches!(b, b'"' | b'\r' | b'\n'))
    }
}

/// Why a record could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Problem {
    /// `Rfc`: an unterminated quote or text after a closing quote.
    Syntax,
    /// A field longer than [`Dialect::max_cell`] (for the M dialects, the length checks
    /// `Csv.Document` makes, see [`scan_m`]).
    CellTooLong,
    /// `Rfc`: more than [`Dialect::max_fields`] fields.
    TooManyFields,
}

/// One step of the tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scan {
    /// A record was read; the next one starts at this offset.
    Record(usize),
    /// The text ends inside a record (or right after a CR) and is not the end of the input.
    Incomplete,
    /// End of input.
    End,
    /// The record starting here cannot be read.
    Bad(Problem),
}

#[derive(Debug, Clone, Copy)]
struct FieldPos {
    start: usize,
    end: usize,
    /// In [`Rec::scratch`] rather than in the text.
    owned: bool,
    quoted: bool,
}

/// The fields of the last record read, reused between records.
#[derive(Debug, Default)]
pub(crate) struct Rec {
    fields: Vec<FieldPos>,
    scratch: String,
    /// Number of fields of the record (only the first `keep` are stored).
    count: usize,
}

impl Rec {
    pub fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.fields.clear();
        self.scratch.clear();
        self.count = 0;
    }

    /// Number of fields in the record.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Field `i` (when stored) and whether it was quoted.
    pub fn field<'a>(&'a self, text: &'a str, i: usize) -> Option<(&'a str, bool)> {
        let f = self.fields.get(i)?;
        let src = if f.owned { self.scratch.as_str() } else { text };
        Some((src.get(f.start..f.end).unwrap_or(""), f.quoted))
    }
}

/// A field being built: a slice of the text while possible, else a copy in the scratch buffer.
struct Building {
    borrowed: Option<Range<usize>>,
    owned_start: usize,
    owned: bool,
    len: usize,
}

impl Building {
    fn new(scratch: &str) -> Self {
        Self {
            borrowed: None,
            owned_start: scratch.len(),
            owned: false,
            len: 0,
        }
    }

    fn append(&mut self, text: &str, r: Range<usize>, scratch: &mut String) {
        if r.is_empty() {
            return;
        }
        self.len = self.len.saturating_add(r.len());
        if self.owned {
            scratch.push_str(text.get(r).unwrap_or(""));
            return;
        }
        match &self.borrowed {
            None => self.borrowed = Some(r),
            Some(b) if b.end == r.start => self.borrowed = Some(b.start..r.end),
            Some(b) => {
                let b = b.clone();
                self.owned = true;
                scratch.push_str(text.get(b).unwrap_or(""));
                scratch.push_str(text.get(r).unwrap_or(""));
            }
        }
    }

    fn quote(&mut self, text: &str, scratch: &mut String) {
        if !self.owned {
            self.owned = true;
            if let Some(b) = self.borrowed.take() {
                scratch.push_str(text.get(b).unwrap_or(""));
            }
        }
        scratch.push('"');
        self.len = self.len.saturating_add(1);
    }

    fn finish(self, quoted: bool, scratch: &str) -> FieldPos {
        if self.owned {
            FieldPos {
                start: self.owned_start,
                end: scratch.len(),
                owned: true,
                quoted,
            }
        } else {
            let r = self.borrowed.unwrap_or(0..0);
            FieldPos {
                start: r.start,
                end: r.end,
                owned: false,
                quoted,
            }
        }
    }
}

/// Whether the delimiter starts at `i`: `Some(true)`/`Some(false)`, or `None` when the text
/// ends inside a possible delimiter and more input may follow.
fn delim_at(b: &[u8], i: usize, delim: &[u8], eof: bool) -> Option<bool> {
    let rest = b.get(i..).unwrap_or_default();
    if rest.starts_with(delim) {
        return Some(true);
    }
    if !eof && rest.len() < delim.len() && delim.starts_with(rest) && !rest.is_empty() {
        return None;
    }
    Some(false)
}

/// The end of an unquoted run starting at `i`: the first CR, LF or (whole) delimiter, or the
/// end of the text.
fn unquoted_end(b: &[u8], i: usize, delim: &[u8]) -> usize {
    let d0 = delim.first().copied().unwrap_or(b'\n');
    let single = delim.len() == 1;
    let mut k = i;
    while let Some(&c) = b.get(k) {
        if c == b'\r' || c == b'\n' {
            return k;
        }
        if c == d0 && (single || b.get(k..).is_some_and(|r| r.starts_with(delim))) {
            return k;
        }
        k += 1;
    }
    b.len()
}

/// Reads the record starting at `pos`, storing at most `keep` fields in `rec`.
pub(crate) fn scan(text: &str, pos: usize, eof: bool, d: &Dialect<'_>, keep: usize, rec: &mut Rec) -> Scan {
    match d.quoting {
        Quoting::Rfc => scan_rfc(text, pos, eof, d, keep, rec),
        Quoting::MCsv | Quoting::MNone => scan_m(text, pos, eof, d, keep, rec),
    }
}

/// `read_csv`'s record rules (see the module documentation).
fn scan_rfc(text: &str, pos: usize, eof: bool, d: &Dialect<'_>, keep: usize, rec: &mut Rec) -> Scan {
    let b = text.as_bytes();
    let n = b.len();
    rec.clear();
    if pos >= n {
        return if eof { Scan::End } else { Scan::Incomplete };
    }
    let mut i = pos;
    loop {
        let mut f = Building::new(&rec.scratch);
        let quoted = b.get(i) == Some(&b'"');
        if quoted {
            let mut j = i + 1;
            loop {
                let Some(k) = b
                    .get(j..)
                    .and_then(|r| r.iter().position(|&c| c == b'"'))
                    .map(|k| k + j)
                else {
                    return if eof {
                        Scan::Bad(Problem::Syntax)
                    } else {
                        Scan::Incomplete
                    };
                };
                if k + 1 >= n && !eof {
                    return Scan::Incomplete;
                }
                f.append(text, j..k, &mut rec.scratch);
                if b.get(k + 1) == Some(&b'"') {
                    f.quote(text, &mut rec.scratch);
                    j = k + 2;
                } else {
                    i = k + 1;
                    break;
                }
            }
            if f.len > d.max_cell {
                return Scan::Bad(Problem::CellTooLong);
            }
            match b.get(i) {
                None if !eof => return Scan::Incomplete,
                None | Some(b'\r' | b'\n') => {}
                Some(_) => match delim_at(b, i, d.delim, eof) {
                    None => return Scan::Incomplete,
                    Some(true) => {}
                    Some(false) => return Scan::Bad(Problem::Syntax),
                },
            }
        } else {
            let k = unquoted_end(b, i, d.delim);
            if k >= n && !eof {
                return Scan::Incomplete;
            }
            f.append(text, i..k, &mut rec.scratch);
            if f.len > d.max_cell {
                return Scan::Bad(Problem::CellTooLong);
            }
            i = k;
        }
        rec.count += 1;
        if rec.count <= keep {
            let fp = f.finish(quoted, &rec.scratch);
            rec.fields.push(fp);
        }
        if rec.count > d.max_fields {
            return Scan::Bad(Problem::TooManyFields);
        }
        if i < n && b.get(i..).is_some_and(|r| r.starts_with(d.delim)) {
            i += d.delim.len();
            continue;
        }
        return match b.get(i) {
            None => Scan::Record(n),
            Some(b'\r') => {
                if i + 1 >= n && !eof {
                    Scan::Incomplete
                } else if b.get(i + 1) == Some(&b'\n') {
                    Scan::Record(i + 2)
                } else {
                    Scan::Record(i + 1)
                }
            }
            Some(_) => Scan::Record(i + 1),
        };
    }
}

/// `Csv.Document`'s record rules (see the module documentation).
///
/// Length checks, as `Csv.Document` makes them: a quoted part is checked after each `""` it
/// holds, and the field is checked when text follows the quoted part or the field is not
/// quoted. A field over [`Dialect::max_cell`] is [`Problem::CellTooLong`].
fn scan_m(text: &str, pos: usize, eof: bool, d: &Dialect<'_>, keep: usize, rec: &mut Rec) -> Scan {
    let b = text.as_bytes();
    let n = b.len();
    let span_lines = d.quoting == Quoting::MCsv;
    rec.clear();
    if pos >= n {
        return if eof { Scan::End } else { Scan::Incomplete };
    }
    let mut i = pos;
    loop {
        let mut f = Building::new(&rec.scratch);
        let quoted = b.get(i) == Some(&b'"');
        let mut too_long = false;
        if quoted {
            let mut j = i + 1;
            loop {
                let stop = b.get(j..).and_then(|r| {
                    r.iter()
                        .position(|&c| c == b'"' || (!span_lines && (c == b'\r' || c == b'\n')))
                });
                let Some(k) = stop.map(|k| k + j) else {
                    if !eof {
                        return Scan::Incomplete;
                    }
                    f.append(text, j..n, &mut rec.scratch);
                    i = n;
                    break;
                };
                f.append(text, j..k, &mut rec.scratch);
                if b.get(k) != Some(&b'"') {
                    i = k;
                    break;
                }
                if k + 1 >= n && !eof {
                    return Scan::Incomplete;
                }
                if b.get(k + 1) == Some(&b'"') {
                    f.quote(text, &mut rec.scratch);
                    if f.len > d.max_cell {
                        too_long = true;
                    }
                    j = k + 2;
                } else {
                    i = k + 1;
                    break;
                }
            }
        }
        let k = unquoted_end(b, i, d.delim);
        if k >= n && !eof {
            return Scan::Incomplete;
        }
        if k > i {
            f.append(text, i..k, &mut rec.scratch);
            if f.len > d.max_cell {
                too_long = true;
            }
        }
        i = k;
        if too_long {
            return Scan::Bad(Problem::CellTooLong);
        }
        rec.count += 1;
        if rec.count <= keep {
            let fp = f.finish(quoted, &rec.scratch);
            rec.fields.push(fp);
        }
        if i < n && b.get(i..).is_some_and(|r| r.starts_with(d.delim)) {
            i += d.delim.len();
            continue;
        }
        return match b.get(i) {
            None => Scan::Record(n),
            Some(b'\r') => {
                if i + 1 >= n && !eof {
                    Scan::Incomplete
                } else if b.get(i + 1) == Some(&b'\n') {
                    Scan::Record(i + 2)
                } else {
                    Scan::Record(i + 1)
                }
            }
            Some(_) => Scan::Record(i + 1),
        };
    }
}

/// How a chunk's parse stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The next record starts at or after the chunk's nominal end.
    Nominal,
    /// The region ends inside a record ([`ChunkOut::end`] is where it starts).
    Incomplete,
    /// End of input.
    End,
    /// The record at [`ChunkOut::end`] cannot be read.
    Bad(Problem),
    /// The consumer stopped the parse (its output says why).
    Halt,
}

/// A chunk's parse: where it started and ended, why it stopped, and the consumer's output.
#[derive(Debug)]
pub(crate) struct ChunkOut<T> {
    pub start: usize,
    pub end: usize,
    pub stop: Stop,
    pub out: T,
}

/// Parses records from `from` until one starts at or after `nominal_end`, handing each to
/// `each` (which returns `false` to stop). The consumer's state is created by `init`.
#[allow(clippy::too_many_arguments)] // one chunk's whole configuration, called in two places
pub(crate) fn parse_chunk<T>(
    text: &str,
    from: usize,
    nominal_end: usize,
    eof: bool,
    d: &Dialect<'_>,
    keep: usize,
    init: impl FnOnce() -> T,
    mut each: impl FnMut(&mut T, &Rec) -> bool,
) -> ChunkOut<T> {
    let mut out = init();
    let mut rec = Rec::new();
    let mut pos = from;
    let stop = loop {
        if pos >= nominal_end && pos < text.len() {
            break Stop::Nominal;
        }
        match scan(text, pos, eof, d, keep, &mut rec) {
            Scan::Record(next) => {
                if !each(&mut out, &rec) {
                    pos = next;
                    break Stop::Halt;
                }
                pos = next;
            }
            Scan::Incomplete => break Stop::Incomplete,
            Scan::End => break Stop::End,
            Scan::Bad(p) => break Stop::Bad(p),
        }
    };
    ChunkOut {
        start: from,
        end: pos,
        stop,
        out,
    }
}

/// Worker threads for parsing: the machine's parallelism, at most 32.
pub(crate) fn threads() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, 32)
}

/// Smallest chunk worth a thread (tiny in tests, so small inputs exercise the chunking).
#[cfg(not(test))]
const MIN_CHUNK: usize = 256 << 10;
#[cfg(test)]
const MIN_CHUNK: usize = 97;

/// Parses `text[start..]` in parallel chunks (see the module documentation) and returns the
/// chunks' outputs in order, each starting where the previous one really ended. `work(from,
/// nominal_end)` parses one chunk (normally with [`parse_chunk`]). The outputs stop at the
/// first chunk that did not stop at its nominal end.
pub(crate) fn run_region<T: Send>(
    text: &str,
    start: usize,
    threads: usize,
    work: &(dyn Fn(usize, usize) -> ChunkOut<T> + Sync),
) -> Vec<ChunkOut<T>> {
    let b = text.as_bytes();
    let len = b.len();
    let span = len.saturating_sub(start);
    let parts = (span / MIN_CHUNK).clamp(1, threads.max(1));
    let mut bounds = vec![start];
    for p in 1..parts {
        let nominal = start + span / parts * p;
        let prev = bounds.last().copied().unwrap_or(start);
        if nominal <= prev {
            continue;
        }
        if let Some(nl) = b.get(nominal..).and_then(|r| r.iter().position(|&c| c == b'\n')) {
            let at = nominal + nl + 1;
            if at < len && at > prev {
                bounds.push(at);
            }
        }
    }
    bounds.push(len);
    let ranges: Vec<(usize, usize)> = bounds
        .windows(2)
        .filter_map(|w| Some((*w.first()?, *w.get(1)?)))
        .collect();
    let mut outs: Vec<Option<ChunkOut<T>>> = Vec::with_capacity(ranges.len());
    if ranges.len() <= 1 {
        outs.extend(ranges.iter().map(|&(a, z)| Some(work(a, z))));
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = ranges
                .iter()
                .skip(1)
                .map(|&(a, z)| s.spawn(move || work(a, z)))
                .collect();
            outs.push(ranges.first().map(|&(a, z)| work(a, z)));
            for h in handles {
                // A worker that panicked is redone below on this thread.
                outs.push(h.join().ok());
            }
        });
    }
    let mut done = Vec::with_capacity(outs.len());
    let mut expected = start;
    for (out, &(_, nominal_end)) in outs.into_iter().zip(&ranges) {
        let out = match out {
            Some(o) if o.start == expected => o,
            _ => work(expected, nominal_end),
        };
        expected = out.end;
        let stop = out.stop;
        done.push(out);
        if stop != Stop::Nominal {
            break;
        }
    }
    done
}

/// Size of one cell in memory.
pub(crate) const CELL_BYTES: usize = std::mem::size_of::<crate::CellValue>();
/// What a text cell costs besides its bytes (allocator rounding and bookkeeping).
pub(crate) const TEXT_OVERHEAD: usize = 16;

/// The memory a fast CSV load holds, checked against [`crate::Limits::max_load_bytes`] and
/// reserved against the process-wide limit (`bicraft-mem`, class `io`) until the load ends.
pub(crate) struct Budget {
    limit: usize,
    held: usize,
    reservation: bicraft_mem::Reservation,
}

impl Budget {
    pub fn new(limits: &crate::Limits) -> Self {
        Self {
            limit: limits.max_load_bytes,
            held: 0,
            reservation: bicraft_mem::global().empty(bicraft_mem::Class::Io),
        }
    }

    /// Sets the bytes the load needs (an estimate before loading, the count while loading);
    /// refused cleanly over the budget or the process limit.
    pub fn need(&mut self, bytes: usize, what: &str) -> Result<(), crate::Error> {
        if bytes > self.limit {
            return Err(crate::Error::LimitExceeded(format!(
                "{what} needs about {} of memory, over the {} load budget",
                bicraft_mem::size_text(bytes),
                bicraft_mem::size_text(self.limit)
            )));
        }
        if bytes > self.held {
            self.reservation
                .grow(bytes - self.held, None, &mut |_| 0)
                .map_err(|e| {
                    crate::Error::LimitExceeded(format!(
                        "{what} needs about {} of memory: {e}",
                        bicraft_mem::size_text(bytes)
                    ))
                })?;
        } else {
            self.reservation.shrink_to(bytes);
        }
        self.held = bytes;
        Ok(())
    }
}

/// Records and bytes per record over (up to) the first `n` records from `from`: the estimate
/// of a table's size before it is loaded. Also the bytes of the fields in `text_cols`.
pub(crate) fn sample(
    text: &str,
    from: usize,
    eof: bool,
    d: &Dialect<'_>,
    keep: usize,
    text_cols: &[bool],
    n: usize,
) -> (usize, usize, usize) {
    let mut rec = Rec::new();
    let mut pos = from;
    let mut records = 0usize;
    let mut text_bytes = 0usize;
    while records < n {
        match scan(text, pos, eof, d, keep, &mut rec) {
            Scan::Record(next) => {
                records += 1;
                for (i, _) in text_cols.iter().enumerate().filter(|(_, t)| **t) {
                    if let Some((s, _)) = rec.field(text, i) {
                        text_bytes += s.len() + TEXT_OVERHEAD;
                    }
                }
                pos = next;
            }
            _ => break,
        }
    }
    (records, pos - from, text_bytes)
}

/// The memory of `rows` rows of `width` cells, with `text_per_row` text bytes per row.
pub(crate) fn table_bytes(rows: usize, width: usize, text_per_row: f64) -> usize {
    let cells = rows.saturating_mul(width).saturating_mul(CELL_BYTES);
    let texts = (rows as f64 * text_per_row).min(usize::MAX as f64) as usize;
    cells.saturating_add(texts)
}

/// The 1-based line of byte `pos`, counting CR LF, lone CR and LF as one line break each (as
/// `read_csv` counts lines).
#[cfg(test)]
pub(crate) fn line_of(text: &str, pos: usize) -> usize {
    let b = text.as_bytes();
    let mut line = 1usize;
    let mut i = 0usize;
    while i < pos.min(b.len()) {
        match b.get(i) {
            Some(b'\r') => {
                line += 1;
                if b.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            Some(b'\n') => line += 1,
            _ => {}
        }
        i += 1;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(q: Quoting) -> Dialect<'static> {
        Dialect {
            quoting: q,
            delim: b",",
            max_cell: 1 << 20,
            max_fields: 4096,
        }
    }

    fn records(text: &str, q: Quoting) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        let mut rec = Rec::new();
        let mut pos = 0;
        loop {
            match scan(text, pos, true, &d(q), usize::MAX, &mut rec) {
                Scan::Record(next) => {
                    out.push(
                        (0..rec.count())
                            .map(|i| rec.field(text, i).unwrap().0.to_string())
                            .collect(),
                    );
                    pos = next;
                }
                Scan::End => return out,
                other => panic!("{other:?} at {pos}"),
            }
        }
    }

    #[test]
    fn m_rules() {
        assert_eq!(
            records("a,\"b\"\"c\"d\n\n\"x\ny\",z", Quoting::MCsv),
            vec![
                vec!["a".to_string(), "b\"cd".into()],
                vec![String::new()],
                vec!["x\ny".into(), "z".into()]
            ]
        );
        assert_eq!(
            records("\"x\ny\",z\r\n", Quoting::MNone),
            vec![vec!["x".to_string()], vec!["y\"".into(), "z".into()]]
        );
        assert_eq!(
            records("a,b,", Quoting::MCsv),
            vec![vec!["a".to_string(), "b".into(), String::new()]]
        );
        assert_eq!(records("\"open", Quoting::MCsv), vec![vec!["open".to_string()]]);
    }

    #[test]
    fn incomplete_at_region_end() {
        let mut rec = Rec::new();
        for q in [Quoting::Rfc, Quoting::MCsv, Quoting::MNone] {
            assert_eq!(scan("a,b\r", 0, false, &d(q), 9, &mut rec), Scan::Incomplete);
            assert_eq!(scan("a,b", 0, false, &d(q), 9, &mut rec), Scan::Incomplete);
            assert_eq!(scan("a,b\n", 0, false, &d(q), 9, &mut rec), Scan::Record(4));
            assert_eq!(scan("\"a\"", 0, false, &d(q), 9, &mut rec), Scan::Incomplete);
        }
        let two = Dialect {
            delim: "§".as_bytes(),
            ..d(Quoting::Rfc)
        };
        let text = "\"a\"\u{a7}";
        let cut = text.get(..3).unwrap();
        assert_eq!(scan(cut, 0, false, &two, 9, &mut rec), Scan::Incomplete);
        assert_eq!(scan(text, 0, true, &two, 9, &mut rec), Scan::Record(5));
    }

    #[test]
    fn rfc_errors() {
        let mut rec = Rec::new();
        assert_eq!(
            scan("\"open", 0, true, &d(Quoting::Rfc), 9, &mut rec),
            Scan::Bad(Problem::Syntax)
        );
        assert_eq!(
            scan("\"a\"b", 0, true, &d(Quoting::Rfc), 9, &mut rec),
            Scan::Bad(Problem::Syntax)
        );
        let tight = Dialect {
            max_cell: 2,
            max_fields: 2,
            ..d(Quoting::Rfc)
        };
        assert_eq!(
            scan("abc", 0, true, &tight, 9, &mut rec),
            Scan::Bad(Problem::CellTooLong)
        );
        assert_eq!(
            scan("a,b,c", 0, true, &tight, 9, &mut rec),
            Scan::Bad(Problem::TooManyFields)
        );
        assert_eq!(line_of("a\r\nb\rc\nd", 7), 4);
    }

    #[test]
    fn parallel_matches_sequential_across_quoted_newlines() {
        // Many quoted fields holding line feeds: chunks often start inside one.
        let mut text = String::new();
        for i in 0..40_000 {
            text.push_str(&format!("{i},\"x\n{i}\ny\",z\n"));
        }
        for q in [Quoting::Rfc, Quoting::MCsv] {
            let dialect = d(q);
            let work = |a: usize, z: usize| {
                parse_chunk(
                    &text,
                    a,
                    z,
                    true,
                    &dialect,
                    9,
                    || 0usize,
                    |n, _| {
                        *n += 1;
                        true
                    },
                )
            };
            let outs = run_region(&text, 0, 8, &work);
            assert!(outs.len() > 1);
            assert_eq!(outs.iter().map(|o| o.out).sum::<usize>(), 40_000);
            assert_eq!(outs.last().unwrap().stop, Stop::End);
        }
    }
}
