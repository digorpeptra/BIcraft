//! [`check_value`](crate::check_value): a PBIR property value against the property's type.
//!
//! PBIR writes a property value as `{"expr": <expression>}` (a `Literal` for plain values, a
//! field or rule expression for conditional formatting) or, for colors, `{"solid": {"color":
//! {"expr": …}}}`. Literals are `true`/`false`, numbers with a type suffix (`12D`, `3L`, `2.5M`)
//! and single-quoted text (`'Bottom'`, `''` escaping a quote).

use serde_json::{Map, Value};

use crate::{PropType, Property, suggest};

/// Expression kinds a conditional (data-bound) value may use.
const CONDITIONAL_EXPRS: &[&str] = &[
    "Measure",
    "Column",
    "Aggregation",
    "HierarchyLevel",
    "Conditional",
    "FillRule",
    "SelectRef",
    "ThemeDataColor",
    "ResourcePackageItem",
    "Arithmetic",
    "Transform",
];

/// Expression kinds that name a field.
const FIELD_EXPRS: &[&str] = &["Measure", "Column", "Aggregation", "HierarchyLevel", "SelectRef"];

/// Longest literal checked (longer text is still accepted for text properties).
const MAX_LITERAL: usize = 1_000_000;

pub(crate) fn check_value(p: &Property, value: &Value) -> Result<(), String> {
    if matches!(p.ty, PropType::Other { .. }) {
        return Ok(());
    }
    let name = &p.name;
    let Some(o) = value.as_object() else {
        return Err(format!(
            "`{name}` expects {}, written as a PBIR value ({{\"expr\": …}}); got {}",
            p.ty.describe(),
            short(value)
        ));
    };
    if o.len() == 1
        && let Some(expr) = o.get("expr")
    {
        return check_expr(p, expr);
    }
    if o.len() == 1
        && let Some(solid) = o.get("solid")
    {
        return match p.ty {
            PropType::Color => check_solid(p, solid),
            _ => Err(format!(
                "`{name}` expects {}, not a color ({{\"solid\": …}})",
                p.ty.describe()
            )),
        };
    }
    if matches!(p.ty, PropType::Image) && o.len() == 1 && o.get("image").is_some_and(Value::is_object) {
        return Ok(());
    }
    Err(format!(
        "`{name}` expects {}; {} is not a PBIR property value ({{\"expr\": …}}{})",
        p.ty.describe(),
        short(value),
        if matches!(p.ty, PropType::Color) {
            " or {\"solid\": {\"color\": …}}"
        } else {
            ""
        }
    ))
}

/// The single key and value of an expression object.
fn expr_kind(expr: &Value) -> Option<(&str, &Value)> {
    let o = expr.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.iter().next().map(|(k, v)| (k.as_str(), v))
}

fn literal_text(inner: &Value) -> Option<&str> {
    let l = inner.as_object()?;
    if l.len() != 1 {
        return None;
    }
    l.get("Value")?.as_str()
}

fn check_expr(p: &Property, expr: &Value) -> Result<(), String> {
    let name = &p.name;
    let Some((kind, inner)) = expr_kind(expr) else {
        return Err(format!(
            "`{name}`: an expression is an object with one key (Literal, Measure, …); got {}",
            short(expr)
        ));
    };
    if kind == "Literal" {
        let text = literal_text(inner).ok_or_else(|| {
            format!(
                "`{name}`: a Literal is {{\"Value\": \"…\"}}; got {}",
                short(inner)
            )
        })?;
        return check_literal(p, text);
    }
    match &p.ty {
        PropType::Color => Err(format!(
            "`{name}` is a color: write {{\"solid\": {{\"color\": {{\"expr\": {{\"{kind}\": …}}}}}}}}, not {{\"expr\": …}}"
        )),
        PropType::Field if FIELD_EXPRS.contains(&kind) => Ok(()),
        PropType::Field => Err(format!(
            "`{name}` expects a field (Column, Measure, Aggregation…); got a {kind} expression"
        )),
        PropType::Image => Ok(()),
        _ if !p.conditional => Err(format!(
            "`{name}` takes a literal ({}), not a {kind} expression (it has no conditional formatting)",
            p.ty.describe()
        )),
        _ if CONDITIONAL_EXPRS.contains(&kind) => Ok(()),
        _ => Err(format!(
            "`{name}`: unknown expression kind `{kind}` (expected a Literal, or one of {})",
            CONDITIONAL_EXPRS.join(", ")
        )),
    }
}

