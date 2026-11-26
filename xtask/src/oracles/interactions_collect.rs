//! `oracle-gen interactions DIR collect [OUT.json]`: turns one recording of the interaction
//! oracle (`DIR/raw/`: `instance.json`, `trace.jsonl`, `readings.json`, `results.json`, written by
//! xtask/interaction-oracle/record.ps1) into the versioned corpus
//! `testsupport/interaction_oracle/corpus.json` (format: docs/interaction-oracle.md), and copies
//! the model next to it.
//!
//! Each traced DAX query is given to the visuals of the page shown when Desktop sent it whose
//! fields are the query's fields (the columns and measures it references outside its filter
//! tables). A visual keeps its last query until a new one arrives: Desktop answers a query it
//! already sent from its cache without asking the engine, so a step that sends nothing leaves
//! the visual on its previous data. When the display no longer matches that query (a step went
//! back to an earlier state, served from the cache), the earlier query of the same visual whose
//! result matches the display is taken instead (`query_from: "cache"`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{read_json, write_json};

/// The repository's corpus folder.
pub fn corpus_dir() -> PathBuf {
    crate::root().join("testsupport").join("interaction_oracle")
}

/// Milliseconds since 1970 of an ISO 8601 date-time with `Z` or a `+hh:mm` offset (fractions of
/// any length); `None` when it does not parse.
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let t = s.get(from..from + len)?;
        if !t.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse().ok()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, se) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut i = 19;
    let mut ms = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let frac = s.get(start..i)?;
        let first3: String = frac.chars().chain("000".chars()).take(3).collect();
        ms = first3.parse().ok()?;
    }
    let offset_min = match b.get(i) {
        Some(b'Z') | None => 0,
        Some(c @ (b'+' | b'-')) => {
            let oh = num(i + 1, 2)?;
            let om = num(i + 4, 2)?;
            let v = oh * 60 + om;
            if *c == b'-' { -v } else { v }
        }
        _ => return None,
    };
    // days from civil (Howard Hinnant's algorithm)
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi - offset_min) * 60_000 + se * 1000 + ms)
}

/// The fields a Desktop query is about: the `'Table'[Column]` references outside the variables
/// that define filter tables (`__DS0FilterTable`, `__H0FilterTable`…), as `Table[Column]`;
/// the page tag (`Tag`) is left out.
pub fn query_fields(dax: &str) -> BTreeSet<String> {
    let mut kept = String::new();
    let mut rest = dax;
    loop {
        let Some(pos) = find_var(rest) else {
            kept.push_str(rest);
            break;
        };
        let (before, after) = rest.split_at(pos);
        kept.push_str(before);
        let name: String = after
            .get(3..)
            .unwrap_or_default()
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let tail = after.get(3..).unwrap_or_default();
        let body_end = find_var(tail)
            .map(|p| p + 3)
            .or_else(|| after.find("EVALUATE"))
            .unwrap_or(after.len());
        if !name.contains("FilterTable") {
            kept.push_str(after.get(..body_end).unwrap_or_default());
        }
        rest = after.get(body_end..).unwrap_or_default();
    }
    let mut out = BTreeSet::new();
    let cs: Vec<char> = kept.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs.get(i) != Some(&'\'') {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut table = String::new();
        while let Some(&c) = cs.get(j) {
            if c == '\'' {
                if cs.get(j + 1) == Some(&'\'') {
                    table.push('\'');
                    j += 2;
                    continue;
                }
                break;
            }
            table.push(c);
            j += 1;
        }
        if cs.get(j + 1) == Some(&'[') {
            let mut k = j + 2;
            let mut col = String::new();
            while let Some(&c) = cs.get(k) {
                if c == ']' {
                    break;
                }
                col.push(c);
                k += 1;
            }
            if table != "Tag" {
                out.insert(format!("{table}[{col}]"));
            }
            i = k + 1;
        } else {
            i = j + 1;
        }
    }
    out
}

