//! The text box editor's document (ROADMAP M11.20): the text as one string (a line per
//! paragraph), a style per character, an alignment and list kind per paragraph, and the dynamic
//! values as tokens `«[Measure]»` in the text. Read from the snapshot's `text.paragraphs` and
//! written back as `textbox.set`'s `paragraphs`, so one committed edit is one command.
//!
//! Pure data and logic: the widget (`rich.rs`) lays it out and edits the string; [`Doc::sync`]
//! follows each edit (the inserted characters take the typing style, a line split keeps its
//! paragraph's alignment and list), [`Doc::apply`] styles a selection.

use serde_json::{Map, Value, json};

/// The delimiters of a dynamic value in the editor's text.
pub const OPEN: char = '\u{ab}';
pub const CLOSE: char = '\u{bb}';
/// Most characters the editor holds (the engine's cap).
pub const MAX_CHARS: usize = 100_000;
/// Most paragraphs read.
const MAX_PARAGRAPHS: usize = 500;

/// A run's style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub family: Option<String>,
    /// Points.
    pub size: Option<f32>,
    /// `#RRGGBB`.
    pub color: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub fn name(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum List {
    #[default]
    None,
    Bullet,
    Number,
}

/// A paragraph's own settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Para {
    pub align: Align,
    pub list: List,
}

/// The editor's document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Doc {
    pub text: String,
    /// One per character of `text` (the `\n` of a line break too).
    pub styles: Vec<Style>,
    /// One per line of `text`.
    pub paras: Vec<Para>,
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool) == Some(true)
}

impl Doc {
    /// An empty document (one empty paragraph).
    pub fn empty() -> Doc {
        Doc {
            text: String::new(),
            styles: Vec::new(),
            paras: vec![Para::default()],
        }
    }

