//! Dev aid: re-pack a decompressed ABF, optionally replacing files.
//!
//! ```text
//! cargo run -p bicraft-abf --example abf_repack -- <in.abf> <out.abf> [<path> <replacement-file>]...
//! ```
//!
//! `<path>` is resolved like `Abf::entry` (e.g. `metadata.sqlitedb`). Prints counts only.

use std::process::ExitCode;

use bicraft_abf::Abf;

fn run(args: &[String]) -> Result<(), String> {
    let [input, output, pairs @ ..] = args else {
        return Err("usage: abf_repack <in.abf> <out.abf> [<path> <replacement-file>]...".into());
    };
    if pairs.len() % 2 != 0 {
        return Err("replacements come in <path> <file> pairs".into());
    }
    let bytes = std::fs::read(input).map_err(|e| format!("cannot read the input: {e}"))?;
    let abf = Abf::parse(&bytes).map_err(|e| e.to_string())?;
    let mut writer = abf.to_writer().map_err(|e| e.to_string())?;
    for pair in pairs.chunks(2) {
        if let [path, file] = pair {
            let data = std::fs::read(file).map_err(|e| format!("cannot read a replacement: {e}"))?;
            writer.replace(path, data).map_err(|e| e.to_string())?;
        }
    }
    let out = writer.finish().map_err(|e| e.to_string())?;
    std::fs::write(output, &out).map_err(|e| format!("cannot write the output: {e}"))?;
    println!(
        "{} files, {} -> {} bytes, {}",
        writer.len(),
        bytes.len(),
        out.len(),
        if out == bytes { "identical" } else { "changed" }
    );
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
