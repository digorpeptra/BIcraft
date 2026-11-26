//! The editor's row, column, text, number and date transforms beyond the first ribbon's
//! ([`super::transform`]): Remove and Keep rows, Fill, Replace errors, Transpose, Reverse rows,
//! Use headers as first row, Move (reorder), Duplicate column, Index column, Merge columns,
//! Conditional column, and the Transform / Add column menus' Format, Extract, Standard,
//! Scientific, Trigonometry, Rounding, Information, Date, Time and Duration items, each in place
//! (`Table.TransformColumns`) or as a new column (`Table.AddColumn`).
//!
//! The function shapes follow the public Power Query M function reference (Microsoft Learn:
//! `Table.Skip`, `Table.RemoveLastN`, `Table.FirstN`, `Table.LastN`, `Table.Range`,
//! `Table.AlternateRows`, `Table.RemoveRowsWithErrors`, `Table.SelectRowsWithErrors`,
//! `Table.ReplaceErrorValues`, `Table.FillDown`, `Table.FillUp`, `Table.Transpose`,
//! `Table.ReverseRows`, `Table.DemoteHeaders`, `Table.ReorderColumns`, `Table.DuplicateColumn`,
//! `Table.AddIndexColumn`, `Table.CombineColumns`, `Table.TransformColumns`, `Table.AddColumn`
//! and the `Text.*`, `Number.*`, `Date.*`, `DateTime.*` and `Time.*` functions they call). The
//! English step and column names are BIcraft's choice in the style of the first ribbon's; they are
//! not compared with what Desktop writes.

use super::transform::{FilterOp, Literal, MAX_COLUMNS, field, ident, text};

/// Largest row count or position a row transform takes.
pub const MAX_ROWS: u64 = 1_000_000_000;
/// Most rules of one conditional column.
pub const MAX_RULES: usize = 100;

/// A per-value operation of the Transform and Add column menus.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnOp {
    Lower,
    Upper,
    Proper,
    Trim,
    Clean,
    Prefix(String),
    Suffix(String),
    Length,
    FirstChars(u64),
    LastChars(u64),
    /// Characters from `start` (0-based), `count` of them.
    TextRange {
        start: u64,
        count: u64,
    },
    BeforeDelimiter(String),
    AfterDelimiter(String),
    Year,
    Month,
    Day,
    Quarter,
    MonthName,
    DayName,
    DayOfWeek,
    WeekOfYear,
    DayOfYear,
    StartOfMonth,
    EndOfMonth,
    StartOfYear,
    EndOfYear,
    /// The date part of a date/time.
    DateOnly,
    Hour,
    Minute,
    Second,
    /// Round to these decimal places.
    Round(u32),
    RoundUp,
    RoundDown,
    Abs,
    Add(f64),
    Subtract(f64),
    Multiply(f64),
    Divide(f64),
    /// Text between a start and an end delimiter.
    BetweenDelimiters(String, String),
    IntegerDivide(f64),
    Modulo(f64),
    /// `n` percent of the value.
    Percentage(f64),
    /// The value as a percentage of `n`.
    PercentOf(f64),
    Power(f64),
    SquareRoot,
    Exponent,
    Log10,
    Ln,
    Factorial,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    IsEven,
    IsOdd,
    Sign,
    /// Today's date minus the date (a duration).
    Age,
    StartOfQuarter,
    EndOfQuarter,
    StartOfWeek,
    EndOfWeek,
    StartOfDay,
    EndOfDay,
    DaysInMonth,
    WeekOfMonth,
    /// The time part of a date/time.
    TimeOnly,
    StartOfHour,
    EndOfHour,
    DurationDays,
    DurationHours,
    DurationMinutes,
    DurationSeconds,
    /// Total days divided by 365.
    TotalYears,
    TotalDays,
    TotalHours,
    TotalMinutes,
    TotalSeconds,
}

/// The operation names (`op` of `column_op`) and what each takes: `""` nothing, `"text"`,
/// `"texts"` (`text` and `end`), `"count"`, `"range"` (start and count), `"number"`, `"digits"`.
pub const COLUMN_OPS: [(&str, &str); 79] = [
    ("lower", ""),
    ("upper", ""),
    ("proper", ""),
    ("trim", ""),
    ("clean", ""),
    ("prefix", "text"),
    ("suffix", "text"),
    ("length", ""),
    ("first_chars", "count"),
    ("last_chars", "count"),
    ("text_range", "range"),
    ("before_delimiter", "text"),
    ("after_delimiter", "text"),
    ("year", ""),
    ("month", ""),
    ("day", ""),
    ("quarter", ""),
    ("month_name", ""),
    ("day_name", ""),
    ("day_of_week", ""),
    ("week_of_year", ""),
    ("day_of_year", ""),
    ("start_of_month", ""),
    ("end_of_month", ""),
    ("start_of_year", ""),
    ("end_of_year", ""),
    ("date_only", ""),
    ("hour", ""),
    ("minute", ""),
    ("second", ""),
    ("round", "digits"),
    ("round_up", ""),
    ("round_down", ""),
    ("abs", ""),
    ("add", "number"),
    ("subtract", "number"),
    ("multiply", "number"),
    ("divide", "number"),
    ("between_delimiters", "texts"),
    ("integer_divide", "number"),
    ("modulo", "number"),
    ("percentage", "number"),
    ("percent_of", "number"),
    ("power", "number"),
    ("square_root", ""),
    ("exponent", ""),
    ("log10", ""),
    ("ln", ""),
    ("factorial", ""),
    ("sin", ""),
    ("cos", ""),
    ("tan", ""),
    ("asin", ""),
    ("acos", ""),
    ("atan", ""),
    ("is_even", ""),
    ("is_odd", ""),
    ("sign", ""),
    ("age", ""),
    ("start_of_quarter", ""),
    ("end_of_quarter", ""),
    ("start_of_week", ""),
    ("end_of_week", ""),
    ("start_of_day", ""),
    ("end_of_day", ""),
    ("days_in_month", ""),
    ("week_of_month", ""),
    ("time_only", ""),
    ("start_of_hour", ""),
    ("end_of_hour", ""),
    ("duration_days", ""),
    ("duration_hours", ""),
    ("duration_minutes", ""),
    ("duration_seconds", ""),
    ("total_years", ""),
    ("total_days", ""),
    ("total_hours", ""),
    ("total_minutes", ""),
    ("total_seconds", ""),
];

