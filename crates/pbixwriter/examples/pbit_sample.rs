//! Writes a sample PBIT for a manual check in Power BI Desktop.
//!
//! ```text
//! pbit_sample <model.bim> <Name.Report dir> <out.pbit>   # model + PBIR documents of a PBIP
//! pbit_sample --rebuild <in.pbit> <out.pbit>             # re-emit a PBIT through the writer
//! ```
//!
//! Report documents are taken verbatim from `definition/` and `StaticResources/` (until the PBIR
//! reader lands they cannot go through `bicraft_report::Report`).

use std::error::Error;
use std::path::{Path, PathBuf};

use bicraft_opc::{Package, decode_text};
use bicraft_pbixwriter::{check_package, part, write_pbit_documents};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (model, docs, out) = match args.as_slice() {
        [flag, input, out] if flag == "--rebuild" => {
            let pkg = Package::open(Path::new(input))?;
            let schema = pkg.require(part::DATA_MODEL_SCHEMA)?;
            let model = bicraft_tmsl::read(&decode_text(&schema.name, &schema.data)?.0)?;
            let docs = pkg
                .parts
                .iter()
                .filter_map(|p| {
                    p.name
                        .strip_prefix(part::REPORT_PREFIX)
                        .filter(|r| r.starts_with("definition/") || r.starts_with("StaticResources/"))
                        .map(|r| (r.to_string(), p.data.clone()))
                })
                .collect();
            (model, docs, out)
        }
        [bim, report_dir, out] => {
            let bytes = std::fs::read(bim)?;
            let model = bicraft_tmsl::read(&decode_text(bim, &bytes)?.0)?;
            let mut docs = Vec::new();
            for sub in ["definition", "StaticResources"] {
                collect(Path::new(report_dir), &PathBuf::from(sub), &mut docs)?;
            }
            docs.sort();
            (model, docs, out)
        }
        _ => {
            eprintln!(
                "usage: pbit_sample <model.bim> <Name.Report dir> <out.pbit>\n       pbit_sample --rebuild <in.pbit> <out.pbit>"
            );
            std::process::exit(2);
        }
    };
    let pkg = write_pbit_documents(&model, &docs)?;
    let bytes = pkg.to_bytes()?;
    let reread = Package::read_bytes(&bytes)?;
    let violations = check_package(&reread);
    if !violations.is_empty() {
        return Err(format!("{} loader-rule violations after re-reading", violations.len()).into());
    }
    std::fs::write(out, &bytes)?;
    println!("wrote {out}: {} parts, {} bytes", pkg.parts.len(), bytes.len());
    Ok(())
}

/// Collects files under `root/rel` as `(rel path with '/', bytes)`.
fn collect(root: &Path, rel: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), Box<dyn Error>> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let rel = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            collect(root, &rel, out)?;
        } else {
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((name, std::fs::read(entry.path())?));
        }
    }
    Ok(())
}
