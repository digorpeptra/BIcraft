//! The Power Query editor's folding indicators and View native query (docs/mcp.md "Power Query
//! editor", docs/formats/data-sources.md "Folding indicators"):
//!
//! - `query.folding`: each applied step of a query with how much of the query, up to that step,
//!   the source runs (folded, partially folded, not folded, unknown);
//! - `query.native`: the SQL statement or URL BIcraft would send for the query at one step.
//!
//! Both evaluate the query at each step with `bicraft_data::m::folding::probe`, which never reads
//! a remote table's rows (navigation listings and column lists may be read). As in
//! `query.preview`, connectors run only when `allow_connectors` names them.

use std::path::PathBuf;
use std::time::Instant;

use bicraft_data::m::folding::{self, Folding, Probe};
use bicraft_data::m::steps;
use bicraft_data::{Limits, SourceOptions};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_str};

/// Default and largest time for all the steps together, in milliseconds.
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// Most queries followed when looking for the connectors a query needs.
const MAX_FOLLOWED: usize = 500;

pub(crate) const PARAMS: &[&str] = &[
    "name",
    "partition",
    "expression",
    "data_root",
    "allow_connectors",
    "timeout_ms",
];

/// The connector families (`allow_connectors`) that `text` calls, itself or through the model's
/// queries it reads, in the order of [`crate::refresh::CONNECTORS`].
pub(crate) fn needed_connectors(s: &Session, text: &str) -> Vec<&'static str> {
    let limits = Limits::default();
    let mut found: Vec<&'static str> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut todo = vec![text.to_string()];
    while let Some(t) = todo.pop() {
        for r in bicraft_data::m::analyze(&t, &limits).references {
            if let Some(f) = crate::refresh::family_of(&r.name) {
                if !found.contains(&f) {
                    found.push(f);
                }
                continue;
            }
            if seen.len() >= MAX_FOLLOWED || seen.contains(&r.name) {
                continue;
            }
            if let Some(e) = s
                .model
                .as_ref()
                .and_then(|m| m.expressions.iter().find(|e| e.name == r.name))
            {
                seen.push(r.name.clone());
                todo.push(e.expression.clone());
            }
        }
    }
    crate::refresh::CONNECTORS
        .iter()
        .copied()
        .filter(|c| found.contains(c))
        .collect()
}

/// What a probe of the query needs: its name and text, the connectors, the folder and the
/// deadline.
pub(crate) struct Probing {
    kind: &'static str,
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) allow_list: Vec<String>,
    connectors: bicraft_data::connect::Connectors,
    pub(crate) root: Option<PathBuf>,
    pub(crate) started: Instant,
    timeout: u64,
}

