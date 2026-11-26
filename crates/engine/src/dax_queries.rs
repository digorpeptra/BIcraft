//! The DAX query view's tabs as part of the open file, and its Format query and Cancel
//! (`dax.queries`, `dax.set_queries`, `dax.format`, `dax.cancel`; docs/formats/dax-queries.md).
//!
//! `file.open` reads a PBIP project's `DAXQueries` folder into [`State`]; the app hands its tabs
//! back with `dax.set_queries` (an unsaved change of its own, not a model edit, no undo step);
//! `file.save` writes them into the project, and `file.save_as` / `project.new` into the new
//! project. A PBIX or PBIT keeps them for the session only: no public description of where a
//! package holds them (docs/gaps.md G81).
//!
//! `dax.cancel` stops the `dax.evaluate` runs of the session that started before it: each run
//! notes the session's cancel epoch when it starts ([`Running`]), `dax.cancel` moves the epoch on,
//! and the evaluator, which checks [`Session::is_cancelled`] where it checks its time budget, then
//! fails with [`crate::CANCELLED`]. `dax.cancel` is read-only, so a server runs it beside the
//! evaluation it stops (`shared.rs`).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bicraft_pbip::{DaxQueries, DaxQuery};
use serde_json::{Value, json};

use crate::params::{allow, opt_str, str_param};
use crate::{Session, Source};

/// The open file's DAX query tabs.
#[derive(Debug, Default, Clone)]
pub(crate) struct State {
    pub queries: DaxQueries,
    /// Changed since the file was opened or saved.
    pub dirty: bool,
    /// Changes with every `file.open` (the app reloads its tabs when it moves).
    pub generation: u64,
    /// Why the file's queries could not be read (they are then left as they are on disk until
    /// the app sets new ones).
    pub error: Option<String>,
}

/// The session's cancel epoch and how many evaluations run.
#[derive(Debug, Default)]
pub(crate) struct Cancel {
    epoch: AtomicU64,
    running: AtomicUsize,
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

/// Where the queries are saved: `project` (the open file is a PBIP project, or a new model that
/// a save makes one), else `session` (a PBIX, PBIT or model.bim keeps them for the session).
fn saved_in(s: &Session) -> &'static str {
    match &s.source {
        Some(Source::Project { .. }) | None => "project",
        _ => "session",
    }
}

/// The queries `file.open` puts in the session: the project's.
pub(crate) fn at_open(s: &mut Session) {
    let (queries, error) = match &s.source {
        Some(Source::Project { project, .. }) => match bicraft_pbip::read_dax_queries(project) {
            Ok(q) => (q.unwrap_or_default(), None),
            Err(e) => (DaxQueries::default(), Some(e.to_string())),
        },
        _ => (DaxQueries::default(), None),
    };
    s.dax_queries = State {
        queries,
        dirty: false,
        generation: next_generation(),
        error,
    };
}

/// `model.new`: no queries.
pub(crate) fn reset(s: &mut Session) {
    at_open(s);
}

/// What the snapshot says: `{generation, tabs: [{name, text}], default_tab, saved_in}`.
pub(crate) fn snapshot(s: &Session) -> Value {
    let st = &s.dax_queries;
    json!({
        "generation": st.generation,
        "tabs": st.queries.tabs.iter().map(|t| json!({"name": t.name, "text": t.text})).collect::<Vec<_>>(),
        "default_tab": st.queries.default_tab,
        "saved_in": saved_in(s),
    })
}

/// The queries have changes a save writes (`dirty.queries` of the snapshot).
pub(crate) fn unsaved(s: &Session) -> bool {
    s.dax_queries.dirty && saved_in(s) == "project"
}

/// `dax.queries {}`.
pub(crate) fn dax_queries(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let mut out = snapshot(s);
    if let Some(o) = out.as_object_mut() {
        o.insert("dirty".into(), json!(s.dax_queries.dirty));
        o.insert("error".into(), json!(s.dax_queries.error));
    }
    Ok(out)
}