/// The arguments a [`ColumnOp`] is built from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpArgs {
    pub text: Option<String>,
    /// The end delimiter of `between_delimiters`.
    pub end: Option<String>,
    pub count: Option<u64>,
    pub start: Option<u64>,
    pub number: Option<f64>,
}

fn num(n: f64) -> String {
    Literal::Number(n).m()
}

impl ColumnOp {
    /// The operation `name` with its arguments.
    pub fn from_name(name: &str, a: &OpArgs) -> Result<Self, String> {
        let need_text = |what: &str| {
            a.text
                .clone()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| format!("`{name}` needs `{what}`"))
        };
        let need_count = || {
            a.count
                .filter(|n| (1..=MAX_ROWS).contains(n))
                .ok_or_else(|| format!("`{name}` needs `count` (1 to {MAX_ROWS})"))
        };
        let need_number = || {
            a.number
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("`{name}` needs `number`"))
        };
        Ok(match name {
            "lower" => ColumnOp::Lower,
            "upper" => ColumnOp::Upper,
            "proper" => ColumnOp::Proper,
            "trim" => ColumnOp::Trim,
            "clean" => ColumnOp::Clean,
            "prefix" => ColumnOp::Prefix(need_text("text")?),
            "suffix" => ColumnOp::Suffix(need_text("text")?),
            "length" => ColumnOp::Length,
            "first_chars" => ColumnOp::FirstChars(need_count()?),
            "last_chars" => ColumnOp::LastChars(need_count()?),
            "text_range" => ColumnOp::TextRange {
                start: a.start.filter(|n| *n <= MAX_ROWS).unwrap_or(0),
                count: need_count()?,
            },
            "before_delimiter" => ColumnOp::BeforeDelimiter(need_text("text")?),
            "after_delimiter" => ColumnOp::AfterDelimiter(need_text("text")?),
            "year" => ColumnOp::Year,
            "month" => ColumnOp::Month,
            "day" => ColumnOp::Day,
            "quarter" => ColumnOp::Quarter,
            "month_name" => ColumnOp::MonthName,
            "day_name" => ColumnOp::DayName,
            "day_of_week" => ColumnOp::DayOfWeek,
            "week_of_year" => ColumnOp::WeekOfYear,
            "day_of_year" => ColumnOp::DayOfYear,
            "start_of_month" => ColumnOp::StartOfMonth,
            "end_of_month" => ColumnOp::EndOfMonth,
            "start_of_year" => ColumnOp::StartOfYear,
            "end_of_year" => ColumnOp::EndOfYear,
            "date_only" => ColumnOp::DateOnly,
            "hour" => ColumnOp::Hour,
            "minute" => ColumnOp::Minute,
            "second" => ColumnOp::Second,
            "round" => {
                let d = a.count.unwrap_or(0);
                ColumnOp::Round(
                    u32::try_from(d)
                        .ok()
                        .filter(|d| *d <= 15)
                        .ok_or("`round` takes 0 to 15 decimal places (`count`)")?,
                )
            }
            "round_up" => ColumnOp::RoundUp,
            "round_down" => ColumnOp::RoundDown,
            "abs" => ColumnOp::Abs,
            "add" => ColumnOp::Add(need_number()?),
            "subtract" => ColumnOp::Subtract(need_number()?),
            "multiply" => ColumnOp::Multiply(need_number()?),
            "divide" => {
                let n = need_number()?;
                if n == 0.0 {
                    return Err("`divide` cannot divide by 0".into());
                }
                ColumnOp::Divide(n)
            }
            "between_delimiters" => {
                let end = a
                    .end
                    .clone()
                    .filter(|t| !t.is_empty())
                    .ok_or("`between_delimiters` needs `end` (the end delimiter)")?;
                ColumnOp::BetweenDelimiters(need_text("text")?, end)
            }
            "integer_divide" | "modulo" | "percent_of" => {
                let n = need_number()?;
                if n == 0.0 {
                    return Err(format!("`{name}` cannot divide by 0"));
                }
                match name {
                    "integer_divide" => ColumnOp::IntegerDivide(n),
                    "modulo" => ColumnOp::Modulo(n),
                    _ => ColumnOp::PercentOf(n),
                }
            }
            "percentage" => ColumnOp::Percentage(need_number()?),
            "power" => ColumnOp::Power(need_number()?),
            "square_root" => ColumnOp::SquareRoot,
            "exponent" => ColumnOp::Exponent,
            "log10" => ColumnOp::Log10,
            "ln" => ColumnOp::Ln,
            "factorial" => ColumnOp::Factorial,
            "sin" => ColumnOp::Sin,
            "cos" => ColumnOp::Cos,
            "tan" => ColumnOp::Tan,
            "asin" => ColumnOp::Asin,
            "acos" => ColumnOp::Acos,
            "atan" => ColumnOp::Atan,
            "is_even" => ColumnOp::IsEven,
            "is_odd" => ColumnOp::IsOdd,
            "sign" => ColumnOp::Sign,
            "age" => ColumnOp::Age,
            "start_of_quarter" => ColumnOp::StartOfQuarter,
            "end_of_quarter" => ColumnOp::EndOfQuarter,
            "start_of_week" => ColumnOp::StartOfWeek,
            "end_of_week" => ColumnOp::EndOfWeek,
            "start_of_day" => ColumnOp::StartOfDay,
            "end_of_day" => ColumnOp::EndOfDay,
            "days_in_month" => ColumnOp::DaysInMonth,
            "week_of_month" => ColumnOp::WeekOfMonth,
            "time_only" => ColumnOp::TimeOnly,
            "start_of_hour" => ColumnOp::StartOfHour,
            "end_of_hour" => ColumnOp::EndOfHour,
            "duration_days" => ColumnOp::DurationDays,
            "duration_hours" => ColumnOp::DurationHours,
            "duration_minutes" => ColumnOp::DurationMinutes,
            "duration_seconds" => ColumnOp::DurationSeconds,
            "total_years" => ColumnOp::TotalYears,
            "total_days" => ColumnOp::TotalDays,
            "total_hours" => ColumnOp::TotalHours,
            "total_minutes" => ColumnOp::TotalMinutes,
            "total_seconds" => ColumnOp::TotalSeconds,
            other => {
                return Err(format!(
                    "unknown `op` `{other}` (expected {})",
                    COLUMN_OPS.map(|x| x.0).join(", ")
                ));
            }
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            ColumnOp::Lower => "lower",
            ColumnOp::Upper => "upper",
            ColumnOp::Proper => "proper",
            ColumnOp::Trim => "trim",
            ColumnOp::Clean => "clean",
            ColumnOp::Prefix(_) => "prefix",
            ColumnOp::Suffix(_) => "suffix",
            ColumnOp::Length => "length",
            ColumnOp::FirstChars(_) => "first_chars",
            ColumnOp::LastChars(_) => "last_chars",
            ColumnOp::TextRange { .. } => "text_range",
            ColumnOp::BeforeDelimiter(_) => "before_delimiter",
            ColumnOp::AfterDelimiter(_) => "after_delimiter",
            ColumnOp::Year => "year",
            ColumnOp::Month => "month",
            ColumnOp::Day => "day",
            ColumnOp::Quarter => "quarter",
            ColumnOp::MonthName => "month_name",
            ColumnOp::DayName => "day_name",
            ColumnOp::DayOfWeek => "day_of_week",
            ColumnOp::WeekOfYear => "week_of_year",
            ColumnOp::DayOfYear => "day_of_year",
            ColumnOp::StartOfMonth => "start_of_month",
            ColumnOp::EndOfMonth => "end_of_month",
            ColumnOp::StartOfYear => "start_of_year",
            ColumnOp::EndOfYear => "end_of_year",
            ColumnOp::DateOnly => "date_only",
            ColumnOp::Hour => "hour",
            ColumnOp::Minute => "minute",
            ColumnOp::Second => "second",
            ColumnOp::Round(_) => "round",
            ColumnOp::RoundUp => "round_up",
            ColumnOp::RoundDown => "round_down",
            ColumnOp::Abs => "abs",
            ColumnOp::Add(_) => "add",
            ColumnOp::Subtract(_) => "subtract",
            ColumnOp::Multiply(_) => "multiply",
            ColumnOp::Divide(_) => "divide",
            ColumnOp::BetweenDelimiters(..) => "between_delimiters",
            ColumnOp::IntegerDivide(_) => "integer_divide",
            ColumnOp::Modulo(_) => "modulo",
            ColumnOp::Percentage(_) => "percentage",
            ColumnOp::PercentOf(_) => "percent_of",
            ColumnOp::Power(_) => "power",
            ColumnOp::SquareRoot => "square_root",
            ColumnOp::Exponent => "exponent",
            ColumnOp::Log10 => "log10",
            ColumnOp::Ln => "ln",
            ColumnOp::Factorial => "factorial",
            ColumnOp::Sin => "sin",
            ColumnOp::Cos => "cos",
            ColumnOp::Tan => "tan",
            ColumnOp::Asin => "asin",
            ColumnOp::Acos => "acos",
            ColumnOp::Atan => "atan",
            ColumnOp::IsEven => "is_even",
            ColumnOp::IsOdd => "is_odd",
            ColumnOp::Sign => "sign",
            ColumnOp::Age => "age",
            ColumnOp::StartOfQuarter => "start_of_quarter",
            ColumnOp::EndOfQuarter => "end_of_quarter",
            ColumnOp::StartOfWeek => "start_of_week",
            ColumnOp::EndOfWeek => "end_of_week",
            ColumnOp::StartOfDay => "start_of_day",
            ColumnOp::EndOfDay => "end_of_day",
            ColumnOp::DaysInMonth => "days_in_month",
            ColumnOp::WeekOfMonth => "week_of_month",
            ColumnOp::TimeOnly => "time_only",
            ColumnOp::StartOfHour => "start_of_hour",
            ColumnOp::EndOfHour => "end_of_hour",
            ColumnOp::DurationDays => "duration_days",
            ColumnOp::DurationHours => "duration_hours",
            ColumnOp::DurationMinutes => "duration_minutes",
            ColumnOp::DurationSeconds => "duration_seconds",
            ColumnOp::TotalYears => "total_years",
            ColumnOp::TotalDays => "total_days",
            ColumnOp::TotalHours => "total_hours",
            ColumnOp::TotalMinutes => "total_minutes",
            ColumnOp::TotalSeconds => "total_seconds",
        }
    }

    /// The M applied to value `x` (`_` in place, `[Column]` in a new column).
    fn apply(&self, x: &str) -> String {
        let call = |f: &str| format!("{f}({x})");
        match self {
            ColumnOp::Lower => call("Text.Lower"),
            ColumnOp::Upper => call("Text.Upper"),
            ColumnOp::Proper => call("Text.Proper"),
            ColumnOp::Trim => call("Text.Trim"),
            ColumnOp::Clean => call("Text.Clean"),
            ColumnOp::Prefix(p) => format!("{} & {x}", text(p)),
            ColumnOp::Suffix(s) => format!("{x} & {}", text(s)),
            ColumnOp::Length => call("Text.Length"),
            ColumnOp::FirstChars(n) => format!("Text.Start({x}, {n})"),
            ColumnOp::LastChars(n) => format!("Text.End({x}, {n})"),
            ColumnOp::TextRange { start, count } => format!("Text.Middle({x}, {start}, {count})"),
            ColumnOp::BeforeDelimiter(d) => format!("Text.BeforeDelimiter({x}, {})", text(d)),
            ColumnOp::AfterDelimiter(d) => format!("Text.AfterDelimiter({x}, {})", text(d)),
            ColumnOp::Year => call("Date.Year"),
            ColumnOp::Month => call("Date.Month"),
            ColumnOp::Day => call("Date.Day"),
            ColumnOp::Quarter => call("Date.QuarterOfYear"),
            ColumnOp::MonthName => call("Date.MonthName"),
            ColumnOp::DayName => call("Date.DayOfWeekName"),
            ColumnOp::DayOfWeek => call("Date.DayOfWeek"),
            ColumnOp::WeekOfYear => call("Date.WeekOfYear"),
            ColumnOp::DayOfYear => call("Date.DayOfYear"),
            ColumnOp::StartOfMonth => call("Date.StartOfMonth"),
            ColumnOp::EndOfMonth => call("Date.EndOfMonth"),
            ColumnOp::StartOfYear => call("Date.StartOfYear"),
            ColumnOp::EndOfYear => call("Date.EndOfYear"),
            ColumnOp::DateOnly => call("DateTime.Date"),
            ColumnOp::Hour => call("Time.Hour"),
            ColumnOp::Minute => call("Time.Minute"),
            ColumnOp::Second => call("Time.Second"),
            ColumnOp::Round(d) => format!("Number.Round({x}, {d})"),
            ColumnOp::RoundUp => call("Number.RoundUp"),
            ColumnOp::RoundDown => call("Number.RoundDown"),
            ColumnOp::Abs => call("Number.Abs"),
            ColumnOp::Add(n) => format!("{x} + {}", num(*n)),
            ColumnOp::Subtract(n) => format!("{x} - {}", num(*n)),
            ColumnOp::Multiply(n) => format!("{x} * {}", num(*n)),
            ColumnOp::Divide(n) => format!("{x} / {}", num(*n)),
            ColumnOp::BetweenDelimiters(s, e) => {
                format!("Text.BetweenDelimiters({x}, {}, {})", text(s), text(e))
            }
            ColumnOp::IntegerDivide(n) => format!("Number.IntegerDivide({x}, {})", num(*n)),
            ColumnOp::Modulo(n) => format!("Number.Mod({x}, {})", num(*n)),
            ColumnOp::Percentage(n) => format!("{x} * {} / 100", num(*n)),
            ColumnOp::PercentOf(n) => format!("{x} / {} * 100", num(*n)),
            ColumnOp::Power(n) => format!("Number.Power({x}, {})", num(*n)),
            ColumnOp::SquareRoot => call("Number.Sqrt"),
            ColumnOp::Exponent => call("Number.Exp"),
            ColumnOp::Log10 => call("Number.Log10"),
            ColumnOp::Ln => call("Number.Ln"),
            ColumnOp::Factorial => call("Number.Factorial"),
            ColumnOp::Sin => call("Number.Sin"),
            ColumnOp::Cos => call("Number.Cos"),
            ColumnOp::Tan => call("Number.Tan"),
            ColumnOp::Asin => call("Number.Asin"),
            ColumnOp::Acos => call("Number.Acos"),
            ColumnOp::Atan => call("Number.Atan"),
            ColumnOp::IsEven => call("Number.IsEven"),
            ColumnOp::IsOdd => call("Number.IsOdd"),
            ColumnOp::Sign => call("Number.Sign"),
            ColumnOp::Age => format!("Date.From(DateTime.LocalNow()) - {x}"),
            ColumnOp::StartOfQuarter => call("Date.StartOfQuarter"),
            ColumnOp::EndOfQuarter => call("Date.EndOfQuarter"),
            ColumnOp::StartOfWeek => call("Date.StartOfWeek"),
            ColumnOp::EndOfWeek => call("Date.EndOfWeek"),
            ColumnOp::StartOfDay => call("Date.StartOfDay"),
            ColumnOp::EndOfDay => call("Date.EndOfDay"),
            ColumnOp::DaysInMonth => call("Date.DaysInMonth"),
            ColumnOp::WeekOfMonth => call("Date.WeekOfMonth"),
            ColumnOp::TimeOnly => call("DateTime.Time"),
            ColumnOp::StartOfHour => call("Time.StartOfHour"),
            ColumnOp::EndOfHour => call("Time.EndOfHour"),
            ColumnOp::DurationDays => call("Duration.Days"),
            ColumnOp::DurationHours => call("Duration.Hours"),
            ColumnOp::DurationMinutes => call("Duration.Minutes"),
            ColumnOp::DurationSeconds => call("Duration.Seconds"),
            ColumnOp::TotalYears => format!("Duration.TotalDays({x}) / 365"),
            ColumnOp::TotalDays => call("Duration.TotalDays"),
            ColumnOp::TotalHours => call("Duration.TotalHours"),
            ColumnOp::TotalMinutes => call("Duration.TotalMinutes"),
            ColumnOp::TotalSeconds => call("Duration.TotalSeconds"),
        }
    }

    /// The function `Table.TransformColumns` takes: the library function itself when the
    /// operation is a one-argument call, else `each …` over `_`.
    fn function(&self) -> String {
        let a = self.apply("_");
        match a.strip_suffix("(_)") {
            Some(f) if !f.contains(' ') => f.to_string(),
            _ => format!("each {a}"),
        }
    }

    /// The type of the values it gives.
    fn ty(&self) -> &'static str {
        match self {
            ColumnOp::Lower
            | ColumnOp::Upper
            | ColumnOp::Proper
            | ColumnOp::Trim
            | ColumnOp::Clean
            | ColumnOp::Prefix(_)
            | ColumnOp::Suffix(_)
            | ColumnOp::FirstChars(_)
            | ColumnOp::LastChars(_)
            | ColumnOp::TextRange { .. }
            | ColumnOp::BeforeDelimiter(_)
            | ColumnOp::AfterDelimiter(_)
            | ColumnOp::MonthName
            | ColumnOp::DayName => "type text",
            ColumnOp::Length
            | ColumnOp::Year
            | ColumnOp::Month
            | ColumnOp::Day
            | ColumnOp::Quarter
            | ColumnOp::DayOfWeek
            | ColumnOp::WeekOfYear
            | ColumnOp::DayOfYear
            | ColumnOp::Hour
            | ColumnOp::Minute
            | ColumnOp::RoundUp
            | ColumnOp::RoundDown => "Int64.Type",
            ColumnOp::StartOfMonth
            | ColumnOp::EndOfMonth
            | ColumnOp::StartOfYear
            | ColumnOp::EndOfYear
            | ColumnOp::DateOnly => "type date",
            ColumnOp::Second
            | ColumnOp::Round(_)
            | ColumnOp::Abs
            | ColumnOp::Add(_)
            | ColumnOp::Subtract(_)
            | ColumnOp::Multiply(_)
            | ColumnOp::Divide(_)
            | ColumnOp::Modulo(_)
            | ColumnOp::Percentage(_)
            | ColumnOp::PercentOf(_)
            | ColumnOp::Power(_)
            | ColumnOp::SquareRoot
            | ColumnOp::Exponent
            | ColumnOp::Log10
            | ColumnOp::Ln
            | ColumnOp::Factorial
            | ColumnOp::Sin
            | ColumnOp::Cos
            | ColumnOp::Tan
            | ColumnOp::Asin
            | ColumnOp::Acos
            | ColumnOp::Atan
            | ColumnOp::DurationSeconds
            | ColumnOp::TotalYears
            | ColumnOp::TotalDays
            | ColumnOp::TotalHours
            | ColumnOp::TotalMinutes
            | ColumnOp::TotalSeconds => "type number",
            ColumnOp::BetweenDelimiters(..) => "type text",
            ColumnOp::IntegerDivide(_)
            | ColumnOp::Sign
            | ColumnOp::DaysInMonth
            | ColumnOp::WeekOfMonth
            | ColumnOp::DurationDays
            | ColumnOp::DurationHours
            | ColumnOp::DurationMinutes => "Int64.Type",
            ColumnOp::IsEven | ColumnOp::IsOdd => "type logical",
            ColumnOp::Age => "type duration",
            ColumnOp::StartOfQuarter
            | ColumnOp::EndOfQuarter
            | ColumnOp::StartOfWeek
            | ColumnOp::EndOfWeek => "type date",
            // Start and End of day and hour keep the date/time; BIcraft writes datetime whatever
            // the column's type (a time column's result is not typed `time`)
            ColumnOp::StartOfDay | ColumnOp::EndOfDay | ColumnOp::StartOfHour | ColumnOp::EndOfHour => {
                "type datetime"
            }
            ColumnOp::TimeOnly => "type time",
        }
    }

    /// (step name in place, new column's default name).
    fn names(&self) -> (&'static str, &'static str) {
        match self {
            ColumnOp::Lower => ("Lowercased Text", "lowercase"),
            ColumnOp::Upper => ("Uppercased Text", "UPPERCASE"),
            ColumnOp::Proper => ("Capitalized Each Word", "Capitalize Each Word"),
            ColumnOp::Trim => ("Trimmed Text", "Trim"),
            ColumnOp::Clean => ("Cleaned Text", "Clean"),
            ColumnOp::Prefix(_) => ("Added Prefix", "Prefix"),
            ColumnOp::Suffix(_) => ("Added Suffix", "Suffix"),
            ColumnOp::Length => ("Calculated Text Length", "Length"),
            ColumnOp::FirstChars(_) => ("Extracted First Characters", "First Characters"),
            ColumnOp::LastChars(_) => ("Extracted Last Characters", "Last Characters"),
            ColumnOp::TextRange { .. } => ("Extracted Text Range", "Text Range"),
            ColumnOp::BeforeDelimiter(_) => ("Extracted Text Before Delimiter", "Text Before Delimiter"),
            ColumnOp::AfterDelimiter(_) => ("Extracted Text After Delimiter", "Text After Delimiter"),
            ColumnOp::Year => ("Extracted Year", "Year"),
            ColumnOp::Month => ("Extracted Month", "Month"),
            ColumnOp::Day => ("Extracted Day", "Day"),
            ColumnOp::Quarter => ("Extracted Quarter", "Quarter"),
            ColumnOp::MonthName => ("Extracted Month Name", "Month Name"),
            ColumnOp::DayName => ("Extracted Day Name", "Day Name"),
            ColumnOp::DayOfWeek => ("Extracted Day of Week", "Day of Week"),
            ColumnOp::WeekOfYear => ("Extracted Week of Year", "Week of Year"),
            ColumnOp::DayOfYear => ("Extracted Day of Year", "Day of Year"),
            ColumnOp::StartOfMonth => ("Calculated Start of Month", "Start of Month"),
            ColumnOp::EndOfMonth => ("Calculated End of Month", "End of Month"),
            ColumnOp::StartOfYear => ("Calculated Start of Year", "Start of Year"),
            ColumnOp::EndOfYear => ("Calculated End of Year", "End of Year"),
            ColumnOp::DateOnly => ("Extracted Date", "Date"),
            ColumnOp::Hour => ("Extracted Hour", "Hour"),
            ColumnOp::Minute => ("Extracted Minute", "Minute"),
            ColumnOp::Second => ("Extracted Second", "Second"),
            ColumnOp::Round(_) => ("Rounded Off", "Round"),
            ColumnOp::RoundUp => ("Rounded Up", "Round Up"),
            ColumnOp::RoundDown => ("Rounded Down", "Round Down"),
            ColumnOp::Abs => ("Calculated Absolute Value", "Absolute Value"),
            ColumnOp::Add(_) => ("Added to Column", "Addition"),
            ColumnOp::Subtract(_) => ("Subtracted from Column", "Subtraction"),
            ColumnOp::Multiply(_) => ("Multiplied Column", "Multiplication"),
            ColumnOp::Divide(_) => ("Divided Column", "Division"),
            ColumnOp::BetweenDelimiters(..) => {
                ("Extracted Text Between Delimiters", "Text Between Delimiters")
            }
            ColumnOp::IntegerDivide(_) => ("Integer-Divided Column", "Integer-Division"),
            ColumnOp::Modulo(_) => ("Calculated Modulo", "Modulo"),
            ColumnOp::Percentage(_) => ("Calculated Percentage", "Percentage"),
            ColumnOp::PercentOf(_) => ("Calculated Percent Of", "Percent Of"),
            ColumnOp::Power(_) => ("Calculated Power", "Power"),
            ColumnOp::SquareRoot => ("Calculated Square Root", "Square Root"),
            ColumnOp::Exponent => ("Calculated Exponent", "Exponent"),
            ColumnOp::Log10 => ("Calculated Base-10 Logarithm", "Base-10 Logarithm"),
            ColumnOp::Ln => ("Calculated Natural Logarithm", "Natural Logarithm"),
            ColumnOp::Factorial => ("Calculated Factorial", "Factorial"),
            ColumnOp::Sin => ("Calculated Sine", "Sine"),
            ColumnOp::Cos => ("Calculated Cosine", "Cosine"),
            ColumnOp::Tan => ("Calculated Tangent", "Tangent"),
            ColumnOp::Asin => ("Calculated Arcsine", "Arcsine"),
            ColumnOp::Acos => ("Calculated Arccosine", "Arccosine"),
            ColumnOp::Atan => ("Calculated Arctangent", "Arctangent"),
            ColumnOp::IsEven => ("Calculated Is Even", "Is Even"),
            ColumnOp::IsOdd => ("Calculated Is Odd", "Is Odd"),
            ColumnOp::Sign => ("Calculated Sign", "Sign"),
            ColumnOp::Age => ("Calculated Age", "Age"),
            ColumnOp::StartOfQuarter => ("Calculated Start of Quarter", "Start of Quarter"),
            ColumnOp::EndOfQuarter => ("Calculated End of Quarter", "End of Quarter"),
            ColumnOp::StartOfWeek => ("Calculated Start of Week", "Start of Week"),
            ColumnOp::EndOfWeek => ("Calculated End of Week", "End of Week"),
            ColumnOp::StartOfDay => ("Calculated Start of Day", "Start of Day"),
            ColumnOp::EndOfDay => ("Calculated End of Day", "End of Day"),
            ColumnOp::DaysInMonth => ("Calculated Days in Month", "Days in Month"),
            ColumnOp::WeekOfMonth => ("Extracted Week of Month", "Week of Month"),
            ColumnOp::TimeOnly => ("Extracted Time", "Time"),
            ColumnOp::StartOfHour => ("Calculated Start of Hour", "Start of Hour"),
            ColumnOp::EndOfHour => ("Calculated End of Hour", "End of Hour"),
            ColumnOp::DurationDays => ("Extracted Days", "Days"),
            ColumnOp::DurationHours => ("Extracted Hours", "Hours"),
            ColumnOp::DurationMinutes => ("Extracted Minutes", "Minutes"),
            ColumnOp::DurationSeconds => ("Extracted Seconds", "Seconds"),
            ColumnOp::TotalYears => ("Calculated Total Years", "Total Years"),
            ColumnOp::TotalDays => ("Calculated Total Days", "Total Days"),
            ColumnOp::TotalHours => ("Calculated Total Hours", "Total Hours"),
            ColumnOp::TotalMinutes => ("Calculated Total Minutes", "Total Minutes"),
            ColumnOp::TotalSeconds => ("Calculated Total Seconds", "Total Seconds"),
        }
    }

    /// The new column's default name.
    pub fn column_name(&self) -> &'static str {
        self.names().1
    }
}