impl Probing {
    pub(crate) fn new(s: &Session, p: &Value) -> Result<Self, String> {
        let timeout = match p.get("timeout_ms") {
            None | Some(Value::Null) => DEFAULT_TIMEOUT_MS,
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=MAX_TIMEOUT_MS).contains(n))
                .ok_or_else(|| format!("`timeout_ms` must be a whole number from 1 to {MAX_TIMEOUT_MS}"))?,
        };
        let (kind, name, text) = crate::query_preview::source(s, p)?;
        let given_root = crate::datasrc::data_root_param(p)?;
        let root = crate::datasrc::effective_root(&given_root, s.data_root.as_deref());
        let allow_list = crate::refresh::string_list(p, "allow_connectors")?.unwrap_or_default();
        let (connectors, _dbx) = crate::refresh::connectors(s, &allow_list, false)?;
        Ok(Self {
            kind,
            name,
            text,
            allow_list,
            connectors,
            root,
            started: Instant::now(),
            timeout,
        })
    }

    /// The query's applied steps (a query that is not a `let` is one step).
    pub(crate) fn steps(&self) -> Result<Vec<String>, String> {
        let st = steps::steps(&self.text, &Limits::default())?;
        Ok(st.steps.into_iter().map(|x| x.name).collect())
    }

    pub(crate) fn options(&self, s: &Session, text: &str) -> Result<SourceOptions, String> {
        let spent = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let left = self.timeout.saturating_sub(spent);
        if left == 0 {
            return Err(format!(
                "the folding indicators took more than {} ms",
                self.timeout
            ));
        }
        let limits = Limits {
            max_millis: left,
            ..Limits::default()
        };
        let mut o =
            crate::datasrc::source_options(s.model.as_ref(), &self.name, text, self.root.as_deref(), limits);
        o.connectors = self.connectors.clone();
        Ok(o)
    }

    /// The probe of the query at `step`; an evaluation error is [`Folding::Unknown`] with it.
    fn probe(&self, s: &Session, step: &str) -> Probe {
        let unknown = |reason: String| Probe {
            folding: Folding::Unknown,
            connector: None,
            native: None,
            reason: Some(reason),
        };
        let text = match crate::query_steps::at_step(&self.text, step) {
            Ok(t) => t,
            Err(e) => return unknown(e),
        };
        let opts = match self.options(s, &text) {
            Ok(o) => o,
            Err(e) => return unknown(e),
        };
        match folding::probe(&text, &opts) {
            Ok(p) => p,
            Err(e) => {
                let msg = crate::datasrc::describe(&e, self.root.as_deref());
                if msg.contains("external source, not evaluated") && self.allow_list.is_empty() {
                    let need = needed_connectors(s, &self.text);
                    unknown(format!(
                        "{msg} (pass allow_connectors: {need:?} to let bicraft run it)"
                    ))
                } else {
                    unknown(msg)
                }
            }
        }
    }
}

/// `sql` or `url`: what a connector's native query is.
fn native_kind(connector: Option<&str>) -> Option<&'static str> {
    match connector? {
        "odata" | "web" | "sharepoint" => Some("url"),
        _ => Some("sql"),
    }
}

/// `query.folding`; see the module docs.
pub(crate) fn query_folding(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, PARAMS)?;
    let pr = Probing::new(s, p)?;
    let names = pr.steps()?;
    let mut out = Vec::with_capacity(names.len());
    for n in &names {
        let x = pr.probe(s, n);
        out.push(json!({
            "name": n,
            "folding": x.folding.as_str(),
            "connector": x.connector,
            "native_available": x.native.is_some(),
            "reason": x.reason,
        }));
    }
    Ok(json!({
        "name": pr.name,
        "source": pr.kind,
        "steps": out,
        "connectors": needed_connectors(s, &pr.text),
        "allow_connectors": pr.allow_list,
        "elapsed_ms": u64::try_from(pr.started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }))
}

/// `query.native`; see the module docs.
pub(crate) fn query_native(s: &Session, p: &Value) -> Result<Value, String> {
    let mut keys = PARAMS.to_vec();
    keys.push("step");
    allow(p, &keys)?;
    let pr = Probing::new(s, p)?;
    let names = pr.steps()?;
    let step = match opt_str(p, "step")? {
        Some(st) => {
            if !names.iter().any(|n| n == st) {
                return Err(format!("query `{}` has no step `{st}`", pr.name));
            }
            st.to_string()
        }
        None => steps::steps(&pr.text, &Limits::default())?
            .last()
            .map(|x| x.name.clone())
            .ok_or("the query has no step")?,
    };
    let x = pr.probe(s, &step);
    let reason = match (&x.native, x.folding) {
        (Some(_), _) => None,
        (None, Folding::Folded) => x.reason.clone(),
        (None, Folding::Unknown) => x.reason.clone(),
        (None, _) => Some(format!(
            "the step does not fold ({}): {}",
            x.folding.as_str(),
            x.reason.clone().unwrap_or_default()
        )),
    };
    Ok(json!({
        "name": pr.name,
        "step": step,
        "folding": x.folding.as_str(),
        "connector": x.connector,
        "kind": x.native.as_ref().and(native_kind(x.connector.as_deref())),
        "native": x.native,
        "reason": reason,
    }))
}
