//! Typed access to a command's JSON parameters. Wrong types are errors, never silently ignored, so
//! an agent's typo (`formatstring`, `"hidden": "yes"`) is reported instead of dropped.

use serde_json::Value;

/// Rejects parameters that are not an object (`null` counts as `{}`) and keys not in `allowed`.
pub(crate) fn allow(params: &Value, allowed: &[&str]) -> Result<(), String> {
    let obj = match params {
        Value::Null => return Ok(()),
        Value::Object(o) => o,
        _ => return Err("parameters must be a JSON object".into()),
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            let expected = if allowed.is_empty() {
                "none".to_string()
            } else {
                allowed.join(", ")
            };
            return Err(format!("unknown parameter `{key}` (expected: {expected})"));
        }
    }
    Ok(())
}

pub(crate) fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string parameter `{key}`"))
}

/// An optional string; `null` counts as absent.
pub(crate) fn opt_str<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("parameter `{key}` must be a string")),
    }
}

pub(crate) fn opt_bool(params: &Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("parameter `{key}` must be true or false")),
    }
}

/// For `*.set` commands: absent → `None` (unchanged), `null` or `""` → `Some(None)` (cleared),
/// a string → `Some(Some(s))` (set).
pub(crate) fn change_str(params: &Value, key: &str) -> Result<Option<Option<String>>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(s)) if s.is_empty() => Ok(Some(None)),
        Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
        Some(_) => Err(format!("parameter `{key}` must be a string or null")),
    }
}

pub(crate) fn force(params: &Value) -> Result<bool, String> {
    Ok(opt_bool(params, "force")?.unwrap_or(false))
}