    /// The snapshot's `text` of a text box (`{paragraphs: [{align?, list?, runs: [{text, bold?,
    /// …, field?}]}]}`); a run with a `field` is a dynamic value.
    pub fn from_snapshot(text: &Value) -> Doc {
        let mut d = Doc::default();
        let paras = text
            .get("paragraphs")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for (i, p) in paras.iter().take(MAX_PARAGRAPHS).enumerate() {
            if i > 0 {
                d.text.push('\n');
                d.styles.push(d.styles.last().cloned().unwrap_or_default());
            }
            let align = match s(p, "align") {
                Some("center") => Align::Center,
                Some("right") => Align::Right,
                _ => Align::Left,
            };
            let list = match s(p, "list") {
                Some("bullet") => List::Bullet,
                Some("number") => List::Number,
                _ => List::None,
            };
            d.paras.push(Para { align, list });
            for r in p
                .get("runs")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let style = Style {
                    bold: flag(r, "bold"),
                    italic: flag(r, "italic"),
                    underline: flag(r, "underline"),
                    family: s(r, "font_family").map(str::to_string),
                    size: r
                        .get("font_size")
                        .and_then(Value::as_f64)
                        .filter(|x| x.is_finite() && *x > 0.0)
                        .map(|x| x as f32),
                    color: s(r, "color").map(str::to_string),
                    url: s(r, "url").map(str::to_string),
                };
                let piece: String = match s(r, "field") {
                    Some(f) => format!("{OPEN}{}{CLOSE}", f.replace([OPEN, CLOSE, '\n'], "")),
                    None => s(r, "text").unwrap_or("").replace(['\r'], ""),
                };
                // a line break inside a run is a paragraph of its own here
                for ch in piece.chars() {
                    if d.styles.len() >= MAX_CHARS {
                        break;
                    }
                    if ch == '\n' {
                        d.paras.push(Para { align, list });
                    }
                    d.text.push(ch);
                    d.styles.push(style.clone());
                }
            }
        }
        if d.paras.is_empty() {
            d.paras.push(Para::default());
        }
        d.fix();
        d
    }

    /// Keeps one style per character and one paragraph per line.
    fn fix(&mut self) {
        let n = self.text.chars().count();
        let last = self.styles.last().cloned().unwrap_or_default();
        self.styles.resize(n, last);
        let lines = self.text.matches('\n').count().saturating_add(1);
        let lp = self.paras.last().copied().unwrap_or_default();
        self.paras.resize(lines, lp);
    }

    /// The style a character typed at `at` gets: the one before it, else after it.
    pub fn style_at(&self, at: usize) -> Style {
        at.checked_sub(1)
            .and_then(|i| self.styles.get(i))
            .or_else(|| self.styles.get(at))
            .cloned()
            .unwrap_or_default()
    }

    /// The line of character `at`.
    pub fn line_of(&self, at: usize) -> usize {
        self.text.chars().take(at).filter(|c| *c == '\n').count()
    }

    /// Follows an edit of the text: the characters between the common prefix and suffix are
    /// replaced, the new ones in `typing`; lines split or joined keep their paragraph settings.
    pub fn sync(&mut self, new: &str, typing: &Style) {
        if new == self.text {
            return;
        }
        let old: Vec<char> = self.text.chars().collect();
        let nw: Vec<char> = new.chars().take(MAX_CHARS).collect();
        let pre = old.iter().zip(&nw).take_while(|(a, b)| a == b).count();
        let max_suf = old.len().min(nw.len()).saturating_sub(pre);
        let suf = old
            .iter()
            .rev()
            .zip(nw.iter().rev())
            .take(max_suf)
            .take_while(|(a, b)| a == b)
            .count();
        let removed = old.get(pre..old.len().saturating_sub(suf)).unwrap_or_default();
        let inserted = nw.get(pre..nw.len().saturating_sub(suf)).unwrap_or_default();
        let line = old.iter().take(pre).filter(|c| **c == '\n').count();
        let gone = removed.iter().filter(|c| **c == '\n').count();
        let added = inserted.iter().filter(|c| **c == '\n').count();
        let here = self.paras.get(line).copied().unwrap_or_default();
        let from = line.saturating_add(1).min(self.paras.len());
        let to = from.saturating_add(gone).min(self.paras.len());
        self.paras.splice(from..to, std::iter::repeat_n(here, added));
        let end = old.len().saturating_sub(suf).min(self.styles.len());
        let at = pre.min(end);
        self.styles
            .splice(at..end, std::iter::repeat_n(typing.clone(), inserted.len()));
        self.text = nw.into_iter().collect();
        self.fix();
    }

    /// Changes the style of the characters in `range` (char indices, any order).
    pub fn apply(&mut self, range: (usize, usize), f: impl Fn(&mut Style)) {
        let (a, b) = (range.0.min(range.1), range.0.max(range.1));
        for st in self.styles.iter_mut().take(b).skip(a) {
            f(st);
        }
    }

    /// Changes the paragraphs the range touches.
    pub fn apply_para(&mut self, range: (usize, usize), f: impl Fn(&mut Para)) {
        let (a, b) = (range.0.min(range.1), range.0.max(range.1));
        let (la, lb) = (self.line_of(a), self.line_of(b));
        for p in self.paras.iter_mut().take(lb.saturating_add(1)).skip(la) {
            f(p);
        }
    }

    /// Inserts a dynamic value token at `at`; returns the character after it.
    pub fn insert_value(&mut self, at: usize, field: &str, style: &Style) -> usize {
        let token: String = format!("{OPEN}{}{CLOSE}", field.replace([OPEN, CLOSE, '\n'], ""));
        let n = token.chars().count();
        let mut chars: Vec<char> = self.text.chars().collect();
        let at = at.min(chars.len());
        let tail = chars.split_off(at);
        let new: String = chars.into_iter().chain(token.chars()).chain(tail).collect();
        self.sync(&new, style);
        at.saturating_add(n)
    }

    /// `textbox.set`'s `paragraphs`.
    pub fn to_paragraphs(&self) -> Value {
        let chars: Vec<char> = self.text.chars().collect();
        let mut out = Vec::new();
        let mut i = 0usize;
        for (li, line) in self.text.split('\n').enumerate() {
            let len = line.chars().count();
            let end = i.saturating_add(len);
            let mut runs: Vec<Value> = Vec::new();
            let mut cur: Option<(Style, String)> = None;
            let flush = |cur: &mut Option<(Style, String)>, runs: &mut Vec<Value>| {
                if let Some((st, t)) = cur.take()
                    && !t.is_empty()
                {
                    runs.push(run_json(&st, Some(&t), None));
                }
            };
            let mut k = i;
            while k < end {
                let ch = chars.get(k).copied().unwrap_or(' ');
                let st = self.styles.get(k).cloned().unwrap_or_default();
                if ch == OPEN
                    && let Some(close) = (k.saturating_add(1)..end).find(|j| chars.get(*j) == Some(&CLOSE))
                {
                    let field: String = chars
                        .get(k.saturating_add(1)..close)
                        .unwrap_or_default()
                        .iter()
                        .collect();
                    if !field.trim().is_empty() {
                        flush(&mut cur, &mut runs);
                        runs.push(run_json(&st, None, Some(field.trim())));
                        k = close.saturating_add(1);
                        continue;
                    }
                }
                match &mut cur {
                    Some((s0, t)) if *s0 == st => t.push(ch),
                    _ => {
                        flush(&mut cur, &mut runs);
                        cur = Some((st, ch.to_string()));
                    }
                }
                k = k.saturating_add(1);
            }
            flush(&mut cur, &mut runs);
            let p = self.paras.get(li).copied().unwrap_or_default();
            let mut o = Map::new();
            if p.align != Align::Left {
                o.insert("align".into(), json!(p.align.name()));
            }
            match p.list {
                List::Bullet => {
                    o.insert("list".into(), json!("bullet"));
                }
                List::Number => {
                    o.insert("list".into(), json!("number"));
                }
                List::None => {}
            }
            o.insert("runs".into(), Value::Array(runs));
            out.push(Value::Object(o));
            i = end.saturating_add(1);
        }
        Value::Array(out)
    }
}

