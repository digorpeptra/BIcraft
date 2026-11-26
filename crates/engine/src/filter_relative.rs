//! Relative date and relative time filters on filter cards (the Filters pane's "Relative date
//! filtering" and "Relative time filtering"; docs/formats/pbir.md, "Relative date and time
//! filters"): `is in the last / next / this` N days, weeks, months, years (each also by calendar
//! period), today included or not, and N minutes or hours.
//!
//! A card stores the condition only: a `Between` on the column whose bounds are `Now`, `DateAdd`
//! and `DateSpan` expressions (public `semanticQuery` schema; `TimeUnit` 0 day, 1 week, 2 month,
//! 3 year, 6 minute, 7 hour), with the card's `type` `RelativeDate` or `RelativeTime`. The
//! settings are read back from those bounds: a bound shape this module does not write leaves the
//! card as kept (`other`).

use serde_json::{Value, json};

/// Largest N of a relative filter.
pub(crate) const MAX_COUNT: u32 = 10_000;
/// Deepest date expression read back.
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    Last,
    Next,
    This,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Days,
    Weeks,
    Months,
    Years,
    Minutes,
    Hours,
}

const UNITS: [Unit; 6] = [
    Unit::Days,
    Unit::Weeks,
    Unit::Months,
    Unit::Years,
    Unit::Minutes,
    Unit::Hours,
];

impl Unit {
    /// The semantic query's `TimeUnit`.
    fn code(self) -> u64 {
        match self {
            Unit::Days => 0,
            Unit::Weeks => 1,
            Unit::Months => 2,
            Unit::Years => 3,
            Unit::Minutes => 6,
            Unit::Hours => 7,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Unit::Days => "days",
            Unit::Weeks => "weeks",
            Unit::Months => "months",
            Unit::Years => "years",
            Unit::Minutes => "minutes",
            Unit::Hours => "hours",
        }
    }

    /// Minutes and hours: a relative time filter.
    pub(crate) fn is_time(self) -> bool {
        matches!(self, Unit::Minutes | Unit::Hours)
    }
}

/// A relative date or time condition. Canonical: `This` has `count` 1; `calendar` only for weeks,
/// months and years with `Last` / `Next`; `include_today` is `true` where it does not apply
/// (`This`, minutes, hours).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Relative {
    pub range: Range,
    pub count: u32,
    pub unit: Unit,
    pub calendar: bool,
    pub include_today: bool,
}

impl Relative {
    fn canonical(mut self) -> Self {
        if self.range == Range::This {
            self.count = 1;
            self.calendar = false;
            self.include_today = true;
        }
        if self.unit.is_time() {
            self.calendar = false;
            self.include_today = true;
        }
        if self.unit == Unit::Days {
            self.calendar = false;
        }
        self
    }