/// Byte offset of the next `VAR ` keyword (a whole word), if any.
fn find_var(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(p) = s.get(from..).and_then(|t| t.find("VAR ")) {
        let at = from + p;
        let before_ok = at == 0
            || s.get(..at)
                .and_then(|t| t.chars().last())
                .is_some_and(char::is_whitespace);
        if before_ok {
            return Some(at);
        }
        from = at + 4;
    }
    None
}

/// The fields of a plan visual (`[Measure]` is a measure of `Sales`).
pub fn visual_fields(v: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for f in v["fields"].as_array().into_iter().flatten() {
        let Some(t) = f.get(1).and_then(Value::as_str) else {
            continue;
        };
        if t.starts_with('[') {
            out.insert(format!("Sales{t}"));
        } else {
            out.insert(t.to_string());
        }
    }
    out
}

struct Query {
    at: i64,
    text: String,
}

fn read_trace(path: &Path) -> Result<Vec<Query>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let e: Value = serde_json::from_str(line).map_err(|e| format!("trace line: {e}"))?;
        if e["event"] != "QueryBegin" || e["subclass"] != "DAXQuery" {
            continue;
        }
        let (Some(t), Some(at)) = (e["text"].as_str(), e["start"].as_str().and_then(parse_iso)) else {
            continue;
        };
        out.push(Query {
            at,
            text: t.to_string(),
        });
    }
    out.sort_by_key(|q| q.at);
    Ok(out)
}

/// Numbers of a query result (every cell of every result set), for matching a display.
fn result_numbers(res: &Value) -> Vec<f64> {
    let mut out = Vec::new();
    for set in res.as_array().into_iter().flatten() {
        for row in set["rows"].as_array().into_iter().flatten() {
            for v in row.as_object().into_iter().flat_map(|o| o.values()) {
                if let Some(x) = v.as_f64() {
                    out.push(x);
                }
            }
        }
    }
    out
}