/// One rule of a conditional column: when `column op value`, the output.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub column: String,
    pub op: FilterOp,
    pub value: Literal,
    pub output: Literal,
}

/// The transforms of this module.
#[derive(Debug, Clone, PartialEq)]
pub enum More {
    RemoveTopRows(u64),
    RemoveBottomRows(u64),
    KeepTopRows(u64),
    KeepBottomRows(u64),
    /// Keep `count` rows from row `first` (1-based, as the dialog counts).
    KeepRange {
        first: u64,
        count: u64,
    },
    /// From row `first` (1-based), remove `remove` rows then keep `keep`, again and again.
    RemoveAlternateRows {
        first: u64,
        remove: u64,
        keep: u64,
    },
    /// Rows whose every value is null or empty text.
    RemoveBlankRows,
    /// Rows with an error in these columns (none: in any column).
    RemoveErrors(Vec<String>),
    KeepErrors(Vec<String>),
    ReplaceErrors {
        columns: Vec<String>,
        value: Literal,
    },
    FillDown(Vec<String>),
    FillUp(Vec<String>),
    ReverseRows,
    Transpose,
    /// Use headers as first row.
    DemoteHeaders,
    /// Move: the columns in this order (the others keep their places after them).
    ReorderColumns(Vec<String>),
    DuplicateColumn {
        column: String,
        name: String,
    },
    AddIndex {
        name: String,
        start: i64,
        step: i64,
    },
    /// Merge columns into `name` with `delimiter`, in place or as a new column (`add`).
    MergeColumns {
        columns: Vec<String>,
        delimiter: String,
        name: String,
        add: bool,
    },
    ConditionalColumn {
        name: String,
        rules: Vec<Rule>,
        otherwise: Literal,
    },
    /// A per-value operation over `columns` in place, or into a new column `name` read from the
    /// first column (`add`).
    ColumnOp {
        op: ColumnOp,
        columns: Vec<String>,
        add: bool,
        name: Option<String>,
    },
}

