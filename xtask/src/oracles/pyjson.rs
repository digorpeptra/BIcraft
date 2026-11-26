//! JSON text laid out like Python's `json.dumps` (the layout of the files the oracle tools wrote
//! before they moved to Rust), so regenerated files stay byte-identical to the committed ones
//! and to the local oracle sets: `", "` / `": "` separators, optional indentation, `\uXXXX`
//! escapes of non-ASCII text when `ascii` is set, and Python's `repr` of floats.

use std::fmt::Write;

use serde_json::Value;

/// `json.dumps(v, indent=indent, ensure_ascii=ascii)`.
pub fn dumps(v: &Value, indent: Option<usize>, ascii: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, v, indent, ascii, 0);
    out
}

fn newline(out: &mut String, indent: usize, level: usize) {
    out.push('\n');
    for _ in 0..indent.saturating_mul(level) {
        out.push(' ');
    }
}

fn write_value(out: &mut String, v: &Value, indent: Option<usize>, ascii: bool, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                out.push_str(&float_repr(n.as_f64().unwrap_or(0.0)));
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => write_str(out, s, ascii),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                sep(out, i, indent, level);
                write_value(out, x, indent, ascii, level.saturating_add(1));
            }
            close(out, indent, level);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, x)) in m.iter().enumerate() {
                sep(out, i, indent, level);
                write_str(out, k, ascii);
                out.push_str(": ");
                write_value(out, x, indent, ascii, level.saturating_add(1));
            }
            close(out, indent, level);
            out.push('}');
        }
    }
}

fn sep(out: &mut String, i: usize, indent: Option<usize>, level: usize) {
    match indent {
        Some(n) => {
            if i > 0 {
                out.push(',');
            }
            newline(out, n, level.saturating_add(1));
        }
        None => {
            if i > 0 {
                out.push_str(", ");
            }
        }
    }
}

fn close(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent {
        newline(out, n, level);
    }
}

fn write_str(out: &mut String, s: &str, ascii: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (ascii && !(' '..='~').contains(&c)) => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{u:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Python's `repr(float)`: the shortest round-trip digits, positional for decimal exponents
/// -4..16, else `d.ddde+XX`.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // `{:e}` gives the shortest round-trip digits: "-1.2345e3", "5e-5".
    let sci = format!("{x:e}");
    let (mant, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (neg, mant) = match mant.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mant),
    };
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        if exp < 0 {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else if exp + 1 >= n {
            out.push_str(&digits);
            for _ in 0..(exp + 1 - n) {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            let (a, b) = digits.split_at((exp + 1) as usize);
            out.push_str(a);
            out.push('.');
            out.push_str(b);
        }
    } else {
        let (a, b) = digits.split_at(1.min(digits.len()));
        out.push_str(a);
        if !b.is_empty() {
            out.push('.');
            out.push_str(b);
        }
        let _ = write!(out, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    out
}

/// Python's `round(x, 2)`: the correctly rounded two-decimal value.
pub fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_repr_matches_python() {
        for (x, s) in [
            (0.52, "0.52"),
            (0.7, "0.7"),
            (5000.0, "5000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (123456789012345.6, "123456789012345.6"),
            (-2.5, "-2.5"),
            (0.0, "0.0"),
            (100.0, "100.0"),
        ] {
            assert_eq!(float_repr(x), s, "{x}");
        }
        assert_eq!(round2(5123.456), 5123.46);
        assert_eq!(float_repr(round2(3176.4000000001)), "3176.4");
    }

    #[test]
    fn dumps_layouts() {
        let v = json!({"a": [1, 2.5, {}], "b": "é\n\u{1f}", "c": [], "d": true, "e": null});
        assert_eq!(
            dumps(&v, None, false),
            "{\"a\": [1, 2.5, {}], \"b\": \"é\\n\\u001f\", \"c\": [], \"d\": true, \"e\": null}"
        );
        assert_eq!(
            dumps(&v, Some(1), true),
            "{\n \"a\": [\n  1,\n  2.5,\n  {}\n ],\n \"b\": \"\\u00e9\\n\\u001f\",\n \"c\": [],\n \"d\": true,\n \"e\": null\n}"
        );
        assert_eq!(
            dumps(&json!("✓😀\u{7f}"), None, true),
            "\"\\u2713\\ud83d\\ude00\\u007f\""
        );
    }
}