fn result_texts(res: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for set in res.as_array().into_iter().flatten() {
        for row in set["rows"].as_array().into_iter().flatten() {
            for v in row.as_object().into_iter().flat_map(|o| o.values()) {
                match v {
                    Value::String(s) => {
                        out.insert(s.clone());
                    }
                    Value::Number(n) => {
                        out.insert(n.to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// A card's displayed number: "7480", "21,68K", "1,2M" (French or English decimal mark; K, M,
/// Md / bn display units), with half a unit of its last digit as tolerance.
pub fn card_number(text: &str) -> Option<(f64, f64)> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}')
        .collect();
    let (num, unit) = if let Some(n) = t.strip_suffix("Md").or_else(|| t.strip_suffix("bn")) {
        (n, 1e9)
    } else if let Some(n) = t.strip_suffix('K') {
        (n, 1e3)
    } else if let Some(n) = t.strip_suffix('M') {
        (n, 1e6)
    } else {
        (t.as_str(), 1.0)
    };
    let num = num.replace(',', ".");
    let x: f64 = num.parse().ok()?;
    let decimals = num.split('.').nth(1).map_or(0, str::len);
    let tol = if unit == 1.0 && decimals == 0 {
        1e-9
    } else {
        10f64.powi(-i32::try_from(decimals).ok()?) * unit / 2.0
    };
    Some((x * unit, tol))
}

/// The numbers a display reading shows as values (chart marks, card), each with its tolerance;
/// `None` when the reading has nothing to compare.
fn display_numbers(r: &Value) -> Option<Vec<(f64, f64)>> {
    if let Some(pts) = r["points"].as_array().filter(|p| !p.is_empty()) {
        let mut v = Vec::new();
        for p in pts {
            for k in ["value", "highlight"] {
                if let Some(x) = p[k].as_f64() {
                    v.push((x, 1e-6 * x.abs().max(1.0)));
                }
            }
        }
        return Some(v);
    }
    if let Some(a) = r["aria"].as_str() {
        // "Total Amount 7480." / "Total Amount 21,68K." (the card's accessible name)
        let a = a.trim_end_matches('.');
        if a.ends_with("(Vide)") || a.ends_with("(Blank)") {
            return Some(vec![]);
        }
        if let Some(x) = card_number(a.rsplit(' ').next().unwrap_or_default()) {
            return Some(vec![x]);
        }
    }
    None
}

/// The "Select all" item of a list slicer (Desktop's UI language: French or English).
fn is_select_all(t: &str) -> bool {
    let l = t.to_lowercase();
    l == "select all" || l == "sélectionner tout" || l == "tout sélectionner"
}

/// Whether a display reading is consistent with a query result: every displayed number is a
/// cell of the result (charts, cards), every listed item / first table cell a text of it.
fn display_matches(r: &Value, res: &Value) -> Option<bool> {
    if res.is_null() {
        return None;
    }
    if let Some(ds) = display_numbers(r) {
        let qs = result_numbers(res);
        return Some(ds.iter().all(|(d, tol)| qs.iter().any(|q| (d - q).abs() <= *tol)));
    }
    let texts = result_texts(res);
    if let Some(items) = r["items"].as_array().filter(|i| !i.is_empty()) {
        return Some(
            items
                .iter()
                .filter_map(|i| i["text"].as_str())
                .filter(|t| !t.is_empty() && !is_select_all(t))
                .all(|t| texts.contains(t)),
        );
    }
    if let Some(rows) = r["rows"].as_array().filter(|rows| rows.len() > 1) {
        // the header row first; the total row has no product
        return Some(
            rows.iter()
                .skip(1)
                .filter_map(|row| row["cells"].get(0).and_then(Value::as_str))
                .filter(|t| !t.is_empty() && *t != "Total")
                .all(|t| texts.contains(t)),
        );
    }
    None
}

#[derive(Default)]
struct Tally {
    visuals: usize,
    matching: usize,
    mismatching: usize,
    cache: usize,
}

/// The states of one recording (`raw`); its queries are added to `queries` (one id per distinct
/// text across recordings).
fn record_states(
    raw: &Path,
    plan: &Value,
    ids: &mut BTreeMap<String, String>,
    queries: &mut Vec<Value>,
    tally: &mut Tally,
) -> Result<(Vec<Value>, usize), String> {
    let readings = read_json(&raw.join("readings.json"))?;
    let results = read_json(&raw.join("results.json"))?;
    let trace = read_trace(&raw.join("trace.jsonl"))?;

    // distinct queries, in order of first appearance
    let by_text: BTreeMap<&str, &Value> = results["queries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| Some((q["query"].as_str()?, q)))
        .collect();
    for q in &trace {
        if ids.contains_key(&q.text) {
            continue;
        }
        let id = format!("q{:03}", queries.len() + 1);
        ids.insert(q.text.clone(), id.clone());
        let mut e = json!({"id": id, "dax": q.text,
                           "fields": query_fields(&q.text).into_iter().collect::<Vec<_>>()});
        match by_text.get(q.text.as_str()) {
            Some(r) if r.get("error").is_some() => e["error"] = json!(true),
            Some(r) => e["results"] = r["results"].clone(),
            None => e["not_replayed"] = json!(true),
        }
        queries.push(e);
    }
    let results_by_id: BTreeMap<String, Value> = queries
        .iter()
        .filter_map(|q| Some((q["id"].as_str()?.to_string(), q["results"].clone())))
        .collect();
    let result_of = |id: &str| -> &Value { results_by_id.get(id).unwrap_or(&Value::Null) };

    let mut pages: BTreeMap<String, &Value> = BTreeMap::new();
    for s in plan["scenarios"].as_array().into_iter().flatten() {
        if let Some(p) = s["page"].as_str() {
            pages.insert(p.to_string(), s);
        }
    }
    // navigation timeline: which page Desktop showed when
    let steps = readings["steps"].as_array().cloned().unwrap_or_default();
    let mut nav: Vec<(i64, String)> = Vec::new();
    for st in &steps {
        if let (Some(t), Some(p)) = (st["nav_start"].as_str().and_then(parse_iso), st["page"].as_str()) {
            nav.push((t, p.to_string()));
        }
        for a in st["actions"].as_array().into_iter().flatten() {
            if a["do"] == "page"
                && let (Some(t), Some(p)) = (a["at"].as_str().and_then(parse_iso), a["page"].as_str())
            {
                nav.push((t, p.to_string()));
            }
        }
        // back on the step's own page after a page action: the next step's nav_start covers it
    }
    nav.sort();
    nav.dedup();
    let page_at = |t: i64| -> Option<&str> {
        nav.iter()
            .take_while(|(at, _)| *at <= t)
            .last()
            .map(|(_, p)| p.as_str())
    };

    // per visual: every query given to it, in time order
    let mut given: BTreeMap<String, Vec<(i64, String)>> = BTreeMap::new();
    let mut unmatched = 0usize;
    for q in &trace {
        let Some(page) = page_at(q.at).and_then(|p| pages.get(p)) else {
            unmatched += 1;
            continue;
        };
        let fields = query_fields(&q.text);
        let id = ids.get(&q.text).cloned().unwrap_or_default();
        let mut hit = false;
        for v in page["visuals"].as_array().into_iter().flatten() {
            if visual_fields(v) == fields
                && let Some(n) = v["name"].as_str()
            {
                given.entry(n.to_string()).or_default().push((q.at, id.clone()));
                hit = true;
            }
        }
        if !hit {
            unmatched += 1;
        }
    }

    let mut states = Vec::new();
    for st in &steps {
        let (Some(id), Some(shown)) = (st["id"].as_str(), st["shown_page"].as_str()) else {
            continue;
        };
        let Some(page) = pages.get(shown) else {
            continue;
        };
        let end = st["end"].as_str().and_then(parse_iso).unwrap_or(i64::MAX);
        let start = st["start"].as_str().and_then(parse_iso).unwrap_or(0);
        let note = plan_step(&pages, st)
            .map(|s| s["note"].clone())
            .unwrap_or(Value::Null);
        let mut visuals = Vec::new();
        for v in page["visuals"].as_array().into_iter().flatten() {
            let Some(name) = v["name"].as_str() else {
                continue;
            };
            if v["type"] == "actionButton" {
                continue;
            }
            let reading = st["visuals"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| r["name"] == name)
                .cloned()
                .unwrap_or(Value::Null);
            visuals.push(visual_state(
                name,
                v,
                &reading,
                given.get(name).map(Vec::as_slice).unwrap_or_default(),
                (start, end),
                &result_of,
                tally,
            ));
        }
        let actions: Vec<Value> = st["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let mut a = a.clone();
                if let Some(o) = a.as_object_mut() {
                    o.remove("at");
                }
                a
            })
            .collect();
        states.push(
            json!({"id": id, "page": st["page"], "shown_page": shown, "note": note,
                           "actions": actions, "visuals": visuals}),
        );
    }

    Ok((states, unmatched))
}

pub fn collect(dir: &Path, out: Option<&Path>) -> Result<(), String> {
    let plan = read_json(&dir.join("plan.json"))?;
    // every recording DIR/raw*, in name order; a later one replaces the states it repeats
    let mut raws: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.join("readings.json").is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("raw"))
        })
        .collect();
    raws.sort();
    let Some(first) = raws.first() else {
        return Err(format!("{}: no recording (raw*/readings.json)", dir.display()));
    };
    let inst = read_json(&first.join("instance.json"))?;
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut queries: Vec<Value> = Vec::new();
    let mut tally = Tally::default();
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    let mut unmatched = 0usize;
    for raw in &raws {
        let (states, u) = record_states(raw, &plan, &mut ids, &mut queries, &mut tally)?;
        unmatched += u;
        for st in states {
            if let Some(id) = st["id"].as_str() {
                by_id.insert(id.to_string(), st);
            }
        }
    }
    // plan order
    let mut states = Vec::new();
    for s in plan["scenarios"].as_array().into_iter().flatten() {
        for st in s["steps"].as_array().into_iter().flatten() {
            if let Some(v) = st["id"].as_str().and_then(|id| by_id.remove(id)) {
                states.push(v);
            }
        }
    }
    let page_list: Vec<Value> = plan["scenarios"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["visuals"].as_array().is_some_and(|v| !v.is_empty()))
        .map(|s| {
            json!({"page": s["page"], "title": s["title"], "visuals": s["visuals"],
                   "interactions": s["interactions"]})
        })
        .collect();
    let corpus = json!({
        "version": 1,
        "generator": "cargo xtask oracle-gen interactions (xtask/src/oracles/interactions.rs)",
        "model": "model.bim",
        "desktop": inst["desktop"],
        "ui_culture": inst["ui_culture"],
        "recorded": inst["recorded"],
        "recordings": raws.len(),
        "page_filter": "every page filters the disconnected table Tag[Page] on its own id",
        "pages": page_list,
        "states": states,
        "queries": queries,
    });
    let out_path = match out {
        Some(p) => p.to_path_buf(),
        None => corpus_dir().join("corpus.json"),
    };
    if let Some(d) = out_path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        write_json(&d.join("model.bim"), &super::interactions::model(), 1)?;
    }
    write_json(&out_path, &corpus, 1)?;
    println!(
        "{} states, {} visual readings ({} match their query, {} do not, {} from the cache), \
         {} queries ({unmatched} not given to a visual) -> {}",
        states.len(),
        tally.visuals,
        tally.matching,
        tally.mismatching,
        tally.cache,
        queries.len(),
        out_path.display()
    );
    Ok(())
}

/// One visual of one state: its query, where the query comes from, and its display.
fn visual_state<'a>(
    name: &str,
    v: &Value,
    reading: &Value,
    history: &[(i64, String)],
    (start, end): (i64, i64),
    result_of: &impl Fn(&str) -> &'a Value,
    tally: &mut Tally,
) -> Value {
    let history: Vec<&(i64, String)> = history.iter().filter(|(t, _)| *t <= end).collect();
    let mut entry = Map::new();
    entry.insert("name".into(), json!(name));
    entry.insert("type".into(), v["type"].clone());
    let mut chosen = history.last().map(|(t, q)| (*t, q.clone(), "trace"));
    if let Some((_, q, _)) = &chosen
        && display_matches(reading, result_of(q)) == Some(false)
    {
        // back to an earlier state: the cached result the display shows
        let earlier = history
            .iter()
            .rev()
            .find(|(_, q2)| display_matches(reading, result_of(q2)) == Some(true));
        if let Some((t, q2)) = earlier {
            chosen = Some((*t, q2.clone(), "cache"));
        }
    }
    match &chosen {
        Some((t, q, from)) => {
            entry.insert("query".into(), json!(q));
            entry.insert("query_from".into(), json!(from));
            entry.insert("fresh".into(), json!(*t >= start && *from == "trace"));
            let m = display_matches(reading, result_of(q));
            entry.insert("display_matches_query".into(), json!(m));
            match m {
                Some(true) => tally.matching += 1,
                Some(false) => tally.mismatching += 1,
                None => {}
            }
            if *from == "cache" {
                tally.cache += 1;
            }
        }
        None => {
            entry.insert("query".into(), Value::Null);
            entry.insert("query_from".into(), json!("none"));
        }
    }
    entry.insert("display".into(), display_of(reading));
    tally.visuals += 1;
    Value::Object(entry)
}

