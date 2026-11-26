//! The benchmark cases: in-process (the crates, as the engine calls them) and end to end through
//! the `bicraft` binary. Inputs are generated into the scratch folder first.

use std::path::{Path, PathBuf};

use bicraft_engine::Session;
use bicraft_model::{Measure, SemanticModel};
use bicraft_opc::Package;
use serde_json::{Value, json};

use super::synth::{self, ModelShape};
use super::{Bencher, Opts, Work, cli, copy_dir, mcp_session, write_file};

/// A synthetic `metadata.sqlitedb` (our own, see `crates/metadata/tests/gen_fixture.py`).
const SYNTHETIC_METADATA: &[u8] =
    include_bytes!("../../../crates/metadata/tests/fixtures/synthetic_metadata.sqlitedb");

fn exec(s: &mut Session, id: &str, params: Value) -> Result<Value, String> {
    s.execute(id, &params).map_err(|e| format!("{id}: {e}"))
}

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("{}: not UTF-8", p.display()))
}

fn measure_count(m: &SemanticModel) -> usize {
    m.tables.iter().map(|t| t.measures.len()).sum()
}

/// Writes `model.bim` for `shape`; returns (path, text).
fn write_bim(dir: &Path, name: &str, shape: &ModelShape) -> Result<(PathBuf, String), String> {
    let text = serde_json::to_string_pretty(&synth::tmsl_model(shape)).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.bim"));
    write_file(&path, text.as_bytes())?;
    Ok((path, text))
}