    /// The card's `type`.
    pub(crate) fn type_name(&self) -> &'static str {
        if self.unit.is_time() {
            "RelativeTime"
        } else {
            "RelativeDate"
        }
    }

    /// The unit as the `relative.unit` parameter names it.
    fn unit_text(&self) -> String {
        if self.calendar {
            format!("calendar_{}", self.unit.name())
        } else {
            self.unit.name().to_string()
        }
    }

    /// The condition as `*.filter`'s `relative` parameter writes it.
    pub(crate) fn to_json(self) -> Value {
        let range = match self.range {
            Range::Last => "last",
            Range::Next => "next",
            Range::This => "this",
        };
        json!({"range": range, "count": self.count, "unit": self.unit_text(),
               "include_today": self.include_today})
    }

    /// `in the last 3 days`, `in this month`, `in the next 2 calendar weeks (today excluded)`.
    pub(crate) fn describe(&self) -> String {
        let unit = self.unit_text().replace('_', " ");
        let mut t = match self.range {
            Range::This => format!("in this {}", unit.trim_end_matches('s')),
            Range::Last => format!("in the last {} {unit}", self.count),
            Range::Next => format!("in the next {} {unit}", self.count),
        };
        if !self.include_today {
            t.push_str(" (today excluded)");
        }
        t
    }

    /// The `Between` bounds Desktop's Filters pane writes for this condition (docs/formats/
    /// pbir.md, "Relative date and time filters"): `(LowerBound, UpperBound)`.
    fn bounds(&self) -> (Value, Value) {
        let u = self.unit.code();
        let n = i64::from(self.count);
        let now = || json!({"Now": {}});
        let add = |e: Value, amount: i64, unit: u64| json!({"DateAdd": {"Expression": e, "Amount": amount, "TimeUnit": unit}});
        let span = |e: Value, unit: u64| json!({"DateSpan": {"Expression": e, "TimeUnit": unit}});
        if self.unit.is_time() {
            return match self.range {
                Range::Last => (add(now(), -n, u), now()),
                Range::Next => (now(), add(now(), n, u)),
                Range::This => (span(now(), u), span(now(), u)),
            };
        }
        let today = || span(now(), 0);
        match (self.range, self.calendar, self.include_today) {
            (Range::This, _, _) => (span(now(), u), span(now(), u)),
            // the n periods ending today (the public example: last 10 years)
            (Range::Last, false, true) => (span(add(add(now(), 1, 0), -n, u), 0), today()),
            (Range::Last, false, false) => (span(add(now(), -n, u), 0), span(add(now(), -1, 0), 0)),
            (Range::Next, false, true) => (today(), span(add(add(now(), -1, 0), n, u), 0)),
            (Range::Next, false, false) => (span(add(now(), 1, 0), 0), span(add(now(), n, u), 0)),
            // the n whole calendar periods before (after) the current one
            (Range::Last, true, false) => (span(add(now(), -n, u), u), span(add(now(), -1, u), u)),
            (Range::Last, true, true) => (span(add(now(), -n, u), u), today()),
            (Range::Next, true, false) => (span(add(now(), 1, u), u), span(add(now(), n, u), u)),
            (Range::Next, true, true) => (today(), span(add(now(), n, u), u)),
        }
    }

    /// The `Between` condition on `column` (a `Column` expression).
    pub(crate) fn condition(&self, column: &Value) -> Value {
        let (lo, hi) = self.bounds();
        json!({"Between": {"Expression": column, "LowerBound": lo, "UpperBound": hi}})
    }
}

/// Parses the `relative` parameter: `{range: last|next|this, count?: 1..10000 (default 1), unit:
/// days|weeks|months|years|calendar_weeks|calendar_months|calendar_years|minutes|hours,
/// include_today?: bool (default true)}`.
pub(crate) fn parse_param(v: &Value, what: &str) -> Result<Relative, String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("{what} must be an object {{range, count, unit, include_today}}"))?;
    for k in o.keys() {
        if !["range", "count", "unit", "include_today"].contains(&k.as_str()) {
            return Err(format!(
                "{what}.{k} is not a setting (range, count, unit, include_today)"
            ));
        }
    }
    let range = match o
        .get("range")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
    {
        None => Range::Last,
        Some(r) => match r.as_str() {
            "last" => Range::Last,
            "next" => Range::Next,
            "this" => Range::This,
            _ => return Err(format!("{what}.range must be \"last\", \"next\" or \"this\"")),
        },
    };
    if o.get("range").is_some_and(|r| !r.is_string() && !r.is_null()) {
        return Err(format!("{what}.range must be \"last\", \"next\" or \"this\""));
    }
    let count = match o.get("count") {
        None | Some(Value::Null) => 1,
        Some(c) => c
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=MAX_COUNT).contains(n))
            .ok_or_else(|| format!("{what}.count must be a whole number from 1 to {MAX_COUNT}"))?,
    };
    let unit_text = o
        .get("unit")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{what}.unit is required (days, weeks, months, years, minutes, hours)"))?;
    let key: String = unit_text
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    let (calendar, base) = match key.strip_prefix("calendar") {
        Some(b) => (true, b.to_string()),
        None => (false, key),
    };
    let unit = UNITS
        .iter()
        .find(|u| u.name() == base || u.name().trim_end_matches('s') == base)
        .copied()
        .ok_or_else(|| {
            format!(
                "{what}.unit `{unit_text}` is not one of days, weeks, months, years, calendar_weeks, \
                 calendar_months, calendar_years, minutes, hours"
            )
        })?;
    if calendar && matches!(unit, Unit::Days | Unit::Minutes | Unit::Hours) {
        return Err(format!(
            "{what}.unit: calendar periods are weeks, months and years"
        ));
    }
    let include_today = match o.get("include_today") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{what}.include_today must be true or false")),
    };
    Ok(Relative {
        range,
        count,
        unit,
        calendar,
        include_today,
    }
    .canonical())
}