fn run_json(st: &Style, text: Option<&str>, field: Option<&str>) -> Value {
    let mut o = Map::new();
    match (text, field) {
        (_, Some(f)) => {
            o.insert("value".into(), json!(f));
        }
        (Some(t), None) => {
            o.insert("text".into(), json!(t));
        }
        (None, None) => {}
    }
    if st.bold {
        o.insert("bold".into(), json!(true));
    }
    if st.italic {
        o.insert("italic".into(), json!(true));
    }
    if st.underline {
        o.insert("underline".into(), json!(true));
    }
    if let Some(f) = &st.family {
        o.insert("font_family".into(), json!(f));
    }
    if let Some(sz) = st.size {
        o.insert("font_size".into(), json!(sz));
    }
    if let Some(c) = &st.color {
        o.insert("color".into(), json!(c));
    }
    if let Some(u) = &st.url {
        o.insert("url".into(), json!(u));
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Value {
        json!({"paragraphs": [
            {"align": "center", "runs": [
                {"text": "Sales ", "bold": true, "font_size": 14},
                {"text": "this year", "italic": true, "color": "#118DFF"}]},
            {"list": "bullet", "runs": [{"text": "Total: "}, {"text": "12", "bound": "value1", "field": "[Total]"}]}
        ]})
    }

    #[test]
    fn snapshot_text_reads_and_writes_back() {
        let d = Doc::from_snapshot(&snap());
        assert_eq!(d.text, "Sales this year\nTotal: «[Total]»");
        assert_eq!(d.styles.len(), d.text.chars().count());
        assert_eq!(
            d.paras,
            vec![
                Para {
                    align: Align::Center,
                    list: List::None
                },
                Para {
                    align: Align::Left,
                    list: List::Bullet
                }
            ]
        );
        assert_eq!(
            d.to_paragraphs(),
            json!([
                {"align": "center", "runs": [
                    {"text": "Sales ", "bold": true, "font_size": 14.0},
                    {"text": "this year", "italic": true, "color": "#118DFF"}]},
                {"list": "bullet", "runs": [{"text": "Total: "}, {"value": "[Total]"}]}
            ])
        );
    }

    #[test]
    fn typing_takes_the_typing_style_and_lines_keep_their_paragraph() {
        let mut d = Doc::from_snapshot(&snap());
        let bold = Style {
            bold: true,
            ..Style::default()
        };
        // a new line in the middle of the first paragraph: both halves stay centred
        d.sync("Sales\n this year\nTotal: «[Total]»", &bold);
        assert_eq!(d.paras.len(), 3);
        assert_eq!(d.paras[1].align, Align::Center);
        assert_eq!(d.paras[2].list, List::Bullet);
        // the lines joined again
        d.sync("Sales this year\nTotal: «[Total]»", &bold);
        assert_eq!(d.paras.len(), 2);
        // typed at the end, in italic
        let it = Style {
            italic: true,
            ..Style::default()
        };
        d.sync("Sales this year\nTotal: «[Total]»!", &it);
        assert_eq!(d.styles.last(), Some(&it));
        // a selection made bold
        let mut d = Doc::from_snapshot(&json!({"paragraphs": [{"runs": [{"text": "abcdef"}]}]}));
        d.apply((4, 2), |s| s.bold = true);
        assert_eq!(
            d.to_paragraphs(),
            json!([{"runs": [{"text": "ab"}, {"text": "cd", "bold": true}, {"text": "ef"}]}])
        );
        d.apply_para((0, 1), |p| p.align = Align::Right);
        assert_eq!(d.to_paragraphs()[0]["align"], "right");
        // everything deleted: one empty paragraph
        d.sync("", &Style::default());
        assert_eq!(d.to_paragraphs(), json!([{"align": "right", "runs": []}]));
    }

    #[test]
    fn values_are_tokens() {
        let mut d = Doc::empty();
        let at = d.insert_value(0, "[Total]", &Style::default());
        assert_eq!(at, 9);
        d.sync("Now «[Total]» and «» and «open", &Style::default());
        assert_eq!(
            d.to_paragraphs(),
            json!([{"runs": [{"text": "Now "}, {"value": "[Total]"}, {"text": " and «» and «open"}]}])
        );
        // hostile snapshot values never panic
        for v in [
            json!(null),
            json!({"paragraphs": 5}),
            json!({"paragraphs": [5, {"runs": "x"}]}),
        ] {
            let d = Doc::from_snapshot(&v);
            assert_eq!(d.paras.len(), d.text.matches('\n').count() + 1);
            let _ = d.to_paragraphs();
        }
    }
}