fn columns_ok(what: &str, c: &[String], may_be_empty: bool) -> Result<(), String> {
    if c.is_empty() && !may_be_empty {
        return Err(format!("{what}: name at least one column"));
    }
    if c.len() > MAX_COLUMNS {
        return Err(format!("{what}: at most {MAX_COLUMNS} columns"));
    }
    if c.iter().any(|x| x.is_empty()) {
        return Err(format!("{what}: a column name cannot be empty"));
    }
    Ok(())
}

fn texts(items: &[String]) -> String {
    format!(
        "{{{}}}",
        items.iter().map(|s| text(s)).collect::<Vec<_>>().join(", ")
    )
}

fn rows_ok(what: &str, n: u64) -> Result<u64, String> {
    if (1..=MAX_ROWS).contains(&n) {
        Ok(n)
    } else {
        Err(format!("{what}: 1 to {MAX_ROWS} rows"))
    }
}

impl More {
    pub fn kind(&self) -> &'static str {
        match self {
            More::RemoveTopRows(_) => "remove_top_rows",
            More::RemoveBottomRows(_) => "remove_bottom_rows",
            More::KeepTopRows(_) => "keep_top_rows",
            More::KeepBottomRows(_) => "keep_bottom_rows",
            More::KeepRange { .. } => "keep_range",
            More::RemoveAlternateRows { .. } => "remove_alternate_rows",
            More::RemoveBlankRows => "remove_blank_rows",
            More::RemoveErrors(_) => "remove_errors",
            More::KeepErrors(_) => "keep_errors",
            More::ReplaceErrors { .. } => "replace_errors",
            More::FillDown(_) => "fill_down",
            More::FillUp(_) => "fill_up",
            More::ReverseRows => "reverse_rows",
            More::Transpose => "transpose",
            More::DemoteHeaders => "demote_headers",
            More::ReorderColumns(_) => "reorder_columns",
            More::DuplicateColumn { .. } => "duplicate_column",
            More::AddIndex { .. } => "add_index",
            More::MergeColumns { .. } => "merge_columns",
            More::ConditionalColumn { .. } => "conditional_column",
            More::ColumnOp { .. } => "column_op",
        }
    }

    pub fn step_name(&self) -> String {
        match self {
            More::RemoveTopRows(_) => "Removed Top Rows".into(),
            More::RemoveBottomRows(_) => "Removed Bottom Rows".into(),
            More::KeepTopRows(_) => "Kept First Rows".into(),
            More::KeepBottomRows(_) => "Kept Last Rows".into(),
            More::KeepRange { .. } => "Kept Range of Rows".into(),
            More::RemoveAlternateRows { .. } => "Removed Alternate Rows".into(),
            More::RemoveBlankRows => "Removed Blank Rows".into(),
            More::RemoveErrors(_) => "Removed Errors".into(),
            More::KeepErrors(_) => "Kept Errors".into(),
            More::ReplaceErrors { .. } => "Replaced Errors".into(),
            More::FillDown(_) => "Filled Down".into(),
            More::FillUp(_) => "Filled Up".into(),
            More::ReverseRows => "Reversed Rows".into(),
            More::Transpose => "Transposed Table".into(),
            More::DemoteHeaders => "Demoted Headers".into(),
            More::ReorderColumns(_) => "Reordered Columns".into(),
            More::DuplicateColumn { .. } => "Duplicated Column".into(),
            More::AddIndex { .. } => "Added Index".into(),
            More::MergeColumns { add: false, .. } => "Merged Columns".into(),
            More::MergeColumns { add: true, .. } => "Inserted Merged Column".into(),
            More::ConditionalColumn { .. } => "Added Conditional Column".into(),
            More::ColumnOp { op, add: false, .. } => op.names().0.into(),
            More::ColumnOp { op, add: true, .. } => {
                let n = op.names().0;
                format!("Inserted {}", n.strip_prefix("Extracted ").unwrap_or(n))
            }
        }
    }

    /// The step's M, reading step `prev` (its name, unquoted).
    pub fn m(&self, prev: &str) -> Result<String, String> {
        let p = ident(prev);
        Ok(match self {
            More::RemoveTopRows(n) => format!("Table.Skip({p},{})", rows_ok("remove_top_rows", *n)?),
            More::RemoveBottomRows(n) => {
                format!("Table.RemoveLastN({p},{})", rows_ok("remove_bottom_rows", *n)?)
            }
            More::KeepTopRows(n) => format!("Table.FirstN({p},{})", rows_ok("keep_top_rows", *n)?),
            More::KeepBottomRows(n) => format!("Table.LastN({p},{})", rows_ok("keep_bottom_rows", *n)?),
            More::KeepRange { first, count } => {
                let f = rows_ok("keep_range: `first`", *first)?;
                let c = rows_ok("keep_range: `count`", *count)?;
                format!("Table.Range({p},{},{c})", f.saturating_sub(1))
            }
            More::RemoveAlternateRows { first, remove, keep } => {
                let f = rows_ok("remove_alternate_rows: `first`", *first)?;
                let r = rows_ok("remove_alternate_rows: `remove`", *remove)?;
                if *keep > MAX_ROWS {
                    return Err(format!("remove_alternate_rows: `keep` 0 to {MAX_ROWS}"));
                }
                format!("Table.AlternateRows({p},{},{r},{keep})", f.saturating_sub(1))
            }
            More::RemoveBlankRows => format!(
                "Table.SelectRows({p}, each not List.IsEmpty(List.RemoveMatchingItems(Record.FieldValues(_), {{\"\", null}})))"
            ),
            More::RemoveErrors(c) | More::KeepErrors(c) => {
                columns_ok(self.kind(), c, true)?;
                let f = if matches!(self, More::RemoveErrors(_)) {
                    "Table.RemoveRowsWithErrors"
                } else {
                    "Table.SelectRowsWithErrors"
                };
                if c.is_empty() {
                    format!("{f}({p})")
                } else {
                    format!("{f}({p}, {})", texts(c))
                }
            }
            More::ReplaceErrors { columns, value } => {
                columns_ok("replace_errors", columns, false)?;
                let items: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{{{}, {}}}", text(c), value.m()))
                    .collect();
                format!("Table.ReplaceErrorValues({p}, {{{}}})", items.join(", "))
            }
            More::FillDown(c) | More::FillUp(c) => {
                columns_ok(self.kind(), c, false)?;
                let f = if matches!(self, More::FillDown(_)) {
                    "Table.FillDown"
                } else {
                    "Table.FillUp"
                };
                format!("{f}({p},{})", texts(c))
            }
            More::ReverseRows => format!("Table.ReverseRows({p})"),
            More::Transpose => format!("Table.Transpose({p})"),
            More::DemoteHeaders => format!("Table.DemoteHeaders({p})"),
            More::ReorderColumns(c) => {
                columns_ok("reorder_columns", c, false)?;
                format!("Table.ReorderColumns({p},{})", texts(c))
            }
            More::DuplicateColumn { column, name } => {
                if column.is_empty() || name.trim().is_empty() {
                    return Err("duplicate_column: name the column and the copy".into());
                }
                format!("Table.DuplicateColumn({p}, {}, {})", text(column), text(name))
            }
            More::AddIndex { name, start, step } => {
                if name.trim().is_empty() {
                    return Err("add_index: name the new column".into());
                }
                format!(
                    "Table.AddIndexColumn({p}, {}, {start}, {step}, Int64.Type)",
                    text(name)
                )
            }
            More::MergeColumns {
                columns,
                delimiter,
                name,
                add,
            } => {
                columns_ok("merge_columns", columns, false)?;
                if columns.len() < 2 {
                    return Err("merge_columns: name at least two columns".into());
                }
                if name.trim().is_empty() {
                    return Err("merge_columns: name the merged column".into());
                }
                if *add {
                    let parts: Vec<String> = columns
                        .iter()
                        .map(|c| format!("Text.From({})", field(c)))
                        .collect();
                    format!(
                        "Table.AddColumn({p}, {}, each Text.Combine({{{}}}, {}), type text)",
                        text(name),
                        parts.join(", "),
                        text(delimiter)
                    )
                } else {
                    // the columns as text first: the combiner joins texts
                    let typed: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{{{}, type text}}", text(c)))
                        .collect();
                    format!(
                        "Table.CombineColumns(Table.TransformColumnTypes({p}, {{{}}}),{},Combiner.CombineTextByDelimiter({}, QuoteStyle.None),{})",
                        typed.join(", "),
                        texts(columns),
                        text(delimiter),
                        text(name)
                    )
                }
            }
            More::ConditionalColumn {
                name,
                rules,
                otherwise,
            } => {
                if name.trim().is_empty() {
                    return Err("conditional_column: name the new column".into());
                }
                if rules.is_empty() || rules.len() > MAX_RULES {
                    return Err(format!("conditional_column: 1 to {MAX_RULES} rules"));
                }
                let mut body = String::new();
                for (i, r) in rules.iter().enumerate() {
                    if r.column.is_empty() {
                        return Err(format!("conditional_column: rules[{i}] names no column"));
                    }
                    if i > 0 {
                        body.push_str(" else ");
                    }
                    body.push_str(&format!(
                        "if {} then {}",
                        r.op.condition(&r.column, &r.value),
                        r.output.m()
                    ));
                }
                format!(
                    "Table.AddColumn({p}, {}, each {body} else {})",
                    text(name),
                    otherwise.m()
                )
            }
            More::ColumnOp {
                op,
                columns,
                add,
                name,
            } => {
                columns_ok("column_op", columns, false)?;
                if *add {
                    let [c] = columns.as_slice() else {
                        return Err("column_op: a new column reads one column".into());
                    };
                    let n = name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(op.column_name());
                    format!(
                        "Table.AddColumn({p}, {}, each {}, {})",
                        text(n),
                        op.apply(&field(c)),
                        op.ty()
                    )
                } else {
                    let f = op.function();
                    let items: Vec<String> = columns
                        .iter()
                        .map(|c| format!("{{{}, {f}, {}}}", text(c), op.ty()))
                        .collect();
                    format!("Table.TransformColumns({p},{{{}}})", items.join(", "))
                }
            }
        })
    }
}