/// A date expression read back (`Now`, `DateAdd`, `DateSpan` only).
#[derive(Debug, PartialEq)]
enum Expr {
    Now,
    Add(Box<Expr>, i64, u64),
    Span(Box<Expr>, u64),
}

fn expr(v: &Value, depth: usize) -> Option<Expr> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (k, x) = crate::visual_props::only_key(v)?;
    let o = x.as_object()?;
    let unit = || o.get("TimeUnit").and_then(Value::as_u64);
    match k {
        "Now" if o.is_empty() => Some(Expr::Now),
        "DateAdd" if o.len() == 3 => {
            let amount = o.get("Amount")?;
            let a = amount.as_i64().or_else(|| {
                amount
                    .as_f64()
                    .filter(|f| f.fract() == 0.0 && f.abs() <= 1e9)
                    .map(|f| f as i64)
            })?;
            Some(Expr::Add(
                Box::new(expr(o.get("Expression")?, depth + 1)?),
                a,
                unit()?,
            ))
        }
        "DateSpan" if o.len() == 2 => Some(Expr::Span(
            Box::new(expr(o.get("Expression")?, depth + 1)?),
            unit()?,
        )),
        _ => None,
    }
}

/// Every `DateAdd` amount of `e`, made positive.
fn amounts(e: &Expr, out: &mut Vec<u32>) {
    match e {
        Expr::Now => {}
        Expr::Add(x, a, _) => {
            if let Some(n) = u32::try_from(a.unsigned_abs())
                .ok()
                .filter(|n| (1..=MAX_COUNT).contains(n))
            {
                out.push(n);
            }
            amounts(x, out);
        }
        Expr::Span(x, _) => amounts(x, out),
    }
}