/// A numeric literal: value and suffix (`L`, `D`, `M`, or none).
fn parse_number(text: &str) -> Option<(f64, Option<char>)> {
    let (body, suffix) = match text.chars().last() {
        Some(c @ ('L' | 'D' | 'M')) => (text.get(..text.len().saturating_sub(1))?, Some(c)),
        _ => (text, None),
    };
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
    {
        return None;
    }
    body.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| (n, suffix))
}

/// The text of a single-quoted literal (`'it''s'` → `it's`).
fn parse_text(text: &str) -> Option<String> {
    if text.len() < 2 {
        return None;
    }
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    // Inside, quotes come in pairs.
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' && chars.next() != Some('\'') {
            return None;
        }
        out.push(c);
    }
    Some(out)
}

fn num_text(n: f64) -> String {
    format!("{n}")
}

fn check_literal(p: &Property, text: &str) -> Result<(), String> {
    let name = &p.name;
    if text.len() > MAX_LITERAL {
        return Err(format!("`{name}`: literal longer than {MAX_LITERAL} bytes"));
    }
    let got = short_str(text);
    match &p.ty {
        PropType::Bool => match text {
            "true" | "false" => Ok(()),
            _ => Err(format!("`{name}` expects true or false; got {got}")),
        },
        PropType::Number { min, max, integer } => {
            let (n, _) = parse_number(text).ok_or_else(|| {
                format!(
                    "`{name}` expects {} (a literal like 12D{}); got {got}",
                    p.ty.describe(),
                    if *integer { " or 3L" } else { "" }
                )
            })?;
            if *integer && n.fract() != 0.0 {
                return Err(format!("`{name}` expects a whole number; got {got}"));
            }
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(format!("`{name}` expects {}; got {got}", p.ty.describe()));
            }
            Ok(())
        }
        PropType::Text | PropType::Font | PropType::Image => match parse_text(text) {
            Some(_) => Ok(()),
            None => Err(format!(
                "`{name}` expects {} written as a quoted literal ('like this'); got {got}",
                p.ty.describe()
            )),
        },
        PropType::Enum { values } => {
            let numeric = !values.is_empty() && values.iter().all(|v| v.parse::<f64>().is_ok());
            let ok = if numeric {
                parse_number(text)
                    .is_some_and(|(n, _)| values.iter().any(|v| v.parse::<f64>().ok() == Some(n)))
            } else {
                parse_text(text).is_some_and(|t| values.contains(&t))
            };
            if ok {
                return Ok(());
            }
            let hint = if numeric {
                String::new()
            } else {
                parse_text(text)
                    .and_then(|t| {
                        suggest(&t, values.iter().map(String::as_str))
                            .map(|s| format!(" (did you mean '{s}'?)"))
                    })
                    .unwrap_or_default()
            };
            Err(format!("`{name}` expects {}; got {got}{hint}", p.ty.describe()))
        }
        PropType::Color => Err(format!(
            "`{name}` is a color: write {{\"solid\": {{\"color\": {{\"expr\": {{\"Literal\": {{\"Value\": \"'#RRGGBB'\"}}}}}}}}}}; got the literal {got}"
        )),
        PropType::Field => Err(format!(
            "`{name}` expects a field reference, not the literal {got}"
        )),
        PropType::Other { .. } => Ok(()),
    }
}

