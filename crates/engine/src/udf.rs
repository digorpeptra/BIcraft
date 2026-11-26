//! `function.add`, `function.set`, `function.rename`, `function.remove`: the model's DAX
//! user-defined functions (TMSL/TMDL `functions`, kept in the model's unmodelled properties so
//! TMDL, TMSL and PBIX keep them as they are). The rules come from the public "DAX user-defined
//! functions" page of the DAX reference; `docs/formats/dax-udf.md` lists them.
//!
//! A function is stored as `{name, description?, expression, isHidden?, lineageTag}`, where
//! `expression` is `( params ) => body`. Its definition is validated as the DAX query
//! `DEFINE FUNCTION name = expression EVALUATE {0}` against the model after the change
//! ([`bicraft_dax::udf::validate_function`]); measures and other DAX that call it are checked
//! for arity and argument kinds by the binder, and are reported as `broken` when a change breaks
//! them.

use bicraft_dax::udf::{self, UdfParam};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::commands::model;
use crate::dax_rename::{model_sites, refusal, rewrite_sites};
use crate::edit::{self, Edit, apply};
use crate::params::{allow, change_str, force, opt_bool, opt_str, str_param};

/// The compatibility level user-defined functions need (Learn: "DAX UDFs require database
/// compatibility level 1702 or higher").
pub(crate) const UDF_COMPATIBILITY_LEVEL: u32 = 1702;
/// Most parameters a function takes (Learn: between 0 and 256).
const MAX_PARAMS: usize = 256;
/// Longest function name, in characters.
const MAX_NAME_CHARS: usize = 1024;

fn functions(m: &bicraft_model::SemanticModel) -> &[Value] {
    m.extra
        .get("functions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn name_of(f: &Value) -> &str {
    f.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn position(m: &bicraft_model::SemanticModel, name: &str) -> Result<usize, String> {
    functions(m)
        .iter()
        .position(|f| name_of(f).eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let names: Vec<&str> = functions(m).iter().map(name_of).take(20).collect();
            if names.is_empty() {
                format!("the model has no user-defined function `{name}` (it has none)")
            } else {
                format!(
                    "the model has no user-defined function `{name}` (it has {})",
                    names.join(", ")
                )
            }
        })
}

/// Learn's naming rules: letters, digits and underscores, with periods for namespaces (not
/// first, last or doubled); not a built-in function or a reserved word.
pub(crate) fn check_function_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a function name must not be empty".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("a function name is at most {MAX_NAME_CHARS} characters"));
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return Err(format!(
            "`{name}`: a function name has only letters, digits, underscores and periods (no spaces)"
        ));
    }
    if name.starts_with('.') || name.ends_with('.') || name.contains("..") {
        return Err(format!(
            "`{name}`: a function name cannot start or end with a period or have consecutive periods"
        ));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("`{name}`: a function name cannot start with a digit"));
    }
    if bicraft_dax::lookup_function(name).is_some() || bicraft_dax::signature(name).is_some() {
        return Err(format!("`{name}` is a built-in DAX function"));
    }
    let reserved = [
        "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "FUNCTION", "VAR", "RETURN",
    ];
    if reserved.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        return Err(format!("`{name}` is a reserved word"));
    }
    Ok(())
}

/// The `parameters` param: `[{name, type?, subtype?, mode?, default?}]`.
fn parameters(p: &Value) -> Result<Option<Vec<UdfParam>>, String> {
    let list = match p.get("parameters") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err("`parameters` must be a list of {name, type?, subtype?, mode?, default?}".into());
        }
    };
    if list.len() > MAX_PARAMS {
        return Err(format!("a function takes at most {MAX_PARAMS} parameters"));
    }
    let mut out: Vec<UdfParam> = Vec::new();
    for item in list {
        let (name, o) = match item {
            Value::String(s) => (s.as_str(), None),
            Value::Object(o) => (
                o.get("name")
                    .and_then(Value::as_str)
                    .ok_or("each parameter needs a `name`")?,
                Some(o),
            ),
            _ => return Err("each parameter is a name or {name, type?, subtype?, mode?, default?}".into()),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(format!(
                "`{name}`: a parameter name has only letters, digits and underscores"
            ));
        }
        if out.iter().any(|q| q.name.eq_ignore_ascii_case(name)) {
            return Err(format!("parameter `{name}` is given twice"));
        }
        let field = |k: &str| -> Result<Option<&str>, String> {
            match o.and_then(|o| o.get(k)) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.as_str())),
                Some(_) => Err(format!("parameter `{name}`: `{k}` must be a string")),
            }
        };
        out.push(udf::param(
            name,
            field("type")?,
            field("subtype")?,
            field("mode")?,
            field("default")?,
        )?);
    }
    Ok(Some(out))
}

