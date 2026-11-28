//! The Performance analyzer (ROADMAP review lot 14, `docs/target-app-parity.md` row 14): a
//! recording of how long BIcraft's own evaluator takes for each visual, with the DAX query text
//! BIcraft writes for it and the render time the app measures.
//!
//! - `perf.start` / `perf.stop` start and stop a recording; `perf.clear` empties it.
//! - While a recording runs, every visual the viewer computes for the page it shows (not answered
//!   from the result cache) is recorded (`cause: "view"`), and `perf.refresh` evaluates every
//!   visual of a page afresh, without the result cache, each timed on its own
//!   (`cause: "refresh"`, the pane's Refresh visuals).
//! - `perf.render` hands back the render time the app measured for visuals it drew (the renderer's
//!   draw call of each visual), so the export holds both. While a recording runs, the snapshot's
//!   `perf.unrendered` names, per page, the visuals whose latest event has no render time yet: the
//!   app sends the times of the drawing that shows them, after Refresh visuals and after the
//!   viewer's own evaluations alike.
//! - `perf.results` returns the events; `perf.export {out}` writes them as JSON.
//!
//! Times come from a monotonic clock (`std::time::Instant`), in milliseconds. They are BIcraft's
//! own and say nothing about another product's; tests check their structure, never a duration.
//! The recording is the session's diagnostics state: it is not the model, the report or the file,
//! so the commands bump no revision and leave no undo step (`crate::READ_ONLY_COMMANDS`); on a
//! server they are for editors only, since a session is shared and a refresh evaluates afresh.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::Session;
use crate::params::{allow, opt_str, str_param};
use crate::preview::{self, PreviewLimits};

/// Events kept; older ones are dropped (and counted) beyond it.
const MAX_EVENTS: usize = 10_000;
/// Largest render time accepted from the app, in milliseconds (an hour).
const MAX_RENDER_MS: f64 = 3_600_000.0;
/// Most render times one `perf.render` hands back.
const MAX_RENDER_TIMES: usize = 10_000;

/// Why a visual was evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cause {
    /// `perf.refresh` (Refresh visuals).
    Refresh,
    /// The viewer computed it for the page it shows.
    View,
}

impl Cause {
    fn name(self) -> &'static str {
        match self {
            Cause::Refresh => "refresh",
            Cause::View => "view",
        }
    }
}

/// One visual evaluated during a recording.
#[derive(Debug, Clone)]
struct Event {
    seq: u64,
    /// Since the recording started.
    at: Duration,
    page: String,
    visual: String,
    visual_type: Option<String>,
    cause: Cause,
    evaluation: Duration,
    /// Measured by the app and handed back with `perf.render`.
    render_ms: Option<f64>,
    status: String,
    reason: Option<String>,
    rows: Option<u64>,
    query: Option<String>,
    query_notes: Vec<String>,
}

#[derive(Debug, Default)]
struct Inner {
    recording: bool,
    started: Option<Instant>,
    events: Vec<Event>,
    next_seq: u64,
    dropped: u64,
}