/// The tabs of `dax.set_queries`, checked.
fn parse_tabs(p: &Value) -> Result<DaxQueries, String> {
    let Some(list) = p.get("tabs").and_then(Value::as_array) else {
        return Err("`tabs` is required: a list of {name, text}".into());
    };
    if list.len() > bicraft_pbip::MAX_QUERY_TABS {
        return Err(format!("at most {} query tabs", bicraft_pbip::MAX_QUERY_TABS));
    }
    let mut tabs: Vec<DaxQuery> = Vec::with_capacity(list.len());
    let mut total = 0usize;
    for t in list {
        allow(t, &["name", "text"]).map_err(|e| format!("a tab: {e}"))?;
        let name = str_param(t, "name").map_err(|e| format!("a tab: {e}"))?;
        let text = match t.get("text") {
            None | Some(Value::Null) => "",
            Some(Value::String(s)) => s.as_str(),
            Some(_) => return Err(format!("tab `{name}`: `text` must be a string")),
        };
        bicraft_pbip::check_query_name(name).map_err(|why| format!("tab `{name}`: {why}"))?;
        if tabs.iter().any(|o| o.name.eq_ignore_ascii_case(name)) {
            return Err(format!("two query tabs are named `{name}`"));
        }
        total = total.saturating_add(text.len());
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > bicraft_pbip::MAX_QUERY_BYTES {
            return Err(format!(
                "tab `{name}` is longer than {} bytes",
                bicraft_pbip::MAX_QUERY_BYTES
            ));
        }
        if total > 64 * 1024 * 1024 {
            return Err("the query tabs hold more than 64 MiB of text".into());
        }
        tabs.push(DaxQuery {
            name: name.to_string(),
            text: text.to_string(),
        });
    }
    let default_tab = match opt_str(p, "default_tab")? {
        None => None,
        Some(d) => match tabs.iter().find(|t| t.name.eq_ignore_ascii_case(d)) {
            Some(t) => Some(t.name.clone()),
            None => return Err(format!("`default_tab`: no tab is named `{d}`")),
        },
    };
    Ok(DaxQueries { tabs, default_tab })
}

/// `dax.set_queries {tabs: [{name, text}], default_tab?}`.
pub(crate) fn dax_set_queries(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["tabs", "default_tab"])?;
    let queries = parse_tabs(p)?;
    let changed = queries != s.dax_queries.queries;
    if changed {
        s.dax_queries.queries = queries;
        s.dax_queries.dirty = true;
        s.dax_queries.error = None;
    }
    Ok(json!({
        "tabs": s.dax_queries.queries.tabs.len(),
        "changed": changed,
        "dirty": s.dax_queries.dirty,
        "saved_in": saved_in(s),
    }))
}

/// `file.save` of a project: writes the queries when they changed. Returns a warning for a file
/// that cannot hold them.
pub(crate) fn save(s: &mut Session) -> Result<Option<String>, String> {
    if !s.dax_queries.dirty {
        return Ok(None);
    }
    match &s.source {
        Some(Source::Project { project, .. }) => {
            bicraft_pbip::write_dax_queries(project, &s.dax_queries.queries)
                .map_err(|e| format!("the DAX queries could not be saved: {e}"))?;
            s.dax_queries.dirty = false;
            Ok(None)
        }
        _ if s.dax_queries.queries.tabs.is_empty() => Ok(None),
        _ => Ok(Some(
            "the DAX query tabs are kept for this session only: BIcraft saves them in a PBIP project's \
             DAXQueries folder, not in this kind of file (save as .pbip to keep them)"
                .to_string(),
        )),
    }
}

/// After `file.save_as` or `project.new` made a new project the open file: its queries are
/// written into it. A failure is a warning (the project is saved).
pub(crate) fn into_new_project(s: &mut Session) -> Option<String> {
    let Some(Source::Project { project, .. }) = &s.source else {
        return None;
    };
    if s.dax_queries.queries.tabs.is_empty() {
        s.dax_queries.dirty = false;
        return None;
    }
    match bicraft_pbip::write_dax_queries(project, &s.dax_queries.queries) {
        Ok(()) => {
            s.dax_queries.dirty = false;
            None
        }
        Err(e) => Some(format!(
            "the project was saved, but its DAX queries could not be: {e}"
        )),
    }
}