/// Every file under `dir` as (relative `/` path, bytes), like a PBIR reader sees them.
fn collect(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out)?;
            } else {
                let rel = p.strip_prefix(root).map_err(|e| e.to_string())?;
                let rel: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push((rel.join("/"), std::fs::read(&p).map_err(|e| e.to_string())?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// The fields of visual `v` on a report over a `ModelShape::grid(10, 20, 10)` model.
fn visual_spec(page: &str, v: usize) -> Value {
    let t = v % 10;
    let table = synth::table_name(t);
    let measure = synth::measure_name(t + 10 * (v % 10));
    let col = |c: usize| format!("'{table}'[{}]", synth::column_name(c));
    let (kind, fields) = match v % 5 {
        0 => (
            "clusteredColumnChart",
            json!([{"role": "Category", "field": col(2)}, {"role": "Y", "field": format!("[{measure}]")}]),
        ),
        1 => (
            "tableEx",
            json!([{"role": "Values", "field": col(1)}, {"role": "Values", "field": col(2)},
                                 {"role": "Values", "field": format!("[{measure}]")}]),
        ),
        2 => (
            "card",
            json!([{"role": "Values", "field": format!("[{measure}]")}]),
        ),
        3 => (
            "lineChart",
            json!([{"role": "Category", "field": col(3)}, {"role": "Y", "field": format!("Sum({})", col(0))}]),
        ),
        _ => ("slicer", json!([{"role": "Values", "field": col(7)}])),
    };
    json!({"page": page, "visual_type": kind, "x": (v % 5) * 256, "y": (v / 5) * 180,
           "width": 250, "height": 170, "fields": fields})
}

/// Builds a report of `pages` × 20 visuals through engine commands on the model at `bim`.
/// Returns the session and the number of commands run.
fn build_report(bim: &Path, pages: usize) -> Result<(Session, usize), String> {
    let mut s = Session::new();
    exec(&mut s, "file.open", json!({"path": bim}))?;
    exec(&mut s, "report.new", json!({}))?;
    let blank = s
        .report
        .as_ref()
        .and_then(|r| r.pages.first())
        .map(|p| p.name.clone())
        .ok_or("report.new made no page")?;
    let mut n = 2;
    for p in 0..pages {
        let name = format!("Bench page {p}");
        exec(&mut s, "page.add", json!({"display_name": name}))?;
        n += 1;
        for v in 0..20 {
            exec(&mut s, "visual.add", visual_spec(&name, v))?;
            n += 1;
        }
    }
    exec(&mut s, "page.remove", json!({"page": blank}))?;
    Ok((s, n + 1))
}

pub fn run(b: &mut Bencher, opts: &Opts, bin: &Path, scratch: &Path) {
    let quick = opts.quick;
    if !opts.cli_only {
        dax(b, scratch);
        model_files(b, scratch, quick);
    }
    reports(b, opts, bin, scratch);
    projects(b, opts, bin, scratch);
    if !opts.cli_only {
        xpress9(b, quick);
    }
    pbix(b, opts, bin, scratch);
    if !opts.cli_only {
        sqlite(b, quick);
        vertipaq(b, quick);
    }
    if opts.real {
        real(b, opts, bin);
    }
}

fn fail(b: &mut Bencher, name: &str, e: String) {
    b.case(name, 1, || Err(e.clone()));
}

// ---------------------------------------------------------------------------------------------

fn dax(b: &mut Bencher, scratch: &Path) {
    use bicraft_dax::{Context, ExprKind, validate, validate_as};
    b.group("DAX validation");
    let shape = ModelShape::grid(100, 20, 10);
    let model = match bicraft_tmsl::read(&synth::tmsl_model(&shape).to_string()) {
        Ok(m) => m,
        Err(e) => return fail(b, "setup", e.to_string()),
    };
    let expr = synth::measure_expression(&shape, 3);
    b.case("dax.validate, 1 expression, no model", 5000, || {
        let v = validate(&expr, &Context::default());
        Ok(Work::items(1, "expr").note(format!("valid={}", v.valid)))
    });
    let ctx = Context {
        model: Some(&model),
        table: Some("Table 3"),
    };
    b.case(
        "dax.validate, 1 expression, model 100 tables / 1,000 measures",
        5000,
        || {
            let v = validate_as(&expr, &ctx, ExprKind::Measure);
            Ok(Work::items(1, "expr").note(format!("valid={}", v.valid)))
        },
    );
    let corpus = synth::dax_corpus(&shape, 10_000);
    let bytes: u64 = corpus.iter().map(|e| e.len() as u64).sum();
    b.case("dax.validate, 10,000 expressions, same model", 10, || {
        let mut invalid = 0;
        for (i, e) in corpus.iter().enumerate() {
            let table = synth::table_name(i % 100);
            let c = Context {
                model: Some(&model),
                table: Some(&table),
            };
            if !validate_as(e, &c, ExprKind::Measure).valid {
                invalid += 1;
            }
        }
        Ok(Work::items(10_000, "expr")
            .with_bytes(bytes)
            .note(format!("invalid={invalid}")))
    });

    // The 10,000-measure case runs in full runs only (it took 10+ s before gap G5 was fixed)
    let counts: &[usize] = if b.quick {
        &[100, 1_000]
    } else {
        &[100, 1_000, 10_000]
    };
    for &n in counts {
        let shape = ModelShape {
            tables: (n / 100).max(10),
            columns: 20,
            measures: n,
            chain: 100,
            cycle_every: n / 2,
        };
        let name = format!("model.validate, {n} measures (chains of 100, 2 cycles)");
        if !b.wants(&name) {
            continue;
        }
        let model = match bicraft_tmsl::read(&synth::tmsl_model(&shape).to_string()) {
            Ok(m) => m,
            Err(e) => return fail(b, &name, e.to_string()),
        };
        let mut s = Session::new();
        s.model = Some(model);
        b.case(&name, 20, || {
            let v = exec(&mut s, "model.validate", json!({}))?;
            let cycles = v["cycles"].as_array().map_or(0, Vec::len);
            Ok(Work::items(n as u64, "measure")
                .note(format!("invalid={}, cycles={cycles}", v["invalid_count"])))
        });
        // the same through the CLI is in `projects` (validate-model)
        let _ = write_bim(scratch, &format!("validate{n}"), &shape);
    }
}

// ---------------------------------------------------------------------------------------------

fn model_files(b: &mut Bencher, scratch: &Path, quick: bool) {
    b.group("Model files (TMSL / TMDL), tables × 20 columns × 10 measures");
    let sizes: &[usize] = if quick { &[10, 100] } else { &[10, 100, 1_000] };
    for &tables in sizes {
        if !b.wants(&format!("{tables} tables")) {
            continue;
        }
        let shape = ModelShape::grid(tables, 20, 10);
        let text = synth::tmsl_model(&shape).to_string();
        let bytes = text.len() as u64;
        let runs = if tables >= 1_000 { 5 } else { 20 };
        let model = match bicraft_tmsl::read(&text) {
            Ok(m) => m,
            Err(e) => return fail(b, "setup", e.to_string()),
        };
        let note = format!("{} KB TMSL", bytes / 1024);
        b.case(&format!("tmsl::read, {tables} tables"), runs, || {
            let m = bicraft_tmsl::read(&text).map_err(|e| e.to_string())?;
            Ok(Work::items(m.tables.len() as u64, "table")
                .with_bytes(bytes)
                .note(note.clone()))
        });
        b.case(&format!("tmsl::write, {tables} tables"), runs, || {
            let out = bicraft_tmsl::write(&model);
            Ok(Work::items(tables as u64, "table").with_bytes(out.len() as u64))
        });
        let docs = bicraft_tmdl::write_documents(&model);
        let tmdl_bytes: u64 = docs.iter().map(|(_, t)| t.len() as u64).sum();
        b.case(&format!("tmdl::write_documents, {tables} tables"), runs, || {
            let d = bicraft_tmdl::write_documents(&model);
            Ok(Work::items(d.len() as u64, "file").with_bytes(tmdl_bytes))
        });
        b.case(&format!("tmdl::read_documents, {tables} tables"), runs, || {
            let m = bicraft_tmdl::read_documents(&docs).map_err(|e| e.to_string())?;
            Ok(Work::items(measure_count(&m) as u64, "measure").with_bytes(tmdl_bytes))
        });
        let dir = scratch.join(format!("tmdl{tables}"));
        b.case(
            &format!("tmdl::write_folder (disk), {tables} tables"),
            runs,
            || {
                bicraft_tmdl::write_folder(&model, &dir).map_err(|e| e.to_string())?;
                Ok(Work::items(docs.len() as u64, "file").with_bytes(tmdl_bytes))
            },
        );
        b.case(
            &format!("tmdl::read_folder (disk), {tables} tables"),
            runs,
            || {
                let m = bicraft_tmdl::read_folder(&dir).map_err(|e| e.to_string())?;
                Ok(Work::items(m.tables.len() as u64, "table").with_bytes(tmdl_bytes))
            },
        );
    }
}

// ---------------------------------------------------------------------------------------------

fn reports(b: &mut Bencher, opts: &Opts, bin: &Path, scratch: &Path) {
    b.group("Reports (PBIR), pages × 20 visuals, model 10 tables / 100 measures");
    let (bim, _) = match write_bim(scratch, "report-model", &ModelShape::grid(10, 20, 10)) {
        Ok(x) => x,
        Err(e) => return fail(b, "setup", e),
    };
    let sizes: &[usize] = if opts.quick { &[1, 10] } else { &[1, 10, 100] };
    for &pages in sizes {
        if !b.wants(&format!("{pages} pages")) {
            continue;
        }
        let visuals = (pages * 20) as u64;
        let runs = if pages >= 100 { 5 } else { 20 };
        if !opts.cli_only {
            b.case(
                &format!("build via page.add/visual.add commands, {pages} pages"),
                runs,
                || {
                    let (_, n) = build_report(&bim, pages)?;
                    Ok(Work::items(n as u64, "cmd"))
                },
            );
        }
        // the project every later case reads
        let dir = scratch.join(format!("report{pages}"));
        let built = build_report(&bim, pages).and_then(|(mut s, _)| {
            exec(&mut s, "project.new", json!({"dir": dir, "name": "Bench"}))?;
            Ok(s)
        });
        let mut session = match built {
            Ok(s) => s,
            Err(e) => return fail(b, &format!("setup {pages} pages"), e),
        };
        let pbip = dir.join("Bench.pbip");
        let report_dir = dir.join("Bench.Report");
        let docs = match collect(&report_dir) {
            Ok(d) => d,
            Err(e) => return fail(b, "setup", e),
        };
        let bytes: u64 = docs.iter().map(|(_, d)| d.len() as u64).sum();
        let note = format!("{} files, {} KB", docs.len(), bytes / 1024);
        if !opts.cli_only {
            let report = session.report.clone().unwrap_or_default();
            b.case(&format!("pbir::read_dir, {pages} pages"), runs, || {
                let r = bicraft_pbir::read_dir(&report_dir).map_err(|e| e.to_string())?;
                let v: usize = r.pages.iter().map(|p| p.visuals.len()).sum();
                Ok(Work::items(v as u64, "visual")
                    .with_bytes(bytes)
                    .note(note.clone()))
            });
            b.case(&format!("pbir::write_documents, {pages} pages"), runs, || {
                let d = bicraft_pbir::write_documents(&report);
                Ok(Work::items(d.len() as u64, "file").with_bytes(bytes))
            });
            let out = scratch.join(format!("pbir-out{pages}"));
            b.case(&format!("pbir::write_dir (disk), {pages} pages"), runs, || {
                bicraft_pbir::write_dir(&report, &out).map_err(|e| e.to_string())?;
                Ok(Work::items(visuals, "visual").with_bytes(bytes))
            });
            b.case(
                &format!("pbir::validate_documents (schemas), {pages} pages"),
                runs,
                || {
                    let issues = bicraft_pbir::validate_documents(&docs);
                    Ok(Work::items(docs.len() as u64, "file")
                        .with_bytes(bytes)
                        .note(format!("issues={}", issues.len())))
                },
            );
            let mut s = Session::new();
            let opened = exec(&mut s, "file.open", json!({"path": pbip}));
            match opened {
                Ok(_) => b.case(
                    &format!("report.validate (schemas + model fields), {pages} pages"),
                    runs,
                    || {
                        let v = exec(&mut s, "report.validate", json!({}))?;
                        Ok(Work::items(visuals, "visual").note(format!("errors={}", v["error_count"])))
                    },
                ),
                Err(e) => fail(b, "report.validate", e),
            }
            b.case(
                &format!("file.open (PBIP: TMDL + PBIR), {pages} pages"),
                runs,
                || {
                    let mut s = Session::new();
                    exec(&mut s, "file.open", json!({"path": pbip}))?;
                    Ok(Work::items(visuals, "visual"))
                },
            );
            b.case(
                &format!("file.save after a visual move (PBIR rewrite), {pages} pages"),
                runs,
                || {
                    let page = session
                        .report
                        .as_ref()
                        .and_then(|r| r.pages.first())
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    let visual = session
                        .report
                        .as_ref()
                        .and_then(|r| r.pages.first())
                        .and_then(|p| p.visuals.first())
                        .map(|v| v.name.clone())
                        .unwrap_or_default();
                    exec(
                        &mut session,
                        "visual.set_position",
                        json!({"page": page, "visual": visual, "x": 10}),
                    )?;
                    exec(&mut session, "file.save", json!({}))?;
                    Ok(Work::items(visuals, "visual"))
                },
            );
        }
        let p = pbip.to_string_lossy().into_owned();
        b.case(&format!("CLI validate-report, {pages} pages"), runs, || {
            cli(bin, &["validate-report", &p], &[0, 1])?;
            Ok(Work::items(visuals, "visual"))
        });
    }
}

// ---------------------------------------------------------------------------------------------

fn projects(b: &mut Bencher, opts: &Opts, bin: &Path, scratch: &Path) {
    b.group("Process start and projects (end to end)");
    b.case("CLI bicraft --version (process start)", 40, || {
        cli(bin, &["--version"], &[0])?;
        Ok(Work::items(1, "process"))
    });
    b.case("CLI validate-dax, 1 expression, no model", 40, || {
        cli(
            bin,
            &["validate-dax", "CALCULATE(SUM(Sales[Amount]), ALL(Sales))"],
            &[0],
        )?;
        Ok(Work::items(1, "process"))
    });
    let counts: &[usize] = if b.quick { &[1_000] } else { &[1_000, 10_000] };
    for &n in counts {
        let bim = scratch.join(format!("validate{n}.bim"));
        let name = format!("CLI validate-model, {n} measures");
        if !bim.is_file() {
            // only written when the in-process case ran (filters, --cli-only)
            let shape = ModelShape {
                tables: (n / 100).max(10),
                columns: 20,
                measures: n,
                chain: 100,
                cycle_every: n / 2,
            };
            if let Err(e) = write_bim(scratch, &format!("validate{n}"), &shape) {
                fail(b, &name, e);
                continue;
            }
        }
        let p = bim.to_string_lossy().into_owned();
        b.case(&name, 10, || {
            cli(bin, &["validate-model", &p], &[0, 1])?;
            Ok(Work::items(n as u64, "measure"))
        });
    }

    let (bim, _) = match write_bim(scratch, "project-model", &ModelShape::grid(100, 20, 10)) {
        Ok(x) => x,
        Err(e) => return fail(b, "setup", e),
    };
    let mut k = 0usize;
    if !opts.cli_only {
        b.case("project.new from model.bim, 100 tables (in-process)", 10, || {
            k += 1;
            let mut s = Session::new();
            exec(&mut s, "file.open", json!({"path": bim}))?;
            exec(
                &mut s,
                "project.new",
                json!({"dir": scratch.join(format!("pn{k}")), "name": "Bench"}),
            )?;
            Ok(Work::items(100, "table"))
        });
    }
    let bim_s = bim.to_string_lossy().into_owned();
    b.case("CLI new-project --open model.bim, 100 tables", 10, || {
        k += 1;
        let dir = scratch.join(format!("pn{k}"));
        cli(
            bin,
            &["new-project", &dir.to_string_lossy(), "Bench", "--open", &bim_s],
            &[0],
        )?;
        Ok(Work::items(100, "table"))
    });

    // Edit loop on a 10-table project with a 10-page report.
    let base = scratch.join("report10").join("Bench.pbip");
    if !base.is_file() {
        return b.skip("edit loop", "needs the 10-page report project");
    }
    let base_dir = scratch.join("report10");
    let fresh = |tag: &str| -> Result<PathBuf, String> {
        let d = scratch.join(format!("loop-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        copy_dir(&base_dir, &d)?;
        Ok(d.join("Bench.pbip"))
    };
    let add = |i: usize| json!({"table": "Table 0", "name": format!("Loop {i}"), "expression": format!("SUM('Table 0'[Col 0]) + {i}")});
    let run_id = std::cell::Cell::new(0usize);
    // a fresh copy of the project for every run, made outside the timed part
    let copy = |tag: &str| {
        run_id.set(run_id.get() + 1);
        fresh(&format!("{tag}{}", run_id.get()))
    };
    if !opts.cli_only {
        b.case_with_setup(
            "edit loop ×100 (measure.add → file.save → report.validate), in-process",
            3,
            || copy("in"),
            |p| {
                let mut s = Session::new();
                exec(&mut s, "file.open", json!({"path": p}))?;
                for i in 0..100 {
                    exec(&mut s, "measure.add", add(i))?;
                    exec(&mut s, "file.save", json!({}))?;
                    exec(&mut s, "report.validate", json!({}))?;
                }
                Ok(Work::items(100, "iteration"))
            },
        );
    }
    b.case_with_setup(
        "edit loop ×100, one `bicraft run` batch (300 commands)",
        3,
        || copy("batch"),
        |p| {
            let mut args: Vec<String> = vec!["run".into(), "--open".into(), path_str(&p)?.into()];
            for i in 0..100 {
                args.push("--cmd".into());
                args.push(format!("measure.add={}", add(i)));
                args.push("--cmd".into());
                args.push("file.save".into());
                args.push("--cmd".into());
                args.push("report.validate".into());
            }
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            cli(bin, &refs, &[0])?;
            Ok(Work::items(100, "iteration"))
        },
    );
    b.case_with_setup(
        "edit loop ×100, 100 separate `bicraft run` processes",
        3,
        || copy("sep"),
        |p| {
            let ps = path_str(&p)?.to_string();
            for i in 0..100 {
                let cmd = format!("measure.add={}", add(i));
                cli(
                    bin,
                    &[
                        "run",
                        "--open",
                        &ps,
                        "--cmd",
                        &cmd,
                        "--cmd",
                        "file.save",
                        "--cmd",
                        "report.validate",
                    ],
                    &[0],
                )?;
            }
            Ok(Work::items(100, "iteration"))
        },
    );

    b.group("MCP (stdio, one session)");
    b.case(
        "100 validate_dax calls, no model (process start included)",
        10,
        || {
            let calls: Vec<(String, Value)> = (0..100)
                .map(|i| {
                    (
                        "validate_dax".to_string(),
                        json!({"expression": format!("CALCULATE(SUM(Sales[Amount]) + {i}, ALL(Sales))")}),
                    )
                })
                .collect();
            let errors = mcp_session(bin, &calls)?;
            if errors > 0 {
                return Err(format!("{errors} tool calls failed"));
            }
            Ok(Work::items(100, "call"))
        },
    );
    b.case_with_setup(
        "100 tool calls on the 10-page project (open_file, 50 add_measure, 49 validate_dax)",
        5,
        || copy("mcp"),
        |p| {
            let mut calls = vec![("open_file".to_string(), json!({"path": p}))];
            for i in 0..99 {
                if i % 2 == 0 {
                    calls.push(("add_measure".into(), add(i)));
                } else {
                    calls.push((
                        "validate_dax".into(),
                        json!({"expression": format!("CALCULATE([Measure {i}], ALL('Table 1'))")}),
                    ));
                }
            }
            let errors = mcp_session(bin, &calls)?;
            if errors > 0 {
                return Err(format!("{errors} tool calls failed"));
            }
            Ok(Work::items(100, "call"))
        },
    );
}

// ---------------------------------------------------------------------------------------------

fn xpress9(b: &mut Bencher, quick: bool) {
    b.group("XPress9 (DataModel framing, single-threaded)");
    let sizes: &[usize] = if quick { &[1, 16] } else { &[1, 16, 64] };
    for &mb in sizes {
        if !b.wants(&format!("{mb} MB")) {
            continue;
        }
        let input = synth::compressible_bytes(mb << 20, 0x9);
        let compressed = match bicraft_xpress9::compress_datamodel(&input, None) {
            Ok(c) => c,
            Err(e) => return fail(b, "setup", e.to_string()),
        };
        let ratio = format!("ratio {:.2}", input.len() as f64 / compressed.len().max(1) as f64);
        let runs = if mb >= 64 { 3 } else { 10 };
        b.case(&format!("compress {mb} MB"), runs, || {
            let c = bicraft_xpress9::compress_datamodel(&input, None).map_err(|e| e.to_string())?;
            Ok(Work::bytes(input.len() as u64).note(format!("{} KB out", c.len() / 1024)))
        });
        b.case(&format!("decompress {mb} MB"), runs, || {
            let d = bicraft_xpress9::decompress_datamodel(&compressed, 1 << 31).map_err(|e| e.to_string())?;
            if d.len() != input.len() {
                return Err("size mismatch".into());
            }
            Ok(Work::bytes(input.len() as u64).note(ratio.clone()))
        });
    }
}

// ---------------------------------------------------------------------------------------------

/// A synthetic PBIX with `extra_measures` more measures and `padding` bytes of extra storage in
/// its backup (so the `DataModel` has realistic size).
fn synthetic_pbix(extra_measures: usize, padding: usize) -> Result<Package, String> {
    let e = |e: bicraft_pbix::Error| e.to_string();
    let mut pkg = bicraft_pbix::synthetic::pbix(SYNTHETIC_METADATA).map_err(e)?;
    if padding > 0 {
        let pad = synth::compressible_bytes(padding, 0xAB);
        let backup = bicraft_pbix::synthetic::abf(&[
            ("metadata.sqlitedb", SYNTHETIC_METADATA),
            ("0.CryptKey.bin", b"\x01\x02\x03"),
            ("Sales (3).tbl/4.prt/1.Sales (3).Amount (5).0.idf", &pad),
        ])
        .map_err(e)?;
        let dm = bicraft_xpress9::compress_datamodel(&backup, None).map_err(|x| x.to_string())?;
        pkg.put(bicraft_pbix::DATA_MODEL, dm, true);
    }
    if extra_measures > 0 {
        let mut model = bicraft_pbix::read_model(&pkg).map_err(e)?;
        let table = model
            .tables
            .iter_mut()
            .find(|t| !t.columns.is_empty())
            .ok_or("no table")?;
        let tname = table.name.clone();
        let col = table.columns.first().map(|c| c.name.clone()).unwrap_or_default();
        for i in 0..extra_measures {
            table.measures.push(Measure {
                name: format!("Bench {i}"),
                expression: format!("SUM('{tname}'[{col}]) * {i}"),
                format_string: Some("#,0".into()),
                ..Measure::default()
            });
        }
        pkg = bicraft_pbix::write_model(&pkg, &model).map_err(e)?;
    }
    Ok(pkg)
}

/// The PBIX cases on one package.
fn pbix_cases(b: &mut Bencher, label: &str, bytes: &[u8], runs: usize) {
    let size = bytes.len() as u64;
    let pkg = match Package::read_bytes(bytes) {
        Ok(p) => p,
        Err(e) => return fail(b, &format!("open {label}"), e.to_string()),
    };
    b.case(&format!("Package::read_bytes (ZIP), {label}"), runs, || {
        let p = Package::read_bytes(bytes).map_err(|e| e.to_string())?;
        Ok(Work::items(p.parts.len() as u64, "part").with_bytes(size))
    });
    let dm = pkg
        .get(bicraft_pbix::DATA_MODEL)
        .map_or(0, |p| p.data.len() as u64);
    b.case(
        &format!("read_model (XPress9 + ABF + SQLite), {label}"),
        runs,
        || {
            let m = bicraft_pbix::read_model(&pkg).map_err(|e| e.to_string())?;
            Ok(Work::bytes(dm).note(format!(
                "{} tables, {} measures",
                m.tables.len(),
                measure_count(&m)
            )))
        },
    );
    b.case(&format!("read_report, {label}"), runs, || {
        let r = bicraft_pbix::read_report(&pkg).map_err(|e| e.to_string())?;
        let v: usize = r.pages.iter().map(|p| p.visuals.len()).sum();
        Ok(Work::items(v as u64, "visual").note(format!("{} pages, {v} visuals", r.pages.len())))
    });
    let model = bicraft_pbix::read_model(&pkg);
    match model {
        Ok(mut model) => {
            if let Some(t) = model
                .tables
                .iter_mut()
                .find(|t| !t.columns.is_empty() && !t.is_hidden)
            {
                t.measures.push(Measure {
                    name: "Bench added".into(),
                    expression: "1".into(),
                    ..Measure::default()
                });
            }
            b.case(
                &format!("write_model (+1 measure: SQLite + ABF + XPress9), {label}"),
                runs,
                || {
                    let out = bicraft_pbix::write_model(&pkg, &model).map_err(|e| e.to_string())?;
                    let n = out.get(bicraft_pbix::DATA_MODEL).map_or(0, |p| p.data.len());
                    Ok(Work::bytes(dm).note(format!("{} KB DataModel", n / 1024)))
                },
            );
        }
        Err(e) => fail(b, &format!("write_model, {label}"), e.to_string()),
    }
    b.case(&format!("Package::to_bytes (ZIP), {label}"), runs, || {
        let out = pkg.to_bytes().map_err(|e| e.to_string())?;
        Ok(Work::bytes(out.len() as u64))
    });
}

fn pbix(b: &mut Bencher, opts: &Opts, bin: &Path, scratch: &Path) {
    b.group("PBIX (synthetic)");
    let pad = if opts.quick { 8 << 20 } else { 32 << 20 };
    let variants = [
        ("small (fixture model)".to_string(), 0usize, 0usize),
        (format!("1,000 measures + {} MB storage", pad >> 20), 1_000, pad),
    ];
    for (label, measures, padding) in variants {
        if !b.wants(&label) {
            continue;
        }
        let bytes =
            match synthetic_pbix(measures, padding).and_then(|p| p.to_bytes().map_err(|e| e.to_string())) {
                Ok(x) => x,
                Err(e) => {
                    fail(b, &format!("setup {label}"), e);
                    continue;
                }
            };
        if !opts.cli_only {
            pbix_cases(b, &label, &bytes, if padding > 0 { 5 } else { 20 });
        }
        let path = scratch.join(if padding > 0 { "big.pbix" } else { "small.pbix" });
        if let Err(e) = write_file(&path, &bytes) {
            fail(b, &label, e);
            continue;
        }
        let p = path.to_string_lossy().into_owned();
        b.case(&format!("CLI inspect, {label}"), 10, || {
            cli(bin, &["inspect", &p], &[0])?;
            Ok(Work::bytes(bytes.len() as u64))
        });
        let mut i = 0;
        let copy = path.with_extension("copy.pbix");
        b.case_with_setup(
            &format!("CLI run --open … measure.add --save (in place), {label}"),
            5,
            || std::fs::copy(&path, &copy).map_err(|e| e.to_string()),
            |_| {
                i += 1;
                let cmd = format!(
                    "measure.add={}",
                    json!({"table": "Sales", "name": format!("Cli {i}"), "expression": "1", "force": true})
                );
                cli(
                    bin,
                    &["run", "--open", path_str(&copy)?, "--cmd", &cmd, "--save"],
                    &[0],
                )?;
                Ok(Work::bytes(bytes.len() as u64))
            },
        );
    }
}

fn real(b: &mut Bencher, opts: &Opts, bin: &Path) {
    b.group("Real file (local only; timings and counts only)");
    let path = opts.real_file.clone().or_else(|| {
        bicraft_env::var_os("BICRAFT_TESTDATA").map(|d| PathBuf::from(d).join("sample1").join("report.pbix"))
    });
    let Some(path) = path.filter(|p| p.is_file()) else {
        return b.skip(
            "real PBIX",
            "set BICRAFT_TESTDATA (sample1/report.pbix) or pass --real-file",
        );
    };
    let bytes = match std::fs::read(&path) {
        Ok(x) => x,
        Err(e) => return fail(b, "real PBIX", e.to_string()),
    };
    let label = format!("real sample ({:.1} MB)", bytes.len() as f64 / 1e6);
    if !opts.cli_only {
        pbix_cases(b, &label, &bytes, 5);
    }
    let p = path.to_string_lossy().into_owned();
    b.case(&format!("CLI inspect, {label}"), 5, || {
        cli(bin, &["inspect", &p], &[0])?;
        Ok(Work::bytes(bytes.len() as u64))
    });
    b.case(&format!("CLI validate-model, {label}"), 5, || {
        cli(bin, &["validate-model", &p], &[0, 1])?;
        Ok(Work::bytes(bytes.len() as u64))
    });
    b.case(&format!("CLI validate-report, {label}"), 5, || {
        cli(bin, &["validate-report", &p], &[0, 1])?;
        Ok(Work::bytes(bytes.len() as u64))
    });
}

// ---------------------------------------------------------------------------------------------

fn sqlite(b: &mut Bencher, quick: bool) {
    use bicraft_sqlite::{Database, DatabaseBuilder, Value as Sv};
    b.group("SQLite (pure Rust)");
    let rows: i64 = if quick { 50_000 } else { 200_000 };
    let build = || -> Result<Vec<u8>, String> {
        let e = |e: bicraft_sqlite::Error| e.to_string();
        let mut db = DatabaseBuilder::new(4096).map_err(e)?;
        db.create_table(
            "t",
            "CREATE TABLE t (id INTEGER PRIMARY KEY, a INTEGER, b REAL, c TEXT, d TEXT)",
        )
        .map_err(e)?;
        for i in 1..=rows {
            db.insert(
                "t",
                i,
                vec![
                    Sv::Null,
                    Sv::Integer(i * 7),
                    Sv::Real(i as f64 / 3.0),
                    Sv::Text(format!("name {}", i % 1000)),
                    Sv::Text(format!("{{\"k\":{i},\"v\":\"some longer text value\"}}")),
                ],
            )
            .map_err(e)?;
        }
        db.finish().map_err(e)
    };
    let image = match build() {
        Ok(x) => x,
        Err(e) => return fail(b, "setup", e),
    };
    let size = image.len() as u64;
    b.case(&format!("DatabaseBuilder: {rows} rows × 5 columns"), 10, || {
        let img = build()?;
        Ok(Work::items(rows as u64, "row").with_bytes(img.len() as u64))
    });
    b.case(&format!("Database::open + scan {rows} rows"), 10, || {
        let db = Database::from_bytes(&image).map_err(|e| e.to_string())?;
        let mut n = 0u64;
        for r in db.rows("t").map_err(|e| e.to_string())? {
            r.map_err(|e| e.to_string())?;
            n += 1;
        }
        Ok(Work::items(n, "row").with_bytes(size))
    });
}

fn vertipaq(b: &mut Bencher, quick: bool) {
    use bicraft_vertipaq::{
        ColumnEncoding, Limits, StorageType, decode_column, decode_data_ids, parse_idfmeta,
    };
    b.group("VertiPaq column decode (synthetic hybrid RLE + bit-packed)");
    let rows = if quick { 2 << 20 } else { 8 << 20 };
    let (idf, meta, _) = match super::vertipaq::hybrid_column(rows, 1000, 5) {
        Ok(x) => x,
        Err(e) => return fail(b, "setup", e),
    };
    let dict = super::vertipaq::long_dictionary(1000);
    let note = format!("{} MB .idf", idf.len() >> 20);
    b.case(&format!("data ids only, {} M rows", rows >> 20), 10, || {
        let m = parse_idfmeta(&meta).map_err(|e| e.to_string())?;
        let ids = decode_data_ids(&idf, &m, &Limits::default()).map_err(|e| e.to_string())?;
        Ok(Work::items(ids.len() as u64, "cell")
            .with_bytes(idf.len() as u64)
            .note(note.clone()))
    });
    b.case(
        &format!("value-encoded column, {} M rows", rows >> 20),
        10,
        || {
            let c = decode_column(
                &idf,
                &meta,
                None,
                ColumnEncoding::value(-3, 1.0, StorageType::Long),
            )
            .map_err(|e| e.to_string())?;
            Ok(Work::items(c.len() as u64, "cell").with_bytes(idf.len() as u64))
        },
    );
    b.case(
        &format!(
            "hash-encoded column (1,000-value dictionary), {} M rows",
            rows >> 20
        ),
        10,
        || {
            let c = decode_column(&idf, &meta, Some(&dict), ColumnEncoding::hash(StorageType::Long))
                .map_err(|e| e.to_string())?;
            Ok(Work::items(c.len() as u64, "cell").with_bytes(idf.len() as u64))
        },
    );
}
