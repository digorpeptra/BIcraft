//! `cargo xtask parity [--check]`: recompute the parity summary of `docs/parity-checklist.md`
//! from its rows, and the headline of `docs/target-app-parity.md` (M11.7, the Crafting Apps'
//! form: DeckCraft's and GridCraft's `xtask parity`).
//!
//! Two axes. **Authoring rows** `| Area | Feature | P0–P2 | D/P/M/O | Commands | Button |`:
//! one status per Power BI Desktop authoring feature, and whether the BIcraft app has a button
//! for it (`yes` / `no` / `–`). **Viewer rows** `| Area | Feature | P0–P2 | render | web | app |`:
//! one status per surface, `–` where it does not apply. D counts 1, P 0.5, M 0; O and `–` are
//! left out. Weights by priority: P0 3, P1 2, P2 1.
//!
//! **Inventory rows** `| Surface | Group and its controls | Controls | D/P/M/O | Checklist row |`
//! (M11.28, M11.29): the ledger of Power BI Desktop's UI read as a black box, one row per ribbon
//! group, pane, menu or dialog, with the number of controls counted in Desktop and BIcraft's
//! status for the group. Coverage weights each group by its controls (D 1, P 0.5, M 0, O left out).
//!
//! Without `--check` both docs are rewritten between their markers. With `--check` nothing is
//! written and a stale summary or headline is an error; `cargo xtask docs` runs the same check.

use std::path::Path;

pub const CHECKLIST: &str = "docs/parity-checklist.md";
pub const ASSESSMENT: &str = "docs/target-app-parity.md";
const COMMANDS: &str = "crates/engine/src/commands.rs";

const SUMMARY: (&str, &str) = ("<!-- SUMMARY -->", "<!-- /SUMMARY -->");
const HEADLINE: (&str, &str) = ("<!-- HEADLINE -->", "<!-- /HEADLINE -->");

const AUTHORING: &str = "## Authoring rows";
const VIEWER: &str = "## Viewer rows";
const INVENTORY: &str = "## Desktop UI inventory";

/// The three viewer surfaces, in column order.
const SURFACES: [&str; 3] = [
    "`bicraft-cli render` (PNG)",
    "Web viewer (`bicraft-cli view`, page.html)",
    "BIcraft app (egui, desktop and browser)",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Done,
    Partial,
    Missing,
    /// Out of scope, or not applicable to the surface: left out of every share.
    Out,
}

impl St {
    fn parse(s: &str) -> Option<St> {
        match s {
            "D" => Some(St::Done),
            "P" => Some(St::Partial),
            "M" => Some(St::Missing),
            "O" | "–" | "-" => Some(St::Out),
            _ => None,
        }
    }
    fn value(self) -> f64 {
        match self {
            St::Done => 1.0,
            St::Partial => 0.5,
            St::Missing | St::Out => 0.0,
        }
    }
}

#[derive(Debug)]
struct Row {
    area: String,
    feature: String,
    pri: usize,
    /// One status (authoring) or three (render, web, app).
    st: Vec<St>,
    button: bool,
}

/// One group of Desktop's UI in the inventory ledger.
#[derive(Debug)]
struct Group {
    surface: String,
    controls: usize,
    st: St,
}

#[derive(Debug, Default)]
struct Checklist {
    authoring: Vec<Row>,
    viewer: Vec<Row>,
    inventory: Vec<Group>,
}

fn priority(s: &str) -> Option<usize> {
    match s {
        "P0" => Some(0),
        "P1" => Some(1),
        "P2" => Some(2),
        _ => None,
    }
}

fn weight(p: usize) -> f64 {
    match p {
        0 => 3.0,
        1 => 2.0,
        _ => 1.0,
    }
}