fn plan_step<'a>(pages: &BTreeMap<String, &'a Value>, st: &Value) -> Option<&'a Value> {
    let page = pages.get(st["page"].as_str()?)?;
    page["steps"].as_array()?.iter().find(|s| s["id"] == st["id"])
}

/// The display part of a reading kept in the corpus: chart marks, table cells, slicer items and
/// inputs, and a card's shown number. Desktop's own UI words (the "Select all" item, the blank
/// marker, the slicers' header texts) are not kept: a select-all item becomes
/// `{"select_all": true, …}`, a blank card `{"card": null}`.
fn display_of(r: &Value) -> Value {
    if r.is_null() {
        return json!({"found": false});
    }
    let mut o = Map::new();
    for k in ["found", "points", "rows", "inputs"] {
        if let Some(v) = r.get(k) {
            o.insert(k.into(), v.clone());
        }
    }
    if let Some(items) = r["items"].as_array() {
        let items: Vec<Value> = items
            .iter()
            .map(|i| {
                if i["text"].as_str().is_some_and(is_select_all) {
                    json!({"select_all": true, "selected": i["selected"]})
                } else {
                    i.clone()
                }
            })
            .collect();
        o.insert("items".into(), Value::Array(items));
    }
    if let Some(a) = r["aria"].as_str() {
        // "<field> <shown value>." with the field name ours; keep the shown value only
        let a = a.trim_end_matches('.');
        let shown = a.rsplit(' ').next().unwrap_or_default();
        let card = if card_number(shown).is_some() {
            json!(shown)
        } else {
            Value::Null
        };
        o.insert("card".into(), card);
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times() {
        let a = parse_iso("2026-10-23T16:47:52.3133330+02:00").unwrap();
        let b = parse_iso("2026-10-23T14:47:52.313Z").unwrap();
        assert_eq!(a, b);
        assert_eq!(parse_iso("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_iso("not a date"), None);
    }

    #[test]
    fn fields_of_desktop_queries() {
        let q = "DEFINE\n\tVAR __DS0FilterTable = \n\t\tTREATAS({\"Toys\"}, 'Product'[Category])\n\n\tVAR __DS0Core = \n\t\tSUMMARIZECOLUMNS('Store'[Store], __DS0FilterTable, \"Total_Amount\", 'Sales'[Total Amount])\n\nEVALUATE\n\t__DS0Core\n\nORDER BY\n\t'Store'[Store]";
        let f: Vec<String> = query_fields(q).into_iter().collect();
        assert_eq!(f, ["Sales[Total Amount]", "Store[Store]"]);
        let h = "DEFINE\n\tVAR __H0FilterTable = \n\t\tTREATAS({\"Toys\"}, 'Product'[Category])\n\tVAR __DS0Core = SUMMARIZECOLUMNS('Product'[Color], \"A\", 'Sales'[Total Amount], \"B\", IGNORE(CALCULATE('Sales'[Total Amount], KEEPFILTERS(__H0FilterTable))))\nEVALUATE __DS0Core";
        let f: Vec<String> = query_fields(h).into_iter().collect();
        assert_eq!(f, ["Product[Color]", "Sales[Total Amount]"]);
        let s = "DEFINE VAR __DS0FilterTable = TREATAS({\"I02\"}, 'Tag'[Page]) VAR __DS0Core = VALUES('Product'[Category]) EVALUATE __DS0Core";
        let f: Vec<String> = query_fields(s).into_iter().collect();
        assert_eq!(f, ["Product[Category]"]);
    }

    #[test]
    fn display_matching() {
        let res = json!([{"columns": [], "rows": [{"Product[Category]": "Toys", "[A]": 7480}]}]);
        let m = |r: Value| display_matches(&r, &res);
        assert_eq!(m(json!({"points": [{"cat": "Toys", "value": 7480}]})), Some(true));
        assert_eq!(m(json!({"points": [{"cat": "Toys", "value": 7}]})), Some(false));
        assert_eq!(m(json!({"aria": "Total Amount 7480."})), Some(true));
        assert_eq!(m(json!({"items": [{"text": "Toys"}]})), Some(true));
        assert_eq!(m(json!({"text": "Toys"})), None);
        assert_eq!(m(json!({"aria": "Total Amount 7,48K."})), Some(true));
        assert_eq!(m(json!({"aria": "Total Amount 7,47K."})), Some(false));
        assert_eq!(card_number("21,68K"), Some((21680.0, 5.0)));
    }
}