/// `{name, expression, parameters: [{name, type, subtype?, mode, default?, hints}], body}` of a
/// definition (parameters and body only when it parses).
pub(crate) fn describe(name: &str, expression: &str) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(name));
    o.insert("expression".into(), json!(expression));
    if let Ok(sig) = udf::parse_function(expression) {
        o.insert("parameters".into(), json!(sig.params));
        o.insert("body".into(), json!(sig.body));
    }
    Value::Object(o)
}

/// The expression a call's params give: `expression`, or `body` with `parameters` (either part
/// left out keeps the `current` definition's).
fn new_expression(p: &Value, current: Option<&str>) -> Result<Option<String>, String> {
    let expression = opt_str(p, "expression")?;
    let body = opt_str(p, "body")?;
    let params = parameters(p)?;
    if expression.is_some() && (body.is_some() || params.is_some()) {
        return Err("give either `expression` or `body`/`parameters`, not both".into());
    }
    if let Some(e) = expression {
        if e.trim().is_empty() {
            return Err("`expression` must not be empty".into());
        }
        return Ok(Some(e.trim().to_string()));
    }
    if body.is_none() && params.is_none() {
        return Ok(None);
    }
    let old = current.map(udf::parse_function).transpose().map_err(|d| {
        format!(
            "the function's current definition does not parse ({}): give the whole `expression`",
            d.first().map(|d| d.message.as_str()).unwrap_or_default()
        )
    })?;
    let body = match body {
        Some(b) if b.trim().is_empty() => return Err("`body` must not be empty".into()),
        Some(b) => b.to_string(),
        None => old.as_ref().map(|s| s.body.clone()).ok_or("`body` is required")?,
    };
    let params = match params {
        Some(ps) => ps,
        None => old.map(|s| s.params).unwrap_or_default(),
    };
    Ok(Some(udf::compose(&params, &body)))
}

fn raise_level(m: &mut bicraft_model::SemanticModel, notes: &mut Vec<String>) {
    match m.compatibility_level {
        Some(l) if l < UDF_COMPATIBILITY_LEVEL => {
            m.compatibility_level = Some(UDF_COMPATIBILITY_LEVEL);
            notes.push(format!(
                "the compatibility level was raised from {l} to {UDF_COMPATIBILITY_LEVEL} (user-defined functions need it)"
            ));
        }
        None => m.compatibility_level = Some(UDF_COMPATIBILITY_LEVEL),
        Some(_) => {}
    }
}

fn no_cycle(m: &bicraft_model::SemanticModel, name: &str, expression: &str) -> Result<(), String> {
    match udf::call_cycle(m, name, expression) {
        Some(c) => Err(format!(
            "user-defined functions cannot be recursive: {} calls itself through {}",
            name,
            c.join(" → ")
        )),
        None => Ok(()),
    }
}

/// `function.add {name, expression? | parameters? + body?, description?, hidden?, force?}`.
pub(crate) fn function_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "expression",
            "parameters",
            "body",
            "description",
            "hidden",
            "force",
        ],
    )?;
    let name = str_param(p, "name")?;
    check_function_name(name)?;
    let expression = new_expression(p, None)?.ok_or("`expression` (or `body`) is required")?;
    let description = opt_str(p, "description")?;
    let hidden = opt_bool(p, "hidden")?;
    let o = model(s)?;
    if o.tables.is_empty() {
        return Err("user-defined functions need a model with at least one table".into());
    }
    if functions(o).iter().any(|f| name_of(f).eq_ignore_ascii_case(name)) {
        return Err(format!("the model already has a user-defined function `{name}`"));
    }
    apply(s, p, |m| {
        no_cycle(m, name, &expression)?;
        let mut notes = Vec::new();
        raise_level(m, &mut notes);
        let mut f = Map::new();
        f.insert("name".into(), json!(name));
        if let Some(d) = description.filter(|d| !d.is_empty()) {
            f.insert("description".into(), json!(d));
        }
        f.insert("expression".into(), json!(expression));
        if hidden == Some(true) {
            f.insert("isHidden".into(), json!(true));
        }
        f.insert("lineageTag".into(), json!(edit::new_guid()));
        match m.extra.get_mut("functions") {
            Some(Value::Array(a)) => a.push(Value::Object(f)),
            _ => {
                m.extra
                    .insert("functions".into(), Value::Array(vec![Value::Object(f)]));
            }
        }
        let mut e = Edit::new(json!({"function": describe(name, &expression)}));
        e.notes.extend(notes);
        e.check_function(name, &expression);
        Ok(e)
    })
}

