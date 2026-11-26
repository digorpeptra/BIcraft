//! Folding indicators, View native query and query diagnostics (docs/formats/data-sources.md,
//! *Folding indicators*).
//!
//! [`probe`] evaluates a query without reading any remote table's rows: when its value is still a
//! connector value, everything up to it goes to the source ([`Folding::Folded`]) and the
//! connector shows the query it would send ([`crate::connect::Source::native`]); when the
//! evaluator needs the rows of a source's query to go on, the source runs part of the query and
//! BIcraft the rest ([`Folding::Partial`]); a query that reads no folding source is
//! [`Folding::NotFolded`]. Navigation listings and column lists (metadata) may still be read.
//!
//! [`diagnose`] evaluates a query as a refresh does and keeps the requests sent to its sources
//! with BIcraft's own row counts and times.

use std::time::Instant;

use super::eval::Interp;
use super::value::{Flow, Value};
use super::{on_eval_thread, parsed, remote};
use crate::{Error, SourceOptions, TableData};

/// How much of a query (up to one of its steps) the source runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folding {
    /// Everything goes to the source as one request.
    Folded,
    /// The source runs part of it; BIcraft's evaluator runs the rest over the rows it returns.
    Partial,
    /// BIcraft's evaluator runs all of it (no source that folds).
    NotFolded,
    /// Not known: the query could not be evaluated far enough.
    Unknown,
}

impl Folding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Folded => "folded",
            Self::Partial => "partial",
            Self::NotFolded => "not_folded",
            Self::Unknown => "unknown",
        }
    }
}

/// The folding of one query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub folding: Folding,
    /// The connector family of the source (`sqlserver`, `odata`, …).
    pub connector: Option<String>,
    /// The query the source would be sent (a SQL statement, a URL): folded tables only.
    pub native: Option<String>,
    /// Why there is no native query, in English.
    pub reason: Option<String>,
}

/// One request an evaluation sent to a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequest {
    pub connector: String,
    /// `list` (a navigation table's items) or `query` (a table's rows).
    pub kind: &'static str,
    /// The native query, or the navigation path.
    pub text: String,
    pub rows: usize,
    /// BIcraft's own time for the request, in milliseconds.
    pub millis: u64,
}

impl SourceRequest {
    pub(crate) fn new(
        connector: &str,
        kind: &'static str,
        text: String,
        rows: usize,
        started: Option<Instant>,
    ) -> Self {
        Self {
            connector: connector.to_string(),
            kind,
            text,
            rows,
            millis: started.map_or(0, |s| u64::try_from(s.elapsed().as_millis()).unwrap_or(u64::MAX)),
        }
    }
}

fn checked(expression: &str, opts: &SourceOptions) -> Result<(), Error> {
    opts.limits.check_bytes(expression.len(), "M expression")?;
    for (name, e) in &opts.shared {
        opts.limits
            .check_bytes(e.len(), &format!("M expression `{name}`"))?;
    }
    Ok(())
}

fn error(f: Flow) -> Error {
    match f {
        Flow::Fatal(e) => e,
        Flow::Err(e) => Error::Evaluation {
            reason: e.reason.clone(),
            message: e.message.clone(),
        },
    }
}

/// The folding of `expression` (the query at one step); `Err`: [`Folding::Unknown`], with the
/// evaluation's error.
pub fn probe(expression: &str, opts: &SourceOptions) -> Result<Probe, Error> {
    checked(expression, opts)?;
    let expr = expression.to_string();
    let opts = opts.clone();
    on_eval_thread(move || {
        let (main, shared) = parsed(&expr, &opts)?;
        let mut interp = Interp::new(&opts, shared);
        interp.probe = true;
        let r = interp.eval(&main, &None);
        if let Some(connector) = interp.probe_hit {
            return Ok(Probe {
                folding: Folding::Partial,
                connector: Some(connector.to_string()),
                native: None,
                reason: Some(
                    "the source runs the query up to the last folded step; BIcraft runs the rest".into(),
                ),
            });
        }
        match r {
            Ok(Value::Remote(r)) => Ok(remote::describe(&r)),
            Ok(_) => Ok(Probe {
                folding: Folding::NotFolded,
                connector: None,
                native: None,
                reason: Some("no source that folds: BIcraft's evaluator runs the query".into()),
            }),
            Err(f) => Err(error(f)),
        }
    })
}

/// A diagnosed evaluation: its outcome and the requests sent to its sources, in order.
#[derive(Debug)]
pub struct Diagnosis {
    pub result: Result<TableData, Error>,
    pub requests: Vec<SourceRequest>,
}

/// Evaluates `expression` to table data as [`super::load_m_with`] does, keeping its source
/// requests.
pub fn diagnose(expression: &str, opts: &SourceOptions) -> Result<Diagnosis, Error> {
    checked(expression, opts)?;
    let expr = expression.to_string();
    let opts = opts.clone();
    on_eval_thread(move || {
        let (main, shared) = parsed(&expr, &opts)?;
        let mut interp = Interp::new(&opts, shared);
        interp.requests = Some(Vec::new());
        let r = interp
            .eval(&main, &None)
            .and_then(|v| remote::force_value(&mut interp, v));
        let result = super::finish(r, &opts);
        Ok(Diagnosis {
            result,
            requests: interp.requests.take().unwrap_or_default(),
        })
    })
}
