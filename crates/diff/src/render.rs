//! Human forms of a [`Diff`]: plain text (the CLI default) and Markdown (a pull-request
//! comment). Both end every line with `\n` and depend on nothing but the diff, so they are
//! byte-identical on every platform.

use std::fmt::Write as _;

use serde_json::Value;

use crate::lines::{self, Line};
use crate::{Area, Change, Diff, How, Kind, Prop};

/// Longest single value printed inline (characters).
const MAX_INLINE: usize = 200;
/// Most lines printed for one expression or line diff.
const MAX_BLOCK_LINES: usize = 400;
/// Most formatting entries printed for one object.
const MAX_FORMATTING: usize = 30;

/// Control characters (newlines, tabs) as spaces: one object, one line.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

/// A value on one line: strings JSON-quoted, absent as `(none)`.
fn inline(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "(none)".into(),
        Some(v) => cut(
            &one_line(&serde_json::to_string(v).unwrap_or_default()),
            MAX_INLINE,
        ),
    }
}

/// The text of a block property (an expression), when it should be shown as lines.
fn block_text(p: &Prop) -> Option<(Option<&str>, Option<&str>)> {
    let (a, b) = (
        p.before.as_ref().and_then(Value::as_str),
        p.after.as_ref().and_then(Value::as_str),
    );
    let multiline = a.is_some_and(|t| t.contains('\n')) || b.is_some_and(|t| t.contains('\n'));
    let expression = p.property.contains("expression") || p.property.starts_with("filter ");
    if (a.is_some() || b.is_some()) && (multiline || expression) {
        Some((a, b))
    } else {
        None
    }
}

fn symbol(k: Kind) -> &'static str {
    match k {
        Kind::Added => "+",
        Kind::Removed => "-",
        Kind::Changed => "~",
        Kind::Renamed | Kind::Moved => ">",
    }
}

fn how(h: Option<How>) -> &'static str {
    match h {
        Some(How::LineageTag) => ", same lineage tag",
        Some(How::Case) => ", same name ignoring case",
        Some(How::Content) => ", same content",
        Some(How::Name) | None => "",
    }
}

fn headline(c: &Change) -> String {
    let mut s = one_line(&c.id);
    if let Some(l) = &c.label {
        let _ = write!(s, " [{}]", one_line(l));
    }
    match (c.change, &c.from) {
        (Kind::Renamed, Some(f)) => {
            let _ = write!(s, " (renamed from {}{})", one_line(f), how(c.how));
        }
        (Kind::Moved, Some(f)) => {
            let _ = write!(s, " (moved from {})", one_line(f));
        }
        _ => {}
    }
    s
}

/// `+ line` / `- line` / `  line` rows of a before/after text.
fn diff_rows(a: Option<&str>, b: Option<&str>) -> Vec<String> {
    let a = crate::norm::lf(a.unwrap_or(""));
    let b = crate::norm::lf(b.unwrap_or(""));
    let mut rows: Vec<String> = lines::diff(&a, &b)
        .into_iter()
        .map(|l| match l {
            Line::Same(s) => format!("  {s}"),
            Line::Del(s) => format!("- {s}"),
            Line::Ins(s) => format!("+ {s}"),
        })
        .collect();
    if rows.len() > MAX_BLOCK_LINES {
        let more = rows.len() - MAX_BLOCK_LINES;
        rows.truncate(MAX_BLOCK_LINES);
        rows.push(format!("… {more} more lines"));
    }
    rows
}

fn block_rows(text: &str) -> Vec<String> {
    let text = crate::norm::lf(text);
    let mut rows: Vec<String> = text.lines().map(str::to_string).collect();
    if rows.len() > MAX_BLOCK_LINES {
        let more = rows.len() - MAX_BLOCK_LINES;
        rows.truncate(MAX_BLOCK_LINES);
        rows.push(format!("… {more} more lines"));
    }
    rows
}

/// The detail lines of a change, shared by both forms: `(text, block)`, a block being lines
/// to show verbatim (an expression or a line diff).
enum Detail {
    Text(String),
    Block {
        title: String,
        rows: Vec<String>,
        diff: bool,
    },
}

fn details(c: &Change) -> Vec<Detail> {
    let mut out = Vec::new();
    for p in &c.properties {
        let name = one_line(&p.property);
        if p.cosmetic {
            out.push(Detail::Text(format!("{name}: whitespace, comments or case only")));
        } else if let Some((a, b)) = block_text(p) {
            out.push(Detail::Block {
                title: format!("{name}:"),
                rows: diff_rows(a, b),
                diff: true,
            });
        } else {
            out.push(Detail::Text(format!(
                "{name}: {} -> {}",
                inline(p.before.as_ref()),
                inline(p.after.as_ref())
            )));
        }
    }
    for f in &c.fields {
        let mut parts: Vec<String> = f.added.iter().map(|x| format!("+ {}", one_line(x))).collect();
        parts.extend(f.removed.iter().map(|x| format!("- {}", one_line(x))));
        if f.reordered {
            parts.push("reordered".into());
        }
        out.push(Detail::Text(format!(
            "fields {}: {}",
            one_line(&f.role),
            parts.join(", ")
        )));
    }
    for (i, p) in c.formatting.iter().enumerate() {
        if i == MAX_FORMATTING {
            out.push(Detail::Text(format!(
                "formatting: … {} more",
                c.formatting.len() - MAX_FORMATTING
            )));
            break;
        }
        out.push(Detail::Text(format!(
            "formatting {}: {} -> {}",
            one_line(&p.property),
            inline(p.before.as_ref()),
            inline(p.after.as_ref())
        )));
    }
    for v in [&c.before, &c.after].into_iter().flatten() {
        match v.get("expression").and_then(Value::as_str) {
            Some(e) => {
                let mut rest = v.clone();
                if let Some(o) = rest.as_object_mut() {
                    o.remove("expression");
                }
                if rest.as_object().is_some_and(|o| !o.is_empty()) {
                    out.push(Detail::Text(inline(Some(&rest))));
                }
                out.push(Detail::Block {
                    title: "expression:".into(),
                    rows: block_rows(e),
                    diff: false,
                });
            }
            None => out.push(Detail::Text(inline(Some(v)))),
        }
    }
    out
}

