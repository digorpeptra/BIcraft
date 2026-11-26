//! Archive writers of `cargo xtask dist`: zip (Windows) and ustar + gzip (Linux).
//!
//! Both are byte-stable: entries in the given order, fixed timestamps (1980-01-01, the zip
//! epoch), no owner names, so two packagings of the same files give the same archive.

use std::io::Write;

/// One file of an archive.
pub struct Entry {
    /// Path inside the archive, `/`-separated, without the top folder.
    pub path: String,
    pub data: Vec<u8>,
    /// Executable (mode 755 instead of 644).
    pub exec: bool,
}

/// Fixed modification time of every entry: 1980-01-01T00:00:00Z.
const MTIME: u64 = 315_532_800;

/// Folders to create before `entries`, parents first, each once (`top/`, `top/docs/`, …).
fn folders(top: &str, entries: &[Entry]) -> Vec<String> {
    let mut out = vec![format!("{top}/")];
    for e in entries {
        let mut acc = format!("{top}/");
        let parts: Vec<&str> = e.path.split('/').collect();
        for p in parts.iter().take(parts.len().saturating_sub(1)) {
            acc.push_str(p);
            acc.push('/');
            if !out.contains(&acc) {
                out.push(acc.clone());
            }
        }
    }
    out
}

/// A zip with everything under `top/`.
pub fn zip(top: &str, entries: &[Entry]) -> Result<Vec<u8>, String> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let base = SimpleFileOptions::default()
        .last_modified_time(zip::DateTime::default())
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9))
        // zip >= 9 records the host system by default; the modes below are Unix modes.
        .system(zip::System::Unix);
    let err = |e: zip::result::ZipError| format!("zip: {e}");
    for d in folders(top, entries) {
        w.add_directory(d, base.unix_permissions(0o755)).map_err(err)?;
    }
    for e in entries {
        let mode = if e.exec { 0o755 } else { 0o644 };
        w.start_file(format!("{top}/{}", e.path), base.unix_permissions(mode))
            .map_err(err)?;
        w.write_all(&e.data).map_err(|e| format!("zip: {e}"))?;
    }
    let cursor = w.finish().map_err(err)?;
    Ok(cursor.into_inner())
}

/// A gzip-compressed ustar archive with everything under `top/`.
pub fn tar_gz(top: &str, entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    for d in folders(top, entries) {
        tar.extend_from_slice(&header(&d, 0, 0o755, b'5')?);
    }
    for e in entries {
        let mode = if e.exec { 0o755 } else { 0o644 };
        let path = format!("{top}/{}", e.path);
        tar.extend_from_slice(&header(&path, e.data.len() as u64, mode, b'0')?);
        tar.extend_from_slice(&e.data);
        let pad = (512 - e.data.len() % 512) % 512;
        tar.resize(tar.len() + pad, 0);
    }
    // End of archive: two zero blocks.
    tar.resize(tar.len() + 1024, 0);
    let mut gz = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::best());
    gz.write_all(&tar).map_err(|e| format!("gzip: {e}"))?;
    gz.finish().map_err(|e| format!("gzip: {e}"))
}

/// Writes `value` as zero-padded octal filling `field` but its last byte (a NUL).
fn octal(field: &mut [u8], value: u64) -> Result<(), String> {
    let width = field.len().saturating_sub(1);
    let s = format!("{value:0width$o}");
    if s.len() > width {
        return Err(format!("tar: value {value} does not fit {width} octal digits"));
    }
    field[..width].copy_from_slice(s.as_bytes());
    field[width] = 0;
    Ok(())
}

/// A ustar header block. Paths over 100 bytes are split into `prefix` and `name` at a `/`.
fn header(path: &str, size: u64, mode: u32, kind: u8) -> Result<[u8; 512], String> {
    let mut h = [0u8; 512];
    let bytes = path.as_bytes();
    let (prefix, name) = if bytes.len() <= 100 {
        (&b""[..], bytes)
    } else {
        // The split `/` belongs to neither part; skip a trailing one (folders).
        let body = bytes.strip_suffix(b"/").unwrap_or(bytes);
        let at = body
            .iter()
            .enumerate()
            .rev()
            .find(|&(i, &b)| b == b'/' && i <= 155 && bytes.len() - i - 1 <= 100)
            .map(|(i, _)| i)
            .ok_or_else(|| format!("tar: path too long for ustar: {path}"))?;
        (&bytes[..at], &bytes[at + 1..])
    };
    h[..name.len()].copy_from_slice(name);
    octal(&mut h[100..108], u64::from(mode))?;
    octal(&mut h[108..116], 0)?;
    octal(&mut h[116..124], 0)?;
    octal(&mut h[124..136], size)?;
    octal(&mut h[136..148], MTIME)?;
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    // Checksum: sum of the header bytes with the checksum field read as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn sample() -> Vec<Entry> {
        vec![
            Entry {
                path: "bicraft".into(),
                data: b"#!/bin/sh\necho hi\n".to_vec(),
                exec: true,
            },
            Entry {
                path: "docs/mcp.md".into(),
                data: vec![b'x'; 1000],
                exec: false,
            },
        ]
    }

    #[test]
    fn folders_parents_first_once() {
        assert_eq!(folders("top", &sample()), ["top/", "top/docs/"]);
    }

    #[test]
    fn tar_layout_and_checksum() {
        let gz = tar_gz("top", &sample()).unwrap();
        let mut tar = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut tar)
            .unwrap();
        // 2 folders + header + 1 block + header + 2 blocks + 2 end blocks.
        assert_eq!(tar.len(), 512 * 9);
        let h = &tar[512 * 2..512 * 3];
        assert_eq!(&h[..11], b"top/bicraft");
        assert_eq!(&h[100..108], b"0000755\0");
        assert_eq!(&h[124..136], b"00000000022\0");
        assert_eq!(&h[257..263], b"ustar\0");
        let mut copy = h.to_vec();
        copy[148..156].copy_from_slice(b"        ");
        let sum: u32 = copy.iter().map(|&b| u32::from(b)).sum();
        let stored = std::str::from_utf8(&h[148..154]).unwrap();
        assert_eq!(u32::from_str_radix(stored, 8).unwrap(), sum);
        assert_eq!(&tar[512 * 3..512 * 3 + 9], b"#!/bin/sh");
        assert!(tar[512 * 7..].iter().all(|&b| b == 0));
        // Byte-stable.
        assert_eq!(gz, tar_gz("top", &sample()).unwrap());
    }

    #[test]
    fn long_paths_use_the_prefix() {
        let long = format!("{}/{}", "d".repeat(120), "f".repeat(90));
        let h = header(&long, 0, 0o644, b'0').unwrap();
        assert_eq!(&h[..90], "f".repeat(90).as_bytes());
        assert_eq!(&h[345..465], "d".repeat(120).as_bytes());
        assert!(header(&"x".repeat(300), 0, 0o644, b'0').is_err());
    }

    #[test]
    fn zip_round_trip() {
        let bytes = zip("top", &sample()).unwrap();
        {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut h);
            eprintln!("DISTHASH {:x} {}", h.finish(), bytes.len());
        }
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let names: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
        assert!(names.contains(&"top/docs/mcp.md".to_owned()));
        let mut f = z.by_name("top/bicraft").unwrap();
        assert_eq!(f.unix_mode().map(|m| m & 0o777), Some(0o755));
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "#!/bin/sh\necho hi\n");
        drop(f);
        assert_eq!(bytes, zip("top", &sample()).unwrap());
    }
}
