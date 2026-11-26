//! The local clock of M evaluations: what `DateTime.LocalNow()`, `DateTimeZone.LocalNow()` and
//! their `Fixed` forms add to the UTC clock (docs/formats/data-sources.md, "Clock").
//!
//! A process-wide setting, because the offset belongs to the machine (or the server's
//! configuration), not to a session. The default is UTC (offset 0), as in the Power BI service:
//! `bicraft serve` keeps it unless `--local-offset` says otherwise. The desktop app and the CLI
//! install the machine's time zone ([`LocalClock::Zone`] with
//! `bicraft_locale::calendar::machine_offset_minutes`), read at each evaluation, so a refresh
//! after a daylight-saving change gets the new offset.

use std::sync::{Arc, RwLock};

/// The largest offset M accepts on a `datetimezone` (±14:00), in minutes.
pub const MAX_OFFSET_MINUTES: i32 = 14 * 60;

/// The offset of the local time zone at a UTC instant (Unix milliseconds); `None` when it is
/// unknown (then UTC is used).
pub type OffsetFn = Arc<dyn Fn(i64) -> Option<i32> + Send + Sync>;

/// Where local time comes from.
#[derive(Clone, Default)]
pub enum LocalClock {
    /// Local time is UTC (the default, and the Power BI service's).
    #[default]
    Utc,
    /// A fixed offset in minutes (`bicraft serve --local-offset +02:00`).
    Fixed(i32),
    /// A time zone: the offset at the evaluation's instant (daylight saving included).
    Zone(OffsetFn),
}

impl std::fmt::Debug for LocalClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalClock::Utc => write!(f, "Utc"),
            LocalClock::Fixed(m) => write!(f, "Fixed({m})"),
            LocalClock::Zone(_) => write!(f, "Zone"),
        }
    }
}

impl LocalClock {
    /// The offset in minutes at `unix_ms`, always within ±[`MAX_OFFSET_MINUTES`] (anything else,
    /// or an unknown zone, is UTC).
    pub fn offset_at(&self, unix_ms: i64) -> i32 {
        let off = match self {
            LocalClock::Utc => Some(0),
            LocalClock::Fixed(m) => Some(*m),
            LocalClock::Zone(f) => f(unix_ms),
        };
        off.filter(|m| m.unsigned_abs() <= MAX_OFFSET_MINUTES.unsigned_abs())
            .unwrap_or(0)
    }

    /// Parses `--local-offset`: `±HH:MM` (`+02:00`), a whole number of minutes (`-300`), `UTC`
    /// or `Z` (0), or `machine` (also `local`): the time zone `zone` reads.
    pub fn parse(text: &str, zone: Option<OffsetFn>) -> Result<LocalClock, String> {
        let t = text.trim();
        let bad = || format!("`{text}`: give ±HH:MM (e.g. +02:00), minutes (e.g. -300), UTC or machine");
        if t.eq_ignore_ascii_case("z") || t.eq_ignore_ascii_case("utc") {
            return Ok(LocalClock::Utc);
        }
        if t.eq_ignore_ascii_case("machine") || t.eq_ignore_ascii_case("local") {
            return zone
                .map(LocalClock::Zone)
                .ok_or_else(|| format!("`{text}`: this build cannot read the machine's time zone"));
        }
        let two = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit());
        let minutes = match t.split_once(':') {
            Some((h, m)) => {
                let sign = match h.as_bytes().first() {
                    Some(b'+') => 1,
                    Some(b'-') => -1,
                    _ => return Err(bad()),
                };
                let h = h.get(1..).unwrap_or_default();
                if !two(h) || !two(m) {
                    return Err(bad());
                }
                let (h, m): (i32, i32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
                if m >= 60 {
                    return Err(bad());
                }
                sign * (h * 60 + m)
            }
            None => {
                let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
                if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(bad());
                }
                t.parse::<i32>().map_err(|_| bad())?
            }
        };
        if minutes.unsigned_abs() > MAX_OFFSET_MINUTES.unsigned_abs() {
            return Err(format!("`{text}`: offsets go from -14:00 to +14:00"));
        }
        Ok(if minutes == 0 {
            LocalClock::Utc
        } else {
            LocalClock::Fixed(minutes)
        })
    }

    /// How the clock is described in a server's start line.
    pub fn describe(&self) -> String {
        match self {
            LocalClock::Utc => "UTC".into(),
            LocalClock::Fixed(m) => {
                let sign = if *m < 0 { '-' } else { '+' };
                format!(
                    "UTC{sign}{:02}:{:02}",
                    m.unsigned_abs() / 60,
                    m.unsigned_abs() % 60
                )
            }
            LocalClock::Zone(_) => "the machine's time zone".into(),
        }
    }
}

static CLOCK: RwLock<LocalClock> = RwLock::new(LocalClock::Utc);

/// Sets the process's local clock (the desktop app and the CLI at start, `bicraft serve` from
/// `--local-offset`).
pub fn set_local_clock(clock: LocalClock) {
    *CLOCK.write().unwrap_or_else(|e| e.into_inner()) = clock;
}

/// The process's local clock.
pub fn local_clock() -> LocalClock {
    CLOCK.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The local offset, in minutes, at `serial_ms` (milliseconds since 1899-12-30 UTC, the M clock's
/// unit).
pub(crate) fn offset_at_serial_ms(serial_ms: i64) -> i32 {
    let unix_ms = serial_ms.saturating_sub(25_569 * 86_400_000);
    local_clock().offset_at(unix_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_parse() {
        let p = |t: &str| LocalClock::parse(t, None).map(|c| c.offset_at(0));
        assert_eq!(p("+02:00"), Ok(120));
        assert_eq!(p("-05:30"), Ok(-330));
        assert_eq!(p("+05:45"), Ok(345));
        assert_eq!(p("-300"), Ok(-300));
        assert_eq!(p("60"), Ok(60));
        assert_eq!(p("UTC"), Ok(0));
        assert_eq!(p("z"), Ok(0));
        assert!(p("+15:00").is_err());
        assert!(p("+02:60").is_err());
        assert!(p("900").is_err());
        assert!(p("02:00").is_err());
        assert!(p("+2:00").is_err());
        assert!(p("Europe/Paris").is_err());
        assert!(p("machine").is_err());
        assert!(p("").is_err());
        let zone: OffsetFn = Arc::new(|_| Some(60));
        assert_eq!(
            LocalClock::parse("machine", Some(zone)).map(|c| c.offset_at(0)),
            Ok(60)
        );
    }

    #[test]
    fn a_zone_is_read_at_the_instant_and_bounded() {
        // a made-up zone: +1 h before the Unix epoch, +2 h after (daylight saving)
        let zone = LocalClock::Zone(Arc::new(|ms| Some(if ms < 0 { 60 } else { 120 })));
        assert_eq!(zone.offset_at(-1), 60);
        assert_eq!(zone.offset_at(1), 120);
        assert_eq!(LocalClock::Zone(Arc::new(|_| None)).offset_at(0), 0);
        assert_eq!(LocalClock::Zone(Arc::new(|_| Some(24 * 60))).offset_at(0), 0);
        assert_eq!(LocalClock::Zone(Arc::new(|_| Some(i32::MIN))).offset_at(0), 0);
        assert_eq!(LocalClock::Fixed(i32::MIN).offset_at(0), 0);
        assert!(!LocalClock::Fixed(i32::MIN).describe().is_empty());
        assert_eq!(LocalClock::Fixed(-330).describe(), "UTC-05:30");
        assert_eq!(LocalClock::Utc.offset_at(0), 0);
    }
}