/// The relative condition of `cond` (a `Between` on the column `is_column` accepts), for a card
/// of `type` `ty`; `None` for any other shape.
pub(crate) fn parse_condition(
    cond: &Value,
    ty: &str,
    is_column: impl Fn(&Value) -> bool,
) -> Option<Relative> {
    let (k, x) = crate::visual_props::only_key(cond)?;
    let o = x.as_object().filter(|o| k == "Between" && o.len() == 3)?;
    if !is_column(o.get("Expression")?) {
        return None;
    }
    let lo = expr(o.get("LowerBound")?, 0)?;
    let hi = expr(o.get("UpperBound")?, 0)?;
    let mut counts = vec![1];
    amounts(&lo, &mut counts);
    amounts(&hi, &mut counts);
    counts.sort_unstable();
    counts.dedup();
    let time = ty == "RelativeTime";
    for unit in UNITS.iter().filter(|u| u.is_time() == time) {
        for range in [Range::Last, Range::Next, Range::This] {
            for calendar in [false, true] {
                for include_today in [true, false] {
                    for &count in &counts {
                        let r = Relative {
                            range,
                            count,
                            unit: *unit,
                            calendar,
                            include_today,
                        };
                        if r.canonical() != r {
                            continue;
                        }
                        let (a, b) = r.bounds();
                        if expr(&a, 0).as_ref() == Some(&lo) && expr(&b, 0).as_ref() == Some(&hi) {
                            return Some(r);
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col() -> Value {
        json!({"Column": {"Expression": {"SourceRef": {"Source": "d"}}, "Property": "Day"}})
    }

    #[test]
    fn the_public_last_n_shape() {
        // docs/formats/pbir.md: last 10 years, today included
        let r = parse_param(
            &json!({"range": "last", "count": 10, "unit": "years"}),
            "relative",
        )
        .unwrap();
        let c = r.condition(&col());
        assert_eq!(
            c["Between"]["LowerBound"],
            json!({"DateSpan": {"Expression": {"DateAdd": {"Expression": {"DateAdd": {"Expression": {"Now": {}},
                "Amount": 1, "TimeUnit": 0}}, "Amount": -10, "TimeUnit": 3}}, "TimeUnit": 0}})
        );
        assert_eq!(
            c["Between"]["UpperBound"],
            json!({"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 0}})
        );
        assert_eq!(r.type_name(), "RelativeDate");
    }

    #[test]
    fn every_setting_reads_back() {
        let is_col = |v: &Value| *v == col();
        for unit in [
            "days",
            "weeks",
            "months",
            "years",
            "calendar_weeks",
            "calendar_months",
            "calendar_years",
            "minutes",
            "hours",
        ] {
            for range in ["last", "next", "this"] {
                for count in [1, 2, 7, 10_000] {
                    for today in [true, false] {
                        let p = json!({"range": range, "count": count, "unit": unit, "include_today": today});
                        let r = parse_param(&p, "relative").unwrap();
                        let back = parse_condition(&r.condition(&col()), r.type_name(), is_col);
                        assert_eq!(back, Some(r), "{p}");
                        let other = if r.type_name() == "RelativeDate" {
                            "RelativeTime"
                        } else {
                            "RelativeDate"
                        };
                        assert_eq!(parse_condition(&r.condition(&col()), other, is_col), None, "{p}");
                    }
                }
            }
        }
    }

    #[test]
    fn bad_settings_are_refused() {
        for bad in [
            json!(3),
            json!({"unit": "days", "extra": 1}),
            json!({"range": "before", "unit": "days"}),
            json!({"range": 3, "unit": "days"}),
            json!({"unit": "fortnights"}),
            json!({"unit": "calendar_days"}),
            json!({"unit": "calendar_hours"}),
            json!({"count": 0, "unit": "days"}),
            json!({"count": 10_001, "unit": "days"}),
            json!({"count": 1.5, "unit": "days"}),
            json!({"count": -1, "unit": "days"}),
            json!({"unit": "days", "include_today": "yes"}),
            json!({"range": "last"}),
        ] {
            assert!(parse_param(&bad, "relative").is_err(), "{bad}");
        }
        // anything else in the bounds is not read
        let is_col = |_: &Value| true;
        for bad in [
            json!({"Between": {"Expression": col(), "LowerBound": {"Now": {"x": 1}}, "UpperBound": {"Now": {}}}}),
            json!({"Between": {"Expression": col(), "LowerBound": {"Literal": {"Value": "1L"}}, "UpperBound": {"Now": {}}}}),
            json!({"Comparison": {"ComparisonKind": 0, "Left": col(), "Right": {"Now": {}}}}),
            json!({"Between": {"Expression": col(), "LowerBound": {"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 0}},
                "UpperBound": {"DateSpan": {"Expression": {"Now": {}}, "TimeUnit": 2}}}}),
        ] {
            assert_eq!(parse_condition(&bad, "RelativeDate", is_col), None, "{bad}");
        }
        let mut deep = json!({"Now": {}});
        for _ in 0..50 {
            deep = json!({"DateSpan": {"Expression": deep, "TimeUnit": 0}});
        }
        let c = json!({"Between": {"Expression": col(), "LowerBound": deep, "UpperBound": deep}});
        assert_eq!(parse_condition(&c, "RelativeDate", is_col), None);
    }
}