/// `dax.format {query, max_width?, indent?}`.
pub(crate) fn dax_format(_s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &["query", "max_width", "indent", "style"])?;
    let query = str_param(p, "query")?;
    let style = match p.get("style") {
        None | Some(Value::Null) => bicraft_dax::format::Style::Query,
        Some(Value::String(s)) if s == "query" => bicraft_dax::format::Style::Query,
        Some(Value::String(s)) if s == "formula" => bicraft_dax::format::Style::Formula,
        Some(_) => return Err("`style` must be `query` or `formula`".into()),
    };
    let count = |k: &str, default: usize, min: u64, max: u64| -> Result<usize, String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => match v.as_u64() {
                Some(n) if (min..=max).contains(&n) => Ok(usize::try_from(n).unwrap_or(default)),
                _ => Err(format!("`{k}` must be an integer from {min} to {max}")),
            },
        }
    };
    let opts = bicraft_dax::FormatOptions {
        max_width: count("max_width", 100, 20, 1000)?,
        indent: count("indent", 4, 0, 16)?,
        style,
    };
    let out = bicraft_dax::format_dax(query, &opts)?;
    Ok(json!({"changed": out != query, "query": out}))
}

/// A `dax.evaluate` run: counted, and stopped by a `dax.cancel` made after it started.
pub(crate) struct Running<'a> {
    cancel: &'a Cancel,
    _scope: crate::call_scope::Guard,
}

impl<'a> Running<'a> {
    pub(crate) fn start(s: &'a Session) -> Self {
        let cancel = &s.dax_cancel;
        cancel.running.fetch_add(1, Ordering::SeqCst);
        let epoch = cancel.epoch.load(Ordering::SeqCst);
        Running {
            cancel,
            _scope: crate::call_scope::dax_epoch(epoch),
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.cancel.running.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Session {
    /// The run's `dax.evaluate` started before the last `dax.cancel`.
    pub(crate) fn dax_cancelled(&self) -> bool {
        crate::call_scope::dax_epoch_now().is_some_and(|e| e != self.dax_cancel.epoch.load(Ordering::SeqCst))
    }
}

/// `dax.cancel {}`.
pub(crate) fn dax_cancel(s: &Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let running = s.dax_cancel.running.load(Ordering::SeqCst);
    s.dax_cancel.epoch.fetch_add(1, Ordering::SeqCst);
    Ok(json!({"running": running}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_queries_checks_its_tabs() {
        let mut s = Session::new();
        for bad in [
            json!({}),
            json!({"tabs": 1}),
            json!({"tabs": [{"name": "a/b"}]}),
            json!({"tabs": [{"name": "a"}, {"name": "A"}]}),
            json!({"tabs": [{"name": "a", "text": 3}]}),
            json!({"tabs": [{"name": "a", "x": 1}]}),
            json!({"tabs": [{"name": "a"}], "default_tab": "b"}),
        ] {
            assert!(dax_set_queries(&mut s, &bad).is_err(), "{bad}");
        }
        assert!(!s.dax_queries.dirty);
        let ok = json!({"tabs": [{"name": "Query 1", "text": "EVALUATE {1}"}], "default_tab": "query 1"});
        let r = dax_set_queries(&mut s, &ok).unwrap();
        assert_eq!(r["changed"], true);
        assert_eq!(s.dax_queries.queries.default_tab.as_deref(), Some("Query 1"));
        assert!(unsaved(&s));
        let again = dax_set_queries(&mut s, &ok).unwrap();
        assert_eq!(again["changed"], false);
        let q = dax_queries(&s, &json!({})).unwrap();
        assert_eq!(q["tabs"][0]["text"], "EVALUATE {1}");
        assert_eq!(q["saved_in"], "project");
    }

    #[test]
    fn cancel_stops_only_runs_started_before_it() {
        let s = Session::new();
        assert!(!s.dax_cancelled());
        {
            let _run = Running::start(&s);
            assert!(!s.dax_cancelled());
            assert_eq!(dax_cancel(&s, &json!({})).unwrap()["running"], 1);
            assert!(s.dax_cancelled());
            assert!(s.is_cancelled());
        }
        assert!(!s.dax_cancelled());
        let _run = Running::start(&s);
        assert!(!s.dax_cancelled());
        assert!(dax_cancel(&s, &json!({"x": 1})).is_err());
    }

    #[test]
    fn format_takes_its_options() {
        let s = Session::new();
        let r = dax_format(&s, &json!({"query": "evaluate {1,2}"})).unwrap();
        assert_eq!(r["query"], "evaluate\n    {1, 2}");
        assert_eq!(r["changed"], true);
        assert!(dax_format(&s, &json!({"query": "x", "indent": 99})).is_err());
        assert!(dax_format(&s, &json!({"query": "EVALUATE ("})).is_err());
        let r = dax_format(&s, &json!({"query": "evaluate\n  {1, 2}", "indent": 2})).unwrap();
        assert_eq!(r["changed"], false);
    }
}