/// The rows of both sections. A row with a priority but an unknown status is an error, so a
/// typo cannot drop a feature from the counts.
fn parse(text: &str) -> Result<Checklist, String> {
    let mut out = Checklist::default();
    let mut section = 0u8;
    for (n, l) in text.lines().enumerate() {
        let t = l.trim();
        if t.starts_with("## ") {
            section = match t {
                AUTHORING => 1,
                VIEWER => 2,
                INVENTORY => 3,
                _ => 0,
            };
            continue;
        }
        if section == 0 || !t.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if section == 3 {
            let Some(controls) = cells.get(2).and_then(|c| c.parse::<usize>().ok()) else {
                continue;
            };
            if cells.len() != 5 {
                return Err(format!(
                    "{CHECKLIST}:{}: an inventory row has 5 cells, found {}",
                    n + 1,
                    cells.len()
                ));
            }
            let cell = cells.get(3).copied().unwrap_or("");
            let st = match St::parse(cell) {
                Some(s) if cell != "–" && cell != "-" => s,
                _ => {
                    return Err(format!(
                        "{CHECKLIST}:{}: an inventory status is D, P, M or O, found `{cell}`",
                        n + 1
                    ));
                }
            };
            out.inventory.push(Group {
                surface: cells.first().copied().unwrap_or("").to_string(),
                controls,
                st,
            });
            continue;
        }
        let Some(pri) = cells.get(2).and_then(|c| priority(c)) else {
            continue;
        };
        if cells.len() != 6 {
            return Err(format!(
                "{CHECKLIST}:{}: expected 6 cells, found {}",
                n + 1,
                cells.len()
            ));
        }
        let cols: &[usize] = if section == 1 { &[3] } else { &[3, 4, 5] };
        let mut st = Vec::new();
        for &c in cols {
            let cell = cells.get(c).copied().unwrap_or("");
            match St::parse(cell) {
                Some(s) if section == 1 && cell != "O" && s == St::Out => {
                    return Err(format!(
                        "{CHECKLIST}:{}: an authoring status is D, P, M or O",
                        n + 1
                    ));
                }
                Some(s) => st.push(s),
                None => return Err(format!("{CHECKLIST}:{}: unknown status `{cell}`", n + 1)),
            }
        }
        let button = section == 1 && cells.get(5).is_some_and(|b| b.starts_with("yes"));
        let row = Row {
            area: cells.first().copied().unwrap_or("").to_string(),
            feature: cells.get(1).copied().unwrap_or("").to_string(),
            pri,
            st,
            button,
        };
        if section == 1 {
            out.authoring.push(row);
        } else {
            out.viewer.push(row);
        }
    }
    if out.authoring.is_empty() || out.viewer.is_empty() {
        return Err(format!(
            "{CHECKLIST}: needs rows under `{AUTHORING}` and `{VIEWER}`"
        ));
    }
    Ok(out)
}

/// Counts of one column over a set of rows.
#[derive(Default, Clone, Copy)]
struct Tally {
    done: usize,
    partial: usize,
    missing: usize,
    out: usize,
    weighted: f64,
    weights: f64,
}

impl Tally {
    fn of<'a>(rows: impl Iterator<Item = &'a Row>, col: usize) -> Tally {
        let mut t = Tally::default();
        for r in rows {
            let Some(&s) = r.st.get(col) else { continue };
            match s {
                St::Done => t.done += 1,
                St::Partial => t.partial += 1,
                St::Missing => t.missing += 1,
                St::Out => {
                    t.out += 1;
                    continue;
                }
            }
            t.weighted += weight(r.pri) * s.value();
            t.weights += weight(r.pri);
        }
        t
    }
    fn items(&self) -> usize {
        self.done + self.partial + self.missing
    }
    fn done_pct(&self) -> f64 {
        pct(self.done as f64, self.items() as f64)
    }
    fn parity(&self) -> f64 {
        pct(self.weighted, self.weights)
    }
}

fn pct(a: f64, b: f64) -> f64 {
    if b > 0.0 { 100.0 * a / b } else { 0.0 }
}

