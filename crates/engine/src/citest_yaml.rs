//! The YAML reader of `bicraft.tests.yaml` (`project.test`, `docs/ci-tests.md`): YAML text to a
//! JSON value, over `yaml-rust2`'s event parser so that the tree is built here, under limits.
//!
//! Test files are input like any other, so the reader never trusts them:
//!
//! - the text is capped at [`MAX_TEXT_BYTES`], the tree at [`MAX_DEPTH`] levels and
//!   [`MAX_NODES`] nodes;
//! - anchors and aliases are refused (an alias copies a subtree: a few lines could expand to
//!   gigabytes), and so are tags other than the core ones and multi-document streams;
//! - duplicate keys are errors, never "last one wins".
//!
//! Plain scalars follow the YAML 1.2 core schema: `null`/`~`/empty, `true`/`false`, integers and
//! finite floats; everything else (and every quoted scalar) is a string.

use serde_json::{Map, Number, Value};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// Largest test file read, in bytes.
pub(crate) const MAX_TEXT_BYTES: usize = 4 << 20;
/// Deepest nesting of mappings and sequences.
pub(crate) const MAX_DEPTH: usize = 64;
/// Most nodes (scalars, mappings, sequences) in one file.
pub(crate) const MAX_NODES: usize = 200_000;

enum Frame {
    Seq(Vec<Value>),
    /// The mapping so far and the key waiting for its value.
    Map(Map<String, Value>, Option<String>),
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    root: Option<Value>,
    nodes: usize,
    error: Option<String>,
}

impl Builder {
    fn fail(&mut self, mark: Marker, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(format!(
                "line {}, column {}: {}",
                mark.line(),
                mark.col().saturating_add(1),
                message.into()
            ));
        }
    }

    fn count(&mut self, mark: Marker) -> bool {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            self.fail(mark, format!("more than {MAX_NODES} values"));
            return false;
        }
        true
    }

    fn push(&mut self, v: Value, mark: Marker) {
        match self.stack.last_mut() {
            None => {
                if self.root.is_some() {
                    self.fail(mark, "one YAML document only");
                } else {
                    self.root = Some(v);
                }
            }
            Some(Frame::Seq(items)) => items.push(v),
            Some(Frame::Map(map, key)) => match key.take() {
                None => {
                    let k = match v {
                        Value::String(s) => s,
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => String::new(),
                        _ => {
                            self.fail(mark, "a mapping key must be a scalar");
                            return;
                        }
                    };
                    if map.contains_key(&k) {
                        self.fail(mark, format!("duplicate key `{k}`"));
                        return;
                    }
                    *key = Some(k);
                }
                Some(k) => {
                    map.insert(k, v);
                }
            },
        }
    }

    fn open(&mut self, frame: Frame, anchor: usize, mark: Marker) {
        if anchor != 0 {
            self.fail(mark, "anchors and aliases are not supported");
            return;
        }
        if !self.count(mark) {
            return;
        }
        if self.stack.len() >= MAX_DEPTH {
            self.fail(mark, format!("nested deeper than {MAX_DEPTH} levels"));
            return;
        }
        self.stack.push(frame);
    }

    fn close(&mut self, mark: Marker) {
        match self.stack.pop() {
            Some(Frame::Seq(items)) => self.push(Value::Array(items), mark),
            Some(Frame::Map(map, None)) => self.push(Value::Object(map), mark),
            Some(Frame::Map(_, Some(k))) => self.fail(mark, format!("key `{k}` has no value")),
            None => self.fail(mark, "unbalanced document"),
        }
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
            Event::Alias(_) => self.fail(mark, "anchors and aliases are not supported"),
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 {
                    self.fail(mark, "anchors and aliases are not supported");
                    return;
                }
                if !self.count(mark) {
                    return;
                }
                let v = match tag {
                    Some(t) if t.handle == "tag:yaml.org,2002:" || t.handle == "!!" => {
                        match t.suffix.as_str() {
                            "str" => Value::String(text),
                            "null" | "bool" | "int" | "float" => plain(text),
                            other => {
                                self.fail(mark, format!("unsupported tag `!!{other}`"));
                                return;
                            }
                        }
                    }
                    Some(t) => {
                        self.fail(mark, format!("unsupported tag `{}{}`", t.handle, t.suffix));
                        return;
                    }
                    None if style == TScalarStyle::Plain => plain(text),
                    None => Value::String(text),
                };
                self.push(v, mark);
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) if tag.is_some() => {
                let _ = anchor;
                self.fail(mark, "tags on mappings and sequences are not supported");
            }
            Event::SequenceStart(anchor, _) => self.open(Frame::Seq(Vec::new()), anchor, mark),
            Event::MappingStart(anchor, _) => self.open(Frame::Map(Map::new(), None), anchor, mark),
            Event::SequenceEnd | Event::MappingEnd => self.close(mark),
        }
    }
}

/// A plain scalar under the core schema.
fn plain(text: String) -> Value {
    match text.as_str() {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let numeric = text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'))
        && text.bytes().any(|b| b.is_ascii_digit());
    if numeric {
        let t = text.strip_prefix('+').unwrap_or(&text);
        if let Ok(i) = t.parse::<i64>() {
            return Value::Number(i.into());
        }
        if let Some(n) = t.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(text)
}

/// Parses YAML text into a JSON value (an empty document is `null`).
pub(crate) fn parse(text: &str) -> Result<Value, String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!("larger than {MAX_TEXT_BYTES} bytes"));
    }
    let text = text.trim_start_matches('\u{feff}');
    let mut b = Builder::default();
    // The parser is third-party code: a panic in it is reported as a parse error, never a crash.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut parser = Parser::new_from_str(text);
        parser.load(&mut b, true).map_err(|e| e.to_string())
    }));
    match outcome {
        Err(_) => return Err("the YAML parser failed on this text".into()),
        Ok(Err(e)) => return Err(e),
        Ok(Ok(())) => {}
    }
    if let Some(e) = b.error {
        return Err(e);
    }
    if !b.stack.is_empty() {
        return Err("the document ends inside a mapping or sequence".into());
    }
    Ok(b.root.unwrap_or(Value::Null))
}
