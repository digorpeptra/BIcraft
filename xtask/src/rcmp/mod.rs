//! `cargo xtask render-compare`: per-visual comparison of `bicraft-cli render` output with Power BI
//! Desktop's own rendering (tools/render-oracle; method and limits in docs/render.md).

pub(crate) mod img;
mod metrics;
#[cfg(test)]
mod oracle;
mod text;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use img::Rgb;
use text::{Run, TextReport, round};

pub const USAGE: &str = "\
usage: cargo xtask render-compare DESKTOP_DIR OURS_DIR LAYOUT.json [--json] [--diff DIR]
                                  [--min-sim X] [--floors FILE]
  DESKTOP_DIR  capture.ps1 output: NN-<page>.png (+ NN-<page>.json text nodes), in page order
  OURS_DIR     `bicraft-cli render FILE --out OURS_DIR [--text-json]` output, in page order
  LAYOUT.json  the generator's visual list: [{page, page_name, type, x, y, width, height}]
  --json       print the report as JSON
  --diff DIR   write Desktop | bicraft | difference crops per visual
  --min-sim X  fail when a visual's sim is below X
  --floors F   per-family floors (tools/render-oracle/floors/*.json); fail below them
";

/// One visual's result.
#[derive(Debug, Clone)]
pub struct Row {
    pub index: usize,
    pub page: String,
    pub family: String,
    pub scores: metrics::Scores,
    pub text: Option<TextReport>,
}

impl Row {
    fn json(&self) -> Value {
        let s = &self.scores;
        json!({
            "index": self.index,
            "page": self.page,
            "type": self.family,
            "sim": round(s.sim, 4),
            "ink_iou": round(s.ink, 4),
            "ssim": s.ssim.map(|v| round(v, 4)),
            "de2000": s.de.map(|v| round(v, 2)),
            "text": self.text.as_ref().map(TextReport::json),
        })
    }
}

/// Per-family means.
#[derive(Debug, Clone, Default)]
pub struct Family {
    pub n: usize,
    pub sim: f64,
    pub min_sim: f64,
    pub ink: f64,
    pub ssim: Option<f64>,
    pub de: Option<f64>,
    pub text: Option<f64>,
}

fn mean(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (mut s, mut n) = (0.0, 0usize);
    for x in v {
        s += x;
        n += 1;
    }
    (n > 0).then(|| s / n as f64)
}

pub fn families(rows: &[Row]) -> BTreeMap<String, Family> {
    let mut by: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by.entry(r.family.clone()).or_default().push(r);
    }
    by.into_iter()
        .map(|(k, rs)| {
            let f = Family {
                n: rs.len(),
                sim: mean(rs.iter().map(|r| r.scores.sim)).unwrap_or(0.0),
                min_sim: rs.iter().map(|r| r.scores.sim).fold(f64::INFINITY, f64::min),
                ink: mean(rs.iter().map(|r| r.scores.ink)).unwrap_or(0.0),
                ssim: mean(rs.iter().filter_map(|r| r.scores.ssim)),
                de: mean(rs.iter().filter_map(|r| r.scores.de)),
                text: mean(rs.iter().filter_map(|r| r.text.as_ref().map(TextReport::score))),
            };
            (k, f)
        })
        .collect()
}

/// The inputs of one comparison.
pub struct Inputs<'a> {
    pub desktop: &'a Path,
    pub ours: &'a Path,
    pub layout: &'a Path,
    pub diff: Option<&'a Path>,
}

fn pngs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
        .collect();
    v.sort();
    Ok(v)
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// Python's `round()` (half to even), used for the crop rectangle.
fn round_half_even(x: f64) -> i64 {
    let r = x.round();
    let v = if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    };
    v.clamp(-1e12, 1e12) as i64
}

struct Page {
    ours: Rgb,
    desk: Rgb,
    ours_text: Option<Vec<Run>>,
    desk_text: Option<Vec<Run>>,
}

pub fn compare(inp: &Inputs<'_>) -> Result<Vec<Row>, String> {
    let layout = read_json(inp.layout)
        .ok_or_else(|| format!("{}: cannot read the layout JSON", inp.layout.display()))?;
    let layout = layout.as_array().ok_or("the layout must be a JSON list")?;
    let ours = pngs(inp.ours)?;
    let desk = pngs(inp.desktop)?;
    if ours.is_empty() || desk.is_empty() {
        return Err("no PNG pages found".into());
    }
    let mut cache: BTreeMap<usize, Option<Page>> = BTreeMap::new();
    let mut rows = Vec::new();
    for (vi, v) in layout.iter().enumerate() {
        let Some(pi) = v
            .get("page")
            .and_then(Value::as_u64)
            .and_then(|p| usize::try_from(p).ok())
        else {
            continue;
        };
        let page_width = v
            .get("page_width")
            .and_then(Value::as_f64)
            .filter(|w| *w > 0.0)
            .unwrap_or(1280.0);
        let page = cache.entry(pi).or_insert_with(|| {
            let (po, pd) = (ours.get(pi)?, desk.get(pi)?);
            let o = Rgb::load_png(po).ok()?;
            let d = Rgb::load_png(pd).ok()?.resize(o.w, o.h)?;
            let ours_text = read_json(&po.with_extension("json")).map(|j| text::our_runs(&j));
            let desk_text = read_json(&pd.with_extension("json")).map(|j| text::desktop_runs(&j, page_width));
            Some(Page {
                ours: o,
                desk: d,
                ours_text,
                desk_text,
            })
        });
        let Some(page) = page.as_ref() else { continue };
        let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
        let k = page.ours.w as f64 / page_width;
        let b = [x, y, x + w, y + h].map(|z| round_half_even(z * k));
        let (Some(co), Some(cd)) = (
            page.ours.crop(b[0], b[1], b[2], b[3]),
            page.desk.crop(b[0], b[1], b[2], b[3]),
        ) else {
            continue;
        };
        let scores = metrics::score(&cd, &co);
        let text = match (&page.desk_text, &page.ours_text) {
            (Some(d), Some(o)) => {
                let dr: Vec<&Run> = d.iter().filter(|r| r.inside(x, y, w, h)).collect();
                let or: Vec<&Run> = o.iter().filter(|r| r.inside(x, y, w, h)).collect();
                Some(text::match_runs(&dr, &or))
            }
            _ => None,
        };
        let name = v
            .get("page_name")
            .and_then(Value::as_str)
            .unwrap_or("page")
            .to_string();
        let family = v.get("type").and_then(Value::as_str).unwrap_or("?").to_string();
        if let Some(dir) = inp.diff {
            write_diff(dir, &format!("{name}-{vi:03}-{family}.png"), &cd, &co)?;
        }
        rows.push(Row {
            index: vi,
            page: name,
            family,
            scores,
            text,
        });
    }
    Ok(rows)
}

/// Desktop | bicraft | |grey difference| ×4 (white = equal), separated by red bars.
fn write_diff(dir: &Path, name: &str, d: &Rgb, o: &Rgb) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (gd, go) = (d.grey(), o.grey());
    let mut diff = Rgb::new(d.w, d.h, [255; 3]).ok_or("crop too large")?;
    for (p, (a, b)) in diff.px.iter_mut().zip(gd.px.iter().zip(&go.px)) {
        let v = 255u8.saturating_sub(a.abs_diff(*b).saturating_mul(4));
        *p = [255, v, v];
    }
    let gap = 4usize;
    let mut side =
        Rgb::new(d.w.saturating_mul(3).saturating_add(2 * gap), d.h, [255, 0, 0]).ok_or("crop too large")?;
    side.paste(d, 0, 0);
    side.paste(o, d.w + gap, 0);
    side.paste(&diff, 2 * (d.w + gap), 0);
    let bytes = side.encode_png()?;
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    std::fs::write(dir.join(safe), bytes).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Floors: `{"<family>" | "*": {"sim": x, "ink": x, "ssim": x, "text": x, "de": max}}`, applied
/// to the family means ("*" to every family without its own entry); `de` is a ceiling.
pub fn check(rows: &[Row], floors: Option<&Value>, min_sim: Option<f64>) -> Vec<String> {
    let mut fails = Vec::new();
    if let Some(m) = min_sim {
        for r in rows.iter().filter(|r| r.scores.sim < m) {
            fails.push(format!(
                "{} #{} {}: sim {:.4} < {m}",
                r.page, r.index, r.family, r.scores.sim
            ));
        }
    }
    let Some(floors) = floors else { return fails };
    for (name, f) in families(rows) {
        let Some(lim) = floors.get(&name).or_else(|| floors.get("*")) else {
            continue;
        };
        let g = |k: &str| lim.get(k).and_then(Value::as_f64);
        let mut low = |what: &str, got: Option<f64>, floor: Option<f64>| {
            if let (Some(got), Some(fl)) = (got, floor)
                && got < fl
            {
                fails.push(format!("{name}: {what} {got:.4} < floor {fl}"));
            }
        };
        low("sim", Some(f.sim), g("sim"));
        low("ink", Some(f.ink), g("ink"));
        low("ssim", f.ssim, g("ssim"));
        low("text", f.text, g("text"));
        if let (Some(got), Some(max)) = (f.de, g("de"))
            && got > max
        {
            fails.push(format!("{name}: ΔE00 {got:.2} > ceiling {max}"));
        }
    }
    fails
}

fn opt(v: Option<f64>, digits: usize) -> String {
    v.map_or_else(|| "-".into(), |x| format!("{x:.digits$}"))
}

pub fn table(rows: &[Row]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<14} {:<34} {:>6} {:>6} {:>6} {:>6} {:>9}",
        "page", "visual", "sim", "ink", "ssim", "ΔE00", "text"
    );
    for r in rows {
        let t = r.text.as_ref().map_or_else(
            || "-".into(),
            |t| format!("{}/{}", t.pairs.len(), t.desktop.max(t.ours)),
        );
        let _ = writeln!(
            s,
            "{:<14} {:<34} {:>6.3} {:>6.3} {:>6} {:>6} {:>9}",
            r.page,
            r.family,
            r.scores.sim,
            r.scores.ink,
            opt(r.scores.ssim, 3),
            opt(r.scores.de, 2),
            t
        );
        if let Some(t) = &r.text {
            for m in t.missing.iter().take(6) {
                let _ = writeln!(s, "{:>16}missing: {m:?}", "");
            }
            for m in t.extra.iter().take(6) {
                let _ = writeln!(s, "{:>16}extra:   {m:?}", "");
            }
            for p in t.pairs.iter().filter(|p| p.differs()).take(6) {
                let _ = writeln!(
                    s,
                    "{:>16}differs: {:?} size {:+.2} ΔE {:.1} weight {:+} offset ({:+.1}, {:+.1}){}",
                    "",
                    p.text,
                    p.dsize,
                    p.de,
                    p.dweight,
                    p.dx,
                    p.dy,
                    p.family
                        .as_ref()
                        .map(|(d, o)| format!(" family {d} vs {o}"))
                        .unwrap_or_default()
                );
            }
        }
    }
    let _ = writeln!(
        s,
        "\n{:<38} {:>3} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "family", "n", "sim", "min", "ink", "ssim", "ΔE00", "text"
    );
    for (k, f) in families(rows) {
        let _ = writeln!(
            s,
            "{:<38} {:>3} {:>6.3} {:>6.3} {:>6.3} {:>6} {:>6} {:>6}",
            k,
            f.n,
            f.sim,
            f.min_sim,
            f.ink,
            opt(f.ssim, 3),
            opt(f.de, 2),
            opt(f.text, 2)
        );
    }
    let _ = writeln!(
        s,
        "{:<38} {:>3} {:>6.3} {:>6} {:>6.3} {:>6}",
        "mean",
        rows.len(),
        mean(rows.iter().map(|r| r.scores.sim)).unwrap_or(0.0),
        "",
        mean(rows.iter().map(|r| r.scores.ink)).unwrap_or(0.0),
        opt(mean(rows.iter().filter_map(|r| r.scores.ssim)), 3)
    );
    s
}

pub fn report_json(rows: &[Row], fails: &[String]) -> Value {
    json!({
        "visuals": rows.iter().map(Row::json).collect::<Vec<_>>(),
        "families": families(rows).into_iter().map(|(k, f)| (k, json!({
            "n": f.n,
            "sim": round(f.sim, 4),
            "min_sim": round(f.min_sim, 4),
            "ink_iou": round(f.ink, 4),
            "ssim": f.ssim.map(|v| round(v, 4)),
            "de2000": f.de.map(|v| round(v, 2)),
            "text": f.text.map(|v| round(v, 4)),
        }))).collect::<serde_json::Map<_, _>>(),
        "failures": fails,
    })
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut pos: Vec<&str> = Vec::new();
    let (mut as_json, mut diff, mut min_sim, mut floors) = (false, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => as_json = true,
            "--diff" => diff = Some(PathBuf::from(it.next().ok_or("--diff needs a folder")?)),
            "--min-sim" => {
                let v = it.next().ok_or("--min-sim needs a number")?;
                min_sim = Some(
                    v.parse::<f64>()
                        .map_err(|_| format!("--min-sim: `{v}` is not a number"))?,
                );
            }
            "--floors" => {
                let p = PathBuf::from(it.next().ok_or("--floors needs a file")?);
                floors = Some(
                    read_json(&p).ok_or_else(|| format!("{}: cannot read the floors JSON", p.display()))?,
                );
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}\n\n{USAGE}")),
            s => pos.push(s),
        }
    }
    let [desktop, ours, layout] = pos.as_slice() else {
        return Err(format!("expected DESKTOP_DIR OURS_DIR LAYOUT.json\n\n{USAGE}"));
    };
    let rows = compare(&Inputs {
        desktop: Path::new(desktop),
        ours: Path::new(ours),
        layout: Path::new(layout),
        diff: diff.as_deref(),
    })?;
    let fails = check(&rows, floors.as_ref(), min_sim);
    if as_json {
        let s = serde_json::to_string_pretty(&report_json(&rows, &fails)).map_err(|e| e.to_string())?;
        println!("{s}");
    } else {
        print!("{}", table(&rows));
        for f in &fails {
            println!("FAIL {f}");
        }
    }
    if fails.is_empty() {
        Ok(())
    } else {
        Err(format!("{} regression(s) below the thresholds", fails.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bicraft-rcmp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn python_rounding() {
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round_half_even(-2.5), -2);
        assert_eq!(round_half_even(2.4), 2);
    }

    #[test]
    fn end_to_end_on_synthetic_pages() {
        let root = tmp("e2e");
        let (d, o, diff) = (root.join("desk"), root.join("ours"), root.join("diff"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::create_dir_all(&o).unwrap();
        // a 1280-wide page: a black box at (100,100)-(150,150); ours is shifted 30 px right and
        // Desktop's capture is 1.5x larger (resized down before comparing)
        let mut a = Rgb::new(1920, 1080, [255; 3]).unwrap();
        for y in 150..225 {
            for x in 150..225 {
                a.px[y * 1920 + x] = [0, 0, 0];
            }
        }
        let mut b = Rgb::new(1280, 720, [255; 3]).unwrap();
        for y in 100..150 {
            for x in 130..180 {
                b.px[y * 1280 + x] = [0, 0, 0];
            }
        }
        std::fs::write(d.join("01-P.png"), a.encode_png().unwrap()).unwrap();
        std::fs::write(o.join("01-P.png"), b.encode_png().unwrap()).unwrap();
        std::fs::write(
            d.join("01-P.json"),
            r#"[{"t":"Sales","tag":"text","ff":"Segoe UI","fs":"12px","fw":"400","fill":"rgb(0, 0, 0)","x":120,"y":110,"w":30,"h":16,"z":1280}]"#,
        )
        .unwrap();
        std::fs::write(
            o.join("01-P.json"),
            r##"{"texts":[{"t":"Sales","font":"Segoe UI","size":12,"weight":400,"color":"#000000","x":121,"y":110,"w":30,"h":16},{"t":"Extra","font":"Segoe UI","size":9,"weight":400,"color":"#000000","x":400,"y":400,"w":30,"h":16}]}"##,
        )
        .unwrap();
        let layout = root.join("layout.json");
        std::fs::write(
            &layout,
            r#"[{"page":0,"page_name":"P","type":"card","x":80,"y":80,"width":200,"height":100},
                {"page":0,"page_name":"P","type":"shape","x":300,"y":300,"width":200,"height":200},
                {"page":5,"page_name":"Q","type":"card","x":0,"y":0,"width":10,"height":10}]"#,
        )
        .unwrap();
        let rows = compare(&Inputs {
            desktop: &d,
            ours: &o,
            layout: &layout,
            diff: Some(&diff),
        })
        .unwrap();
        assert_eq!(rows.len(), 2, "the page that has no image is skipped");
        let card = &rows[0];
        assert!(
            card.scores.sim < 0.99 && card.scores.ink < 0.5,
            "{:?}",
            card.scores
        );
        let t = card.text.as_ref().unwrap();
        assert_eq!((t.pairs.len(), t.missing.len(), t.extra.len()), (1, 0, 0));
        let shape = &rows[1];
        assert_eq!(shape.scores.sim, 1.0);
        assert_eq!(shape.text.as_ref().unwrap().extra, vec!["Extra".to_string()]);
        assert_eq!(std::fs::read_dir(&diff).unwrap().count(), 2);

        let floors = serde_json::json!({"card": {"sim": 0.999}, "*": {"sim": 0.5, "text": 0.9}});
        let fails = check(&rows, Some(&floors), None);
        assert_eq!(fails.len(), 2, "{fails:?}");
        assert!(fails.iter().any(|f| f.starts_with("card: sim")));
        assert!(fails.iter().any(|f| f.starts_with("shape: text")));
        assert_eq!(check(&rows, None, Some(0.995)).len(), 1);
        let j = report_json(&rows, &fails);
        assert_eq!(j["visuals"].as_array().unwrap().len(), 2);
        assert!(table(&rows).contains("extra:   \"Extra\""));

        let args: Vec<String> = [
            d.to_str().unwrap(),
            o.to_str().unwrap(),
            layout.to_str().unwrap(),
            "--min-sim",
            "0.5",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(cmd(&args).is_ok());
        let bad: Vec<String> = vec!["only-one".into()];
        assert!(cmd(&bad).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