/// The session's recording (interior mutability: the viewer records from `&Session`).
#[derive(Debug, Default)]
pub(crate) struct Recorder(Mutex<Inner>);

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a recording runs.
    pub(crate) fn recording(&self) -> bool {
        self.lock().recording
    }

    /// While a recording runs, the visuals whose latest event has no render time yet, by page
    /// (the snapshot's `perf`): the app hands back the render time of the drawing that shows
    /// them (`perf.render`), so the visuals the viewer evaluates get one without Refresh visuals.
    /// `None` when no recording runs.
    pub(crate) fn snapshot_json(&self) -> Option<Value> {
        let g = self.lock();
        if !g.recording {
            return None;
        }
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        let mut latest: Vec<(&str, &str, bool)> = Vec::new();
        for e in g.events.iter().rev() {
            if seen.insert((&e.page, &e.visual)) {
                latest.push((&e.page, &e.visual, e.render_ms.is_none()));
            }
        }
        let mut pages = Map::new();
        for (page, visual, unrendered) in latest.into_iter().rev() {
            if !unrendered {
                continue;
            }
            if let Value::Array(a) = pages
                .entry(page.to_string())
                .or_insert_with(|| Value::Array(Vec::new()))
            {
                a.push(json!(visual));
            }
        }
        Some(json!({"recording": true, "unrendered": pages}))
    }

    /// Records visual `t` of page `page` when a recording runs.
    pub(crate) fn record(&self, page: &str, cause: Cause, t: preview::perf::Timed) {
        let mut g = self.lock();
        if !g.recording {
            return;
        }
        let at = g.started.map_or(Duration::ZERO, |s| s.elapsed());
        let seq = g.next_seq;
        g.next_seq = g.next_seq.saturating_add(1);
        let status = t
            .data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("ok")
            .to_string();
        let reason = t.data.get("reason").and_then(Value::as_str).map(str::to_string);
        let rows = t.data.get("total_rows").and_then(Value::as_u64);
        g.events.push(Event {
            seq,
            at,
            page: page.to_string(),
            visual: t.visual,
            visual_type: t.visual_type,
            cause,
            evaluation: t.elapsed,
            render_ms: None,
            status,
            reason,
            rows,
            query: t.query.text,
            query_notes: t.query.notes,
        });
        if g.events.len() > MAX_EVENTS {
            let extra = g.events.len() - MAX_EVENTS;
            g.events.drain(..extra);
            g.dropped = g.dropped.saturating_add(extra as u64);
        }
    }
}

/// Milliseconds with three decimals (microseconds).
fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

fn event_json(e: &Event) -> Value {
    let evaluation = ms(e.evaluation);
    let total = e
        .render_ms
        .map_or(evaluation, |r| ((evaluation + r) * 1000.0).round() / 1000.0);
    let mut o = Map::new();
    o.insert("seq".into(), json!(e.seq));
    o.insert("at_ms".into(), json!(ms(e.at)));
    o.insert("page".into(), json!(e.page));
    o.insert("visual".into(), json!(e.visual));
    o.insert("visual_type".into(), json!(e.visual_type));
    o.insert("cause".into(), json!(e.cause.name()));
    o.insert("evaluation_ms".into(), json!(evaluation));
    o.insert("render_ms".into(), json!(e.render_ms));
    o.insert("total_ms".into(), json!(total));
    o.insert("status".into(), json!(e.status));
    if let Some(r) = &e.reason {
        o.insert("reason".into(), json!(r));
    }
    o.insert("rows".into(), json!(e.rows));
    o.insert("query".into(), json!(e.query));
    o.insert("query_notes".into(), json!(e.query_notes));
    Value::Object(o)
}

fn results_json(g: &Inner) -> Value {
    json!({
        "recording": g.recording,
        "elapsed_ms": g.started.map(|s| ms(s.elapsed())),
        "events": g.events.iter().map(event_json).collect::<Vec<_>>(),
        "dropped": g.dropped,
        "clock": "monotonic",
        "measured_by": "BIcraft",
    })
}

/// `perf.start {}`.
pub(crate) fn perf_start(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let mut g = s.perf.lock();
    g.recording = true;
    if g.started.is_none() {
        g.started = Some(Instant::now());
    }
    Ok(json!({"recording": true, "events": g.events.len()}))
}

/// `perf.stop {}`.
pub(crate) fn perf_stop(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let mut g = s.perf.lock();
    g.recording = false;
    Ok(json!({"recording": false, "events": g.events.len()}))
}

/// `perf.clear {}`.
pub(crate) fn perf_clear(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    let mut g = s.perf.lock();
    let cleared = g.events.len();
    g.events.clear();
    g.dropped = 0;
    g.started = g.recording.then(Instant::now);
    Ok(json!({"recording": g.recording, "cleared": cleared}))
}

/// `perf.results {}`.
pub(crate) fn perf_results(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &[])?;
    Ok(results_json(&s.perf.lock()))
}