fn is_hex_color(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn check_solid(p: &Property, solid: &Value) -> Result<(), String> {
    let name = &p.name;
    let color = solid
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get("color"))
        .ok_or_else(|| {
            format!(
                "`{name}`: a solid color is {{\"solid\": {{\"color\": …}}}}; got {}",
                short(solid)
            )
        })?;
    let expr = color
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get("expr"))
        .ok_or_else(|| format!("`{name}`: the color is {{\"expr\": …}}; got {}", short(color)))?;
    let Some((kind, inner)) = expr_kind(expr) else {
        return Err(format!(
            "`{name}`: the color expression has one key; got {}",
            short(expr)
        ));
    };
    match kind {
        "Literal" => {
            let text = literal_text(inner).ok_or_else(|| {
                format!(
                    "`{name}`: a Literal is {{\"Value\": \"…\"}}; got {}",
                    short(inner)
                )
            })?;
            match parse_text(text) {
                Some(c) if is_hex_color(&c) => Ok(()),
                _ => Err(format!(
                    "`{name}` expects a color '#RGB', '#RRGGBB' or '#RRGGBBAA'; got {}",
                    short_str(text)
                )),
            }
        }
        "ThemeDataColor" => check_theme_color(name, inner),
        _ if !p.conditional => Err(format!(
            "`{name}` has no conditional formatting; got a {kind} color"
        )),
        _ if CONDITIONAL_EXPRS.contains(&kind) => Ok(()),
        _ => Err(format!(
            "`{name}`: unknown color expression `{kind}` (expected Literal, ThemeDataColor, or one of {})",
            CONDITIONAL_EXPRS.join(", ")
        )),
    }
}

/// `{"ColorId": n ≥ 0, "Percent": -1..1}` (Percent optional).
fn check_theme_color(name: &str, inner: &Value) -> Result<(), String> {
    let empty = Map::new();
    let o = inner.as_object().unwrap_or(&empty);
    let id_ok = o
        .get("ColorId")
        .and_then(Value::as_f64)
        .is_some_and(|n| n >= 0.0 && n.fract() == 0.0);
    let pct_ok = o
        .get("Percent")
        .is_none_or(|p| p.as_f64().is_some_and(|n| (-1.0..=1.0).contains(&n)));
    let keys_ok = o.keys().all(|k| k == "ColorId" || k == "Percent");
    if inner.is_object() && id_ok && pct_ok && keys_ok {
        Ok(())
    } else {
        Err(format!(
            "`{name}`: a theme color is {{\"ThemeDataColor\": {{\"ColorId\": n (0 or more), \"Percent\": -1 to 1}}}}; got {}",
            short(inner)
        ))
    }
}

fn short(v: &Value) -> String {
    short_str(&v.to_string())
}

fn short_str(s: &str) -> String {
    if s.chars().count() > 60 {
        let head: String = s.chars().take(60).collect();
        format!("{head}…")
    } else {
        s.to_string()
    }
}

impl PropType {
    /// What the type accepts, for messages: "a number from 8 to 60", "one of 'Top', 'Bottom'"…
    pub fn describe(&self) -> String {
        match self {
            Self::Bool => "true or false".into(),
            Self::Number { min, max, integer } => {
                let what = if *integer { "a whole number" } else { "a number" };
                match (min, max) {
                    (Some(a), Some(b)) => format!("{what} from {} to {}", num_text(*a), num_text(*b)),
                    (Some(a), None) => format!("{what} of at least {}", num_text(*a)),
                    (None, Some(b)) => format!("{what} of at most {}", num_text(*b)),
                    (None, None) => what.into(),
                }
            }
            Self::Text => "a text".into(),
            Self::Color => "a color".into(),
            Self::Enum { values } => {
                let numeric = values.iter().all(|v| v.parse::<f64>().is_ok());
                let list: Vec<String> = values
                    .iter()
                    .map(|v| if numeric { v.clone() } else { format!("'{v}'") })
                    .collect();
                format!("one of {}", list.join(", "))
            }
            Self::Font => "a font family".into(),
            Self::Image => "an image".into(),
            Self::Field => "a field".into(),
            Self::Other { description } => description.clone(),
        }
    }
}
