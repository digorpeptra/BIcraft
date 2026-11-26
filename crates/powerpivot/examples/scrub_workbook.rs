//! Fixture aid: replaces machine-specific text in a workbook Excel saved, keeping every offset.
//!
//! ```text
//! cargo run -p bicraft-powerpivot --example scrub_workbook -- <in.xlsx> <out.xlsx> <from>=<to>...
//! ```
//!
//! Excel records the computer name in the `PARTITIONS` file of its data model backup
//! (`xl/model/item.data`). Each `<from>` is replaced by `<to>` (same length) as UTF-16LE text in
//! the backup's uncompressed files, whose CRC end markers are then written again. The tool
//! refuses to write when a `<from>` (ASCII case-insensitive, UTF-16LE or UTF-8) is still found
//! in any part of the package or in any decompressed file of the backup, or when a CRC marker of
//! the result does not match. It prints counts only, never the replaced text.

use std::process::ExitCode;

use bicraft_abf::Abf;
use bicraft_powerpivot::{ITEM_DATA, synthetic::crc_marker};

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Whether `needle` occurs in `hay`, ASCII case-insensitively.
fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

fn found(hay: &[u8], from: &str) -> bool {
    contains_ci(hay, from.as_bytes()) || contains_ci(hay, &utf16(from))
}

fn replace_all(hay: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while let Some(w) = hay.get(i..i + from.len()) {
        if w.eq_ignore_ascii_case(from) {
            if let Some(dst) = hay.get_mut(i..i + from.len()) {
                dst.copy_from_slice(to);
            }
            n += 1;
            i += from.len();
        } else {
            i += 1;
        }
    }
    n
}

/// The decompressed content of every file of the backup (the stored bytes for the others).
fn contents(abf: &Abf<'_>) -> Result<Vec<Vec<u8>>, String> {
    let crc = abf.header().crc_markers;
    abf.files()
        .iter()
        .map(|f| {
            let stored = abf.stored(f).ok_or("a file lies outside the backup")?;
            if !f.compressed {
                return Ok(stored.to_vec());
            }
            let body = if crc {
                stored.get(..stored.len().saturating_sub(4)).unwrap_or_default()
            } else {
                stored
            };
            bicraft_xpress8::decompress_chunked(body, Some(f.size as u64), 1 << 30).map_err(|e| e.to_string())
        })
        .collect()
}

fn run(args: &[String]) -> Result<(), String> {
    let [input, output, pairs @ ..] = args else {
        return Err("usage: scrub_workbook <in.xlsx> <out.xlsx> <from>=<to>...".into());
    };
    let mut swaps = Vec::new();
    for p in pairs {
        let (from, to) = p.split_once('=').ok_or("each replacement is <from>=<to>")?;
        if from.is_empty() || from.encode_utf16().count() != to.encode_utf16().count() {
            return Err("<from> and <to> must be non-empty and of the same length".into());
        }
        swaps.push((from.to_owned(), to.to_owned()));
    }
    let bytes = std::fs::read(input).map_err(|e| format!("cannot read the input: {e}"))?;
    let mut package = bicraft_opc::Package::read_bytes(&bytes).map_err(|e| e.to_string())?;
    let item = package
        .require(ITEM_DATA)
        .map_err(|e| e.to_string())?
        .data
        .clone();
    let abf = Abf::parse(&item).map_err(|e| e.to_string())?;
    if !abf.header().crc_markers {
        return Err("the backup has no CRC markers; this tool expects a workbook Excel saved".into());
    }
    let mut out = item.clone();
    let mut replaced = 0;
    for f in abf.files().iter().filter(|f| !f.compressed) {
        let end = f
            .offset
            .checked_add(f.stored_size)
            .ok_or("file range overflows")?;
        let body_end = end.checked_sub(4).ok_or("file shorter than its CRC marker")?;
        let body = out
            .get_mut(f.offset..body_end)
            .ok_or("a file lies outside the backup")?;
        let before = replaced;
        for (from, to) in &swaps {
            replaced += replace_all(body, &utf16(from), &utf16(to));
        }
        if replaced != before {
            let marker = crc_marker(out.get(f.offset..body_end).unwrap_or_default());
            out.get_mut(body_end..end)
                .ok_or("a file lies outside the backup")?
                .copy_from_slice(&marker);
        }
    }

    let check = Abf::parse(&out).map_err(|e| e.to_string())?;
    for f in check.files() {
        let stored = check.stored(f).ok_or("a file lies outside the backup")?;
        let (body, tail) = stored.split_at(stored.len().saturating_sub(4));
        if crc_marker(body) != tail {
            return Err("a CRC marker does not match after the replacement".into());
        }
    }
    let files = contents(&check)?;
    for (from, _) in &swaps {
        if files.iter().any(|c| found(c, from)) || found(&out, from) {
            return Err("a replaced text is still in the backup (in a compressed file?)".into());
        }
        if package
            .parts
            .iter()
            .any(|p| p.name != ITEM_DATA && found(&p.data, from))
        {
            return Err("a replaced text is still in another part of the package".into());
        }
    }
    let stored = package.require(ITEM_DATA).map_err(|e| e.to_string())?.stored;
    package.put(ITEM_DATA, out, stored);
    let written = package.to_bytes().map_err(|e| e.to_string())?;
    std::fs::write(output, written).map_err(|e| format!("cannot write the output: {e}"))?;
    println!(
        "{replaced} replacement(s) in {} backup file(s) checked",
        files.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("scrub_workbook: {e}");
            ExitCode::FAILURE
        }
    }
}