fn areas(rows: &[Row]) -> Vec<&str> {
    let mut v: Vec<&str> = Vec::new();
    for r in rows {
        if !v.contains(&r.area.as_str()) {
            v.push(&r.area);
        }
    }
    v
}

fn priorities(rows: &[Row], col: usize) -> String {
    (0..3)
        .map(|p| {
            let t = Tally::of(rows.iter().filter(|r| r.pri == p), col);
            format!("P{p} {:.0}%", t.parity())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rows of priority 0 that a column does not have done.
fn open_p0(rows: &[Row], col: usize) -> String {
    let v: Vec<&str> = rows
        .iter()
        .filter(|r| r.pri == 0 && matches!(r.st.get(col), Some(St::Partial | St::Missing)))
        .map(|r| r.feature.as_str())
        .collect();
    if v.is_empty() { "none".into() } else { v.join("; ") }
}

/// Authoring rows BIcraft has (done or partial), and those with a button in the app.
fn buttons(c: &Checklist) -> (usize, usize) {
    let have: Vec<&Row> = c
        .authoring
        .iter()
        .filter(|r| matches!(r.st.first(), Some(St::Done | St::Partial)))
        .collect();
    (have.iter().filter(|r| r.button).count(), have.len())
}

/// Inventory counts over a set of groups: groups, and controls per status.
#[derive(Default, Clone, Copy)]
struct Ledger {
    groups: usize,
    controls: usize,
    done: usize,
    partial: usize,
    missing: usize,
    out: usize,
}

impl Ledger {
    fn of<'a>(groups: impl Iterator<Item = &'a Group>) -> Ledger {
        let mut l = Ledger::default();
        for g in groups {
            l.groups += 1;
            l.controls += g.controls;
            match g.st {
                St::Done => l.done += g.controls,
                St::Partial => l.partial += g.controls,
                St::Missing => l.missing += g.controls,
                St::Out => l.out += g.controls,
            }
        }
        l
    }
    fn in_scope(&self) -> usize {
        self.done + self.partial + self.missing
    }
    fn coverage(&self) -> f64 {
        pct(
            self.done as f64 + 0.5 * self.partial as f64,
            self.in_scope() as f64,
        )
    }
}

fn surfaces(groups: &[Group]) -> Vec<&str> {
    let mut v: Vec<&str> = Vec::new();
    for g in groups {
        if !v.contains(&g.surface.as_str()) {
            v.push(&g.surface);
        }
    }
    v
}

fn inventory_summary(c: &Checklist) -> String {
    let l = Ledger::of(c.inventory.iter());
    let mut s = format!(
        "\n**Desktop UI inventory** ({} groups, {} controls read in Desktop, {} of them out of \
         scope): of the {} in-scope controls, {} sit in groups BIcraft has done, {} partial, {} \
         missing; **coverage {:.0}%** (done 1, partial 0.5, weighted by controls).\n\n",
        l.groups,
        l.controls,
        l.out,
        l.in_scope(),
        l.done,
        l.partial,
        l.missing,
        l.coverage()
    );
    s += "| Surface | Groups | Controls | Done | Partial | Missing | Out of scope | Coverage |\n\
          |---|---|---|---|---|---|---|---|\n";
    for sf in surfaces(&c.inventory) {
        let t = Ledger::of(c.inventory.iter().filter(|g| g.surface == sf));
        let cov = if t.in_scope() == 0 {
            "–".to_string()
        } else {
            format!("{:.0}%", t.coverage())
        };
        s += &format!(
            "| {sf} | {} | {} | {} | {} | {} | {} | {cov} |\n",
            t.groups, t.controls, t.done, t.partial, t.missing, t.out
        );
    }
    s
}

/// Registered engine commands: the `id: "…"` lines of the `COMMANDS` table.
pub fn command_count(root: &Path) -> Result<usize, String> {
    let p = root.join(COMMANDS);
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(text
        .lines()
        .filter(|l| l.trim_start().starts_with("id: \""))
        .count())
}

fn summary(c: &Checklist) -> String {
    let a = Tally::of(c.authoring.iter(), 0);
    let (with_button, have) = buttons(c);
    let mut s = format!(
        "**Authoring: {} of {} items done ({:.0}%), weighted parity {:.0}%** ({}); {} partial, \
         {} missing, {} out of scope. {} of the {} items BIcraft has (done or partial) have a \
         button in the app.\n\n",
        a.done,
        a.items(),
        a.done_pct(),
        a.parity(),
        priorities(&c.authoring, 0),
        a.partial,
        a.missing,
        a.out,
        with_button,
        have
    );
    s += "| Area | Items | Done | Partial | Missing | Parity |\n|---|---|---|---|---|---|\n";
    for ar in areas(&c.authoring) {
        let t = Tally::of(c.authoring.iter().filter(|r| r.area == ar), 0);
        if t.items() == 0 {
            continue;
        }
        s += &format!(
            "| {ar} | {} | {} | {} | {} | {:.0}% |\n",
            t.items(),
            t.done,
            t.partial,
            t.missing,
            t.parity()
        );
    }
    s += &format!("\nOpen P0 authoring items: {}.\n\n", open_p0(&c.authoring, 0));
    s += "**Viewer** (Desktop's reading view):\n\n| Surface | Items | Done | Partial | Missing | \
          Done share | Weighted parity |\n|---|---|---|---|---|---|---|\n";
    for (i, name) in SURFACES.iter().enumerate() {
        let t = Tally::of(c.viewer.iter(), i);
        s += &format!(
            "| {name} | {} | {} | {} | {} | {:.0}% | {:.0}% ({}) |\n",
            t.items(),
            t.done,
            t.partial,
            t.missing,
            t.done_pct(),
            t.parity(),
            priorities(&c.viewer, i)
        );
    }
    s += "\n| Area | render | web | app |\n|---|---|---|---|\n";
    for ar in areas(&c.viewer) {
        let cells: Vec<String> = (0..3)
            .map(|i| {
                let t = Tally::of(c.viewer.iter().filter(|r| r.area == ar), i);
                if t.items() == 0 {
                    "–".into()
                } else {
                    format!("{:.0}%", t.parity())
                }
            })
            .collect();
        if cells.iter().all(|c| c == "–") {
            continue;
        }
        s += &format!("| {ar} | {} |\n", cells.join(" | "));
    }
    s += &format!("\nOpen P0 viewer items in the app: {}.\n", open_p0(&c.viewer, 2));
    if !c.inventory.is_empty() {
        s += &inventory_summary(c);
    }
    s
}

fn headline(c: &Checklist, commands: usize) -> String {
    let a = Tally::of(c.authoring.iter(), 0);
    let (with_button, have) = buttons(c);
    let how = "**measured**: `cargo xtask parity` over [parity-checklist.md](parity-checklist.md) \
               (each row's status is a judgement against the evidence below)";
    let mut s = String::from("| Number | Value | Kind |\n|---|---|---|\n");
    s += &format!(
        "| **Authoring features done** | **{} of {} ({:.0}%)**; {} partial, {} missing | {how} |\n",
        a.done,
        a.items(),
        a.done_pct(),
        a.partial,
        a.missing
    );
    s += &format!(
        "| Authoring, weighted by priority | **{:.0}%** ({}) | {how} |\n",
        a.parity(),
        priorities(&c.authoring, 0)
    );
    s += &format!(
        "| Authoring items with a button in the app | **{with_button} of {have}** BIcraft has | {how} |\n"
    );
    for (i, name) in SURFACES.iter().enumerate() {
        let t = Tally::of(c.viewer.iter(), i);
        s += &format!(
            "| **Viewer: {name}** | **{} of {} ({:.0}%)** done, weighted {:.0}% | {how} |\n",
            t.done,
            t.items(),
            t.done_pct(),
            t.parity()
        );
    }
    if !c.inventory.is_empty() {
        let l = Ledger::of(c.inventory.iter());
        s += &format!(
            "| **Desktop UI controls covered** | **{:.0}%** of {} in-scope controls ({} done, {} \
             partial, {} missing; {} groups, {} controls read, {} out of scope) | **measured**: \
             `cargo xtask parity` over the inventory of [parity-checklist.md](parity-checklist.md) \
             (controls counted in Desktop as a black box; each group's status a judgement) |\n",
            l.coverage(),
            l.in_scope(),
            l.done,
            l.partial,
            l.missing,
            l.groups,
            l.controls,
            l.out
        );
    }
    s += &format!(
        "| Engine commands | **{commands}** | **measured**: `cargo xtask parity` counts the `COMMANDS` \
         table of `crates/engine/src/commands.rs` |\n"
    );
    s
}

/// `text` with the lines between the two markers replaced by `content`.
fn splice(text: &str, file: &str, (start, end): (&str, &str), content: &str) -> Result<String, String> {
    let (Some(a), Some(b)) = (text.find(start), text.find(end)) else {
        return Err(format!("{file}: needs the markers `{start}` and `{end}`"));
    };
    if b < a {
        return Err(format!("{file}: `{end}` comes before `{start}`"));
    }
    let head = text.get(..a + start.len()).unwrap_or("");
    let tail = text.get(b..).unwrap_or("");
    Ok(format!("{head}\n{content}{tail}"))
}

/// The two docs as `cargo xtask parity` writes them: `[(path, current, wanted)]`.
fn plan(root: &Path) -> Result<Vec<(String, String, String)>, String> {
    let read = |rel: &str| {
        let p = root.join(rel);
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let list = read(CHECKLIST)?;
    let doc = read(ASSESSMENT)?;
    let c = parse(&list)?;
    let commands = command_count(root)?;
    let new_list = splice(&list, CHECKLIST, SUMMARY, &summary(&c))?;
    let new_doc = splice(&doc, ASSESSMENT, HEADLINE, &headline(&c, commands))?;
    Ok(vec![
        (CHECKLIST.into(), list, new_list),
        (ASSESSMENT.into(), doc, new_doc),
    ])
}

/// Stale summaries, for `cargo xtask docs`.
pub fn check(root: &Path) -> Vec<String> {
    match plan(root) {
        Ok(files) => files
            .into_iter()
            .filter(|(_, cur, new)| cur.replace("\r\n", "\n") != new.replace("\r\n", "\n"))
            .map(|(f, _, _)| format!("{f}: parity summary is stale (run `cargo xtask parity`)"))
            .collect(),
        Err(e) => vec![e],
    }
}

pub fn cmd(args: &[String]) -> Result<(), String> {
    let mut check_only = false;
    for a in args {
        match a.as_str() {
            "--check" => check_only = true,
            other => {
                return Err(format!(
                    "parity: unknown argument `{other}` (usage: cargo xtask parity [--check])"
                ));
            }
        }
    }
    let root = crate::root();
    if check_only {
        let problems = check(&root);
        if problems.is_empty() {
            eprintln!("parity: {CHECKLIST} and {ASSESSMENT} are up to date");
            return Ok(());
        }
        return Err(problems.join("\n"));
    }
    for (f, cur, new) in plan(&root)? {
        if cur != new {
            std::fs::write(root.join(&f), &new).map_err(|e| format!("{f}: {e}"))?;
            eprintln!("parity: wrote {f}");
        }
    }
    let list = std::fs::read_to_string(root.join(CHECKLIST)).map_err(|e| e.to_string())?;
    print!("{}", summary(&parse(&list)?));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
<!-- SUMMARY -->
old
<!-- /SUMMARY -->

## Authoring rows

| Area | Feature | Priority | Status | Commands | Button |
|---|---|---|---|---|---|
| A | x | P0 | D | `a.b` | yes |
| A | y | P2 | M | — | no |
| B | z | P1 | P | `c.d` | no |
| B | out | P0 | O | — | – |

## Viewer rows

| Area | Feature | Priority | render | web | app |
|---|---|---|---|---|---|
| V | v1 | P0 | D | D | P |
| V | v2 | P1 | – | M | D |

## Desktop UI inventory

| Surface | Group | Controls | Status | Row |
|---|---|---|---|---|
| Ribbon | g1 | 4 | D | x |
| Ribbon | g2 | 2 | P | y |
| Dialogs | g3 | 3 | M | — |
| Dialogs | g4 | 5 | O | — |
";

    #[test]
    fn scores_both_axes() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.authoring.len(), 4);
        assert_eq!(c.viewer.len(), 2);
        let s = summary(&c);
        // Authoring: (3·1 + 1·0 + 2·0.5) / 6 = 67%; 1 of 3 in-scope items done.
        assert!(
            s.contains("**Authoring: 1 of 3 items done (33%), weighted parity 67%**"),
            "{s}"
        );
        assert!(s.contains("1 out of scope. 1 of the 2 items"), "{s}");
        // render: v1 only (v2 not applicable) = 100%; web (3·1 + 2·0) / 5 = 60%;
        // app (3·0.5 + 2·1) / 5 = 70%.
        assert!(s.contains("| 1 | 1 | 0 | 0 | 100% | 100%"), "{s}");
        assert!(s.contains("| 2 | 1 | 0 | 1 | 50% | 60%"), "{s}");
        assert!(s.contains("| 2 | 1 | 1 | 0 | 50% | 70%"), "{s}");
        assert!(s.contains("Open P0 viewer items in the app: v1."), "{s}");
        // Inventory: (4 + 0.5·2 + 0·3) / 9 = 56%; the 5 out-of-scope controls left out.
        assert_eq!(c.inventory.len(), 4);
        assert!(s.contains("**coverage 56%**"), "{s}");
        assert!(s.contains("| Ribbon | 2 | 6 | 4 | 2 | 0 | 0 | 83% |"), "{s}");
        assert!(s.contains("| Dialogs | 2 | 8 | 0 | 0 | 3 | 5 | 0% |"), "{s}");
        let h = headline(&c, 7);
        assert!(h.contains("**56%** of 9 in-scope controls"), "{h}");
        assert!(h.contains("**1 of 3 (33%)**"), "{h}");
        assert!(h.contains("| **7** |"), "{h}");
    }

    #[test]
    fn splices_between_markers_and_is_stable() {
        let c = parse(SAMPLE).unwrap();
        let once = splice(SAMPLE, "t", SUMMARY, &summary(&c)).unwrap();
        assert!(!once.contains("\nold\n"));
        let twice = splice(&once, "t", SUMMARY, &summary(&parse(&once).unwrap())).unwrap();
        assert_eq!(once, twice);
        assert!(splice("no markers", "t", SUMMARY, "x").is_err());
    }

    #[test]
    fn unknown_statuses_and_short_rows_fail() {
        let bad = SAMPLE.replace("| B | z | P1 | P |", "| B | z | P1 | X |");
        assert!(parse(&bad).unwrap_err().contains("unknown status `X`"));
        let dash = SAMPLE.replace("| B | z | P1 | P |", "| B | z | P1 | – |");
        assert!(parse(&dash).is_err(), "`–` is for viewer surfaces only");
        let short = SAMPLE.replace("| V | v2 | P1 | – | M | D |", "| V | v2 | P1 | – | M |");
        assert!(parse(&short).unwrap_err().contains("expected 6 cells"));
        let inv = SAMPLE.replace("| g1 | 4 | D |", "| g1 | 4 | – |");
        assert!(parse(&inv).unwrap_err().contains("inventory status"));
        let inv = SAMPLE.replace("| g1 | 4 | D | x |", "| g1 | 4 | D |");
        assert!(parse(&inv).unwrap_err().contains("5 cells"));
        assert!(parse("nothing").is_err());
    }
}