/// `perf.refresh {page?, visual?}`: the visuals of the page evaluated afresh and recorded.
pub(crate) fn perf_refresh(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "visual"])?;
    let page = opt_str(p, "page")?;
    let visual = opt_str(p, "visual")?;
    if !s.perf.recording() {
        return Err("no recording runs: start one first (perf.start)".into());
    }
    // the same as the Optimize ribbon's Refresh visuals: a paused report shows the new data
    s.preview.request_refresh();
    let Some(base) = s.report.as_ref() else {
        return Err("the session has no report".into());
    };
    // the visuals as the viewer evaluates them: under its slicer selections and cross-filters
    let effective = s.interactions.effective(base);
    let report = effective.as_ref().map_or(base, |e| &e.data);
    let pg = match page {
        Some(name) => report
            .pages
            .iter()
            .find(|x| x.name == name)
            .or_else(|| report.pages.iter().find(|x| x.display_name == name))
            .ok_or_else(|| format!("no page `{name}`"))?,
        None => report
            .active_page
            .as_deref()
            .and_then(|a| report.pages.iter().find(|x| x.name == a))
            .or_else(|| report.pages.first())
            .ok_or("the report has no page")?,
    };
    if let Some(v) = visual
        && !pg.visuals.iter().any(|x| x.name == v)
    {
        return Err(format!("no visual `{v}` on page `{}`", pg.name));
    }
    let limits = PreviewLimits {
        rows_per_visual: crate::snapshot::SnapshotLimits::default().data_rows_per_visual,
        table_rows: preview::STORAGE_TABLE_ROWS,
        fallback_rows: crate::snapshot::SnapshotLimits::default().data_table_rows,
        budget: Duration::MAX,
        now: preview::filters::clock(s),
        utc_offset: preview::filters::utc_offset(s),
    };
    let timed = preview::perf::evaluate_page(s, report, pg, visual, &limits);
    let page_name = pg.name.clone();
    let first = s.perf.lock().next_seq;
    for t in timed {
        s.perf.record(&page_name, Cause::Refresh, t);
    }
    let g = s.perf.lock();
    let visuals: Vec<Value> = g
        .events
        .iter()
        .filter(|e| e.seq >= first)
        .map(event_json)
        .collect();
    Ok(json!({"page": page_name, "visuals": visuals}))
}

/// `perf.render {page, times: {visual: ms}}`: the render time the app measured for each visual,
/// on the latest event of that visual of `page` that has none yet.
pub(crate) fn perf_render(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["page", "times"])?;
    let page = str_param(p, "page")?;
    let times = p
        .get("times")
        .and_then(Value::as_object)
        .ok_or("`times` must be an object {visual: milliseconds}")?;
    if times.len() > MAX_RENDER_TIMES {
        return Err(format!("`times` holds more than {MAX_RENDER_TIMES} visuals"));
    }
    let mut parsed = Vec::with_capacity(times.len());
    for (visual, t) in times {
        let t = t
            .as_f64()
            .filter(|x| x.is_finite() && *x >= 0.0 && *x <= MAX_RENDER_MS)
            .ok_or_else(|| {
                format!(
                    "the render time of `{visual}` must be a number of milliseconds from 0 to {MAX_RENDER_MS}"
                )
            })?;
        parsed.push((visual.as_str(), (t * 1000.0).round() / 1000.0));
    }
    let mut g = s.perf.lock();
    let mut updated = 0usize;
    for (visual, t) in parsed {
        if let Some(e) = g
            .events
            .iter_mut()
            .rev()
            .find(|e| e.page == page && e.visual == visual)
            .filter(|e| e.render_ms.is_none())
        {
            e.render_ms = Some(t);
            updated = updated.saturating_add(1);
        }
    }
    Ok(json!({"updated": updated}))
}

/// `perf.export {out}`: the results as a JSON file.
pub(crate) fn perf_export(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["out"])?;
    let out = match opt_str(p, "out")? {
        None => return Err("missing parameter `out` (the .json file to write)".into()),
        Some(o) if o.trim().is_empty() => return Err("`out` is empty: give the .json file to write".into()),
        Some(o) => std::path::PathBuf::from(o),
    };
    if out.is_dir() {
        return Err(format!(
            "`out` is a folder ({}): give a .json file",
            out.display()
        ));
    }
    let (doc, events) = {
        let g = s.perf.lock();
        (results_json(&g), g.events.len())
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| format!("cannot write the results: {e}"))?;
    if let Some(dir) = out.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&out, &bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(json!({"path": out.display().to_string(), "bytes": bytes.len(), "events": events}))
}

#[cfg(test)]
#[path = "perf_tests.rs"]
mod tests;