fn summary_line(d: &Diff) -> String {
    let s = &d.summary;
    format!(
        "{} added, {} removed, {} changed, {} renamed, {} moved",
        s.added, s.removed, s.changed, s.renamed, s.moved
    )
}

fn data_line(d: &Diff) -> Option<String> {
    d.data.map(|x| {
        if x.same {
            "data snapshot: same".to_string()
        } else {
            format!(
                "data snapshot: changed ({} bytes, CRC-32 {:08x} -> {} bytes, CRC-32 {:08x})",
                x.a.size, x.a.crc32, x.b.size, x.b.crc32
            )
        }
    })
}

fn area_title(a: Area) -> &'static str {
    match a {
        Area::Model => "Model",
        Area::Report => "Report",
    }
}

/// The plain-text form.
pub fn text(d: &Diff, a: &str, b: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "a: {}", one_line(a));
    let _ = writeln!(s, "b: {}", one_line(b));
    if d.identical {
        s.push_str("No differences.\n");
    }
    let mut area = None;
    for c in &d.changes {
        if area != Some(c.area) {
            area = Some(c.area);
            let _ = writeln!(s, "\n{}", area_title(c.area));
        }
        let _ = writeln!(s, "  {} {}", symbol(c.change), headline(c));
        for det in details(c) {
            match det {
                Detail::Text(t) => {
                    let _ = writeln!(s, "      {t}");
                }
                Detail::Block { title, rows, diff } => {
                    let _ = writeln!(s, "      {title}");
                    for r in rows {
                        let indent = if diff { "        " } else { "          " };
                        let _ = writeln!(s, "{indent}{r}");
                    }
                }
            }
        }
    }
    if !d.identical {
        let _ = writeln!(s, "\n{}", summary_line(d));
    }
    if let Some(l) = data_line(d) {
        let _ = writeln!(s, "{l}");
    }
    for n in &d.notes {
        let _ = writeln!(s, "note: {}", one_line(n));
    }
    s
}

/// `` `text` `` with enough backticks to hold the text's own.
fn code(s: &str) -> String {
    let s = one_line(s);
    let run = longest_run(&s, '`');
    if run == 0 {
        format!("`{s}`")
    } else {
        let fence = "`".repeat(run + 1);
        format!("{fence} {s} {fence}")
    }
}

fn longest_run(s: &str, ch: char) -> usize {
    let (mut best, mut cur) = (0, 0);
    for c in s.chars() {
        if c == ch {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Markdown special characters escaped in running text.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in one_line(s).chars() {
        if "\\`*_{}[]()<>#+-.!|~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Added => "added",
        Kind::Removed => "removed",
        Kind::Changed => "changed",
        Kind::Renamed => "renamed",
        Kind::Moved => "moved",
    }
}

/// The Markdown form, for a pull-request comment.
pub fn markdown(d: &Diff, a: &str, b: &str) -> String {
    let mut s = String::new();
    s.push_str("### Power BI diff\n\n");
    let _ = writeln!(s, "{} → {}\n", code(a), code(b));
    if d.identical {
        s.push_str("No differences.\n");
    } else {
        let x = &d.summary;
        s.push_str("| Added | Removed | Changed | Renamed | Moved |\n|---:|---:|---:|---:|---:|\n");
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            x.added, x.removed, x.changed, x.renamed, x.moved
        );
    }
    let mut area = None;
    for c in &d.changes {
        if area != Some(c.area) {
            area = Some(c.area);
            let _ = writeln!(s, "\n#### {}\n", area_title(c.area));
        }
        let mut head = format!("- **{}** {}", kind_word(c.change), code(&c.id));
        if let Some(l) = &c.label {
            let _ = write!(head, " ({})", escape(l));
        }
        match (c.change, &c.from) {
            (Kind::Renamed, Some(f)) => {
                let _ = write!(head, " from {}{}", code(f), escape(how(c.how)));
            }
            (Kind::Moved, Some(f)) => {
                let _ = write!(head, " from {}", code(f));
            }
            _ => {}
        }
        let _ = writeln!(s, "{head}");
        for det in details(c) {
            match det {
                Detail::Text(t) => {
                    let _ = writeln!(s, "  - {}", code(&t));
                }
                Detail::Block { title, rows, diff } => {
                    let body = rows.join("\n");
                    let fence = "`".repeat(longest_run(&body, '`').max(2) + 1);
                    let lang = if diff { "diff" } else { "" };
                    let _ = writeln!(s, "  - {}", escape(&title));
                    let _ = writeln!(s, "\n    {fence}{lang}");
                    for r in rows {
                        let _ = writeln!(s, "    {r}");
                    }
                    let _ = writeln!(s, "    {fence}\n");
                }
            }
        }
    }
    if let Some(l) = data_line(d) {
        let _ = writeln!(s, "\n{}", escape(&l));
    }
    for n in &d.notes {
        let _ = writeln!(s, "\n> {}", escape(n));
    }
    s
}