/// `function.set {name, expression? | parameters? / body?, description?, hidden?, force?}`.
pub(crate) fn function_set(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(
        p,
        &[
            "name",
            "expression",
            "parameters",
            "body",
            "description",
            "hidden",
            "force",
        ],
    )?;
    let name = str_param(p, "name")?;
    let description = change_str(p, "description")?;
    let hidden = opt_bool(p, "hidden")?;
    let current = {
        let m = model(s)?;
        let i = position(m, name)?;
        functions(m)
            .get(i)
            .and_then(|f| f.get("expression"))
            .and_then(udf::expression_text)
    };
    let expression = new_expression(p, current.as_deref())?;
    if expression.is_none() && description.is_none() && hidden.is_none() {
        return Err("function.set needs at least one property to change".into());
    }
    apply(s, p, |m| {
        let i = position(m, name)?;
        if let Some(e) = &expression {
            no_cycle(m, name, e)?;
        }
        let f = m
            .extra
            .get_mut("functions")
            .and_then(|f| f.get_mut(i))
            .and_then(Value::as_object_mut)
            .ok_or("internal: function index out of range")?;
        let stored = f.get("name").and_then(Value::as_str).unwrap_or(name).to_string();
        if let Some(e) = &expression {
            f.insert("expression".into(), json!(e));
        }
        match description {
            Some(Some(d)) if !d.is_empty() => {
                f.insert("description".into(), json!(d));
            }
            Some(_) => {
                f.shift_remove("description");
            }
            None => {}
        }
        match hidden {
            Some(true) => {
                f.insert("isHidden".into(), json!(true));
            }
            Some(false) => {
                f.shift_remove("isHidden");
            }
            None => {}
        }
        let text = f
            .get("expression")
            .and_then(udf::expression_text)
            .unwrap_or_default();
        let mut e = Edit::new(json!({"function": describe(&stored, &text)})).breaking();
        if let Some(x) = &expression {
            e.check_function(&stored, x);
        }
        Ok(e)
    })
}

/// `function.rename {name, new_name}`: the calls in the model's DAX follow.
pub(crate) fn function_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?;
    check_function_name(new_name)?;
    apply(s, p, |m| {
        let i = position(m, name)?;
        let old = functions(m).get(i).map(name_of).unwrap_or(name).to_string();
        if let Some(j) = functions(m)
            .iter()
            .position(|f| name_of(f).eq_ignore_ascii_case(new_name))
            && j != i
        {
            return Err(format!(
                "the model already has a user-defined function `{new_name}`"
            ));
        }
        let rewritten = rewrite_sites(m, |site| crate::import::rename_calls(site.text(), &old, new_name));
        let f = m
            .extra
            .get_mut("functions")
            .and_then(|f| f.get_mut(i))
            .and_then(Value::as_object_mut)
            .ok_or("internal: function index out of range")?;
        f.insert("name".into(), json!(new_name));
        Ok(Edit::new(json!({"function": new_name, "renamed": old, "calls_rewritten": rewritten})).breaking())
    })
}

/// `text` with the model's user-defined function calls expanded, for the evaluator
/// ([`udf::inline_calls`]); the text itself when it calls none or the expansion fails (the
/// evaluator then reports the call as unsupported).
pub(crate) fn expand_calls<'t>(m: &bicraft_model::SemanticModel, text: &'t str) -> std::borrow::Cow<'t, str> {
    match udf::inline_calls(text, m) {
        Ok(Some(t)) => std::borrow::Cow::Owned(t),
        Ok(None) | Err(_) => std::borrow::Cow::Borrowed(text),
    }
}

/// The labels of the model's DAX that call the function `name`.
fn callers(m: &bicraft_model::SemanticModel, name: &str) -> Vec<String> {
    model_sites(m)
        .iter()
        .filter(|site| {
            !matches!(site.owner(), crate::dax_rename::SiteOwner::Function(n) if n.eq_ignore_ascii_case(name))
        })
        .filter(|site| {
            udf::called_functions(m, site.text())
                .iter()
                .any(|c| c.eq_ignore_ascii_case(name))
        })
        .map(crate::dax_rename::DaxSite::label)
        .collect()
}

/// `function.remove {name, force?}`: refused while DAX calls it, unless `force`.
pub(crate) fn function_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "force"])?;
    let name = str_param(p, "name")?;
    let used_by = {
        let m = model(s)?;
        let i = position(m, name)?;
        let stored = functions(m).get(i).map(name_of).unwrap_or(name).to_string();
        let used = callers(m, &stored);
        if !used.is_empty() && !force(p)? {
            return Err(refusal(&format!("function `{stored}`"), &used));
        }
        used
    };
    apply(s, p, |m| {
        let i = position(m, name)?;
        let removed = match m.extra.get_mut("functions") {
            Some(Value::Array(a)) if i < a.len() => a.remove(i),
            _ => return Err("internal: function index out of range".into()),
        };
        if functions(m).is_empty() {
            m.extra.shift_remove("functions");
        }
        Ok(Edit::new(json!({"removed": name_of(&removed), "used_by": used_by})).breaking())
    })
}
