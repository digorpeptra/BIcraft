//! The process's local clock (`bicraft_engine::set_local_clock`, docs/formats/data-sources.md
//! "Clock"): `DateTime.LocalNow()` and `DateTimeZone.LocalNow()` add the offset it gives at the
//! evaluation's instant, UTC by default (the server's and the Power BI service's).
//!
//! The clock is process-wide, so this file is its own test binary and holds this one test.

use std::sync::Arc;

use bicraft_engine::{LocalClock, Session, local_clock, set_local_clock};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// Imports table `name` reading the clock, and answers local time minus UTC (minutes) and the
/// zone's hours of `DateTimeZone.LocalNow()`.
fn offsets(s: &mut Session, name: &str) -> (f64, f64) {
    let m = "let Source = #table(type table [U = datetime, L = datetime, Z = number], \
             {{DateTimeZone.RemoveZone(DateTimeZone.UtcNow()), DateTime.LocalNow(), \
             DateTimeZone.ZoneHours(DateTimeZone.LocalNow()) + DateTimeZone.ZoneMinutes(DateTimeZone.LocalNow()) / 60}}) \
             in Source";
    run(s, "table.import", json!({"name": name, "m": m}));
    let q = format!(
        "EVALUATE ROW(\"d\", ROUND((MAX('{name}'[L]) - MAX('{name}'[U])) * 1440, 0), \"z\", MAX('{name}'[Z]))"
    );
    let v = run(s, "dax.evaluate", json!({"query": q}));
    let row = &v["results"][0]["rows"][0];
    let num = |k: &str| {
        row[k]
            .as_f64()
            .unwrap_or_else(|| panic!("{k} is not a number: {v}"))
    };
    (num("[d]"), num("[z]"))
}

/// `NOW()` minus `UTCNOW()` in minutes, by `dax.evaluate` with these extra parameters.
fn dax_offset(s: &mut Session, extra: Value) -> f64 {
    let mut p = json!({"query": "EVALUATE ROW(\"d\", ROUND((NOW() - UTCNOW()) * 1440, 0))"});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    let v = run(s, "dax.evaluate", p);
    v["results"][0]["rows"][0]["[d]"]
        .as_f64()
        .unwrap_or_else(|| panic!("not a number: {v}"))
}

#[test]
fn local_now_follows_the_process_clock() {
    let mut s = Session::new();
    run(&mut s, "model.new", json!({"name": "Clock"}));

    // the default: UTC, as on a server and in the Power BI service
    assert!(matches!(local_clock(), LocalClock::Utc));
    assert_eq!(offsets(&mut s, "Utc"), (0.0, 0.0));
    assert_eq!(dax_offset(&mut s, json!({})), 0.0);

    // a fixed offset (`bicraft serve --local-offset -05:30`)
    set_local_clock(LocalClock::parse("-05:30", None).unwrap());
    assert_eq!(offsets(&mut s, "Fixed"), (-330.0, -5.5));
    // DAX's NOW() and UTCNOW() are apart by the same offset without `utc_offset_minutes`, with
    // or without `as_of`; the parameter, when given, wins
    assert_eq!(dax_offset(&mut s, json!({})), -330.0);
    assert_eq!(
        dax_offset(&mut s, json!({"as_of": "2026-03-10T12:00:00"})),
        -330.0
    );
    assert_eq!(dax_offset(&mut s, json!({"utc_offset_minutes": 60})), 60.0);
    assert_eq!(dax_offset(&mut s, json!({"utc_offset_minutes": 0})), 0.0);

    // a time zone read at the evaluation's instant (here a made-up zone: +2 h after 2001, as the
    // machine's zone in the desktop app and the CLI)
    set_local_clock(LocalClock::Zone(Arc::new(|unix_ms| {
        Some(if unix_ms > 1_000_000_000_000 { 120 } else { 60 })
    })));
    assert_eq!(offsets(&mut s, "Zone"), (120.0, 2.0));
    // the zone's offset at the `as_of` instant (daylight saving or a change of zone rules)
    assert_eq!(dax_offset(&mut s, json!({})), 120.0);
    assert_eq!(dax_offset(&mut s, json!({"as_of": "1990-06-01"})), 60.0);
    assert_eq!(dax_offset(&mut s, json!({"as_of": "2026-06-01"})), 120.0);

    // an unknown zone, or an offset M cannot hold, is UTC
    set_local_clock(LocalClock::Zone(Arc::new(|_| None)));
    assert_eq!(offsets(&mut s, "Unknown"), (0.0, 0.0));
    set_local_clock(LocalClock::Fixed(20 * 60));
    assert_eq!(offsets(&mut s, "TooFar"), (0.0, 0.0));
    set_local_clock(LocalClock::Utc);
}
