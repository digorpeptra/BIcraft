//! DAX function catalog: a typed [`Signature`] for every function of the public DAX reference
//! (plus the few the engine exposes without a reference page), in `catalog/`. Argument counts,
//! column-declaring functions and keyword arguments are all derived from the signatures. See
//! `docs/formats/dax-functions.md` for the conventions and how the catalog was checked against the
//! engine.
//!
//! The catalog holds every DAX function of the engine it was checked against, so against a model
//! an unknown function is an error (the engine's "failed to resolve name"), unless the model
//! defines a user-defined function of that name. Without a model it is only a warning.

use serde::Serialize;

use crate::types::{ParamKind, Signature};

#[path = "catalog/mod.rs"]
mod catalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Arity {
    pub min: u8,
    /// `None` means variadic.
    pub max: Option<u8>,
}

/// Full typed signature of a function, looked up case-insensitively. Frozen API: the type checker
/// calls it.
pub fn signature(name: &str) -> Option<&'static Signature> {
    catalog::find(name)
}

/// Looks up a function's argument count, case-insensitively.
pub fn lookup(name: &str) -> Option<Arity> {
    signature(name).map(arity)
}

/// The argument count a signature allows: every required param once (a mandatory repeating unit
/// counts once), and no upper bound when the signature repeats.
pub(crate) fn arity(sig: &Signature) -> Arity {
    let required = sig.params.iter().filter(|p| !p.optional).count();
    let any_repeating = sig.repeat_from.is_some() || sig.params.iter().any(|p| p.repeating);
    Arity {
        min: sig
            .min_args
            .unwrap_or_else(|| u8::try_from(required).unwrap_or(u8::MAX)),
        max: if any_repeating {
            None
        } else {
            Some(u8::try_from(sig.params.len()).unwrap_or(u8::MAX))
        },
    }
}

/// Functions whose string-literal arguments declare new column names, usable as `[Name]` later in
/// the same expression: those with a `NewColumnName` parameter.
pub(crate) fn declares_columns(name: &str) -> bool {
    signature(name).is_some_and(|s| {
        s.params
            .iter()
            .any(|p| matches!(p.kind, ParamKind::NewColumnName))
    })
}

/// Bare words some functions accept as arguments (sort orders, intervals, relationship
/// directions, data types, window-function options…); they are neither tables nor variables.
/// The union of every `Keyword` parameter set, plus the keywords of a few `Any` parameters.
pub(crate) fn is_keyword_argument(word: &str) -> bool {
    catalog::is_keyword(word)
}

/// Every function of the catalog, sorted by name (the formula bar's autocomplete, `dax.functions`).
pub fn all() -> &'static [&'static Signature] {
    catalog::all()
}

/// BIcraft's one-sentence description of function `name` (English, French), for the formula
/// bar's autocomplete; written in BIcraft's own words.
pub fn description(name: &str) -> Option<(&'static str, &'static str)> {
    catalog::description(name)
}

/// A signature as a reader writes it: `DIVIDE(numerator, denominator[, alternate_result])`; the
/// repeating unit is written once, then `, …` (`SWITCH(expression, value, result, …[, else])`).
pub fn signature_text(sig: &Signature) -> String {
    let mut out = String::with_capacity(sig.name.len() + 16 * sig.params.len());
    out.push_str(sig.name);
    out.push('(');
    for (i, p) in sig.params.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        if p.optional {
            out.push('[');
            out.push_str(sep);
            out.push_str(p.name);
            out.push(']');
        } else {
            out.push_str(sep);
            out.push_str(p.name);
        }
        let unit_ends = p.repeating && !sig.params.get(i + 1).is_some_and(|q| q.repeating);
        if unit_ends {
            out.push_str(", …");
        }
    }
    out.push(')');
    out
}
