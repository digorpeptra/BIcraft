//! `Number.*`, the typed `*.From` constructors and `Logical.*`.

use super::{Builtin, arg, b, conv_err, culture_arg, number, opt_num, opt_number, opt_text};
use crate::m::convert::{self, apply_facet};
use crate::m::decimal::Dec;
use crate::m::eval::Interp;
use crate::m::value::{Facet, Kind, MType, Num, Res, Value, expr_err, unsupported};

pub(crate) static FUNCS: &[Builtin] = &[
    b("Number.From", 1, 2, from),
    b("Number.FromText", 1, 2, from_text),
    b("Number.ToText", 1, 3, to_text),
    b("Number.Round", 1, 3, round),
    b("Number.RoundUp", 1, 2, round_up),
    b("Number.RoundDown", 1, 2, round_down),
    b("Number.RoundTowardZero", 1, 2, round_toward_zero),
    b("Number.RoundAwayFromZero", 1, 2, round_away),
    b("Number.Abs", 1, 1, abs),
    b("Number.Sign", 1, 1, sign),
    b("Number.Sqrt", 1, 1, sqrt),
    b("Number.Power", 2, 2, power),
    b("Number.Exp", 1, 1, exp),
    b("Number.Ln", 1, 1, ln),
    b("Number.Log", 1, 2, log),
    b("Number.Log10", 1, 1, log10),
    b("Number.Mod", 2, 3, modulo),
    b("Number.IntegerDivide", 2, 3, integer_divide),
    b("Number.Factorial", 1, 1, factorial),
    b("Number.Sin", 1, 1, sin),
    b("Number.Cos", 1, 1, cos),
    b("Number.Tan", 1, 1, tan),
    b("Number.Asin", 1, 1, asin),
    b("Number.Acos", 1, 1, acos),
    b("Number.Atan", 1, 1, atan),
    b("Number.IsEven", 1, 1, is_even),
    b("Number.IsOdd", 1, 1, is_odd),
    b("Number.IsNaN", 1, 1, is_nan),
    b("Number.Random", 0, 0, random),
    b("Number.RandomBetween", 2, 2, random),
    b("Int64.From", 1, 3, int64_from),
    b("Int32.From", 1, 3, int32_from),
    b("Int16.From", 1, 3, int16_from),
    b("Int8.From", 1, 3, int8_from),
    b("Byte.From", 1, 3, byte_from),
    b("Double.From", 1, 2, double_from),
    b("Single.From", 1, 2, single_from),
    b("Decimal.From", 1, 2, decimal_from),
    b("Currency.From", 1, 3, currency_from),
    b("Percentage.From", 1, 2, from),
    b("Logical.From", 1, 1, logical_from),
    b("Logical.FromText", 1, 1, logical_from_text),
    b("Logical.ToText", 1, 1, logical_to_text),
    b("Value.Equals", 2, 3, value_equals),
    b("Value.Compare", 2, 3, value_compare),
];

fn map(a: &[Value], f: impl FnOnce(f64) -> f64) -> Res<Value> {
    Ok(match opt_number(arg(a, 0))? {
        None => Value::Null,
        Some(n) => Value::num(f(n)),
    })
}

fn from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    convert::to_number(arg(a, 0), &c)
}

fn from_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    match opt_text(arg(a, 0))? {
        None => Ok(Value::Null),
        Some(_) => convert::to_number(arg(a, 0), &c),
    }
}

fn to_text(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 2))?;
    let Some(n) = opt_num(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    match opt_text(arg(a, 1))? {
        None => Ok(Value::text(convert::number_to_text(n, &c))),
        Some(f) => match convert::format_number(n, f, &c) {
            Some(s) => Ok(Value::text(s)),
            None => expr_err(format!("The format '{f}' is not supported.")),
        },
    }
}

/// `Number.Round` on a decimal, as Desktop does it: the value is scaled by `10^digits`; a
/// tie (fractional part exactly .5) moves half a unit in the direction `mode` gives, keeping
/// the scale (`2.675` → `2.680`, `-2.5` → `-2.0`), except `TowardZero`, which truncates;
/// any other value goes to the nearest integer. The result is divided back by `10^digits`.
/// `mode`: 0 Up, 1 Down, 2 AwayFromZero, 3 TowardZero, else ToEven.
fn round_dec(x: Dec, digits: i64, mode: i64) -> Option<Dec> {
    if digits > 28 {
        return Some(x);
    }
    if digits < -28 {
        return Some(Dec::ZERO);
    }
    let p = Dec::pow10(i32::try_from(digits).ok()?)?;
    let scaled = match x.mul(p) {
        Ok(s) => s,
        Err(_) if digits >= 0 && i64::from(x.scale()) <= digits => return Some(x),
        Err(_) => return None,
    };
    let r = if scaled.is_half() {
        let half = Dec::parse("0.5")?;
        let neg = scaled.is_negative();
        let toward_zero = |v: Dec| if neg { v.add(half) } else { v.sub(half) };
        let away = |v: Dec| if neg { v.sub(half) } else { v.add(half) };
        match mode {
            0 => scaled.add(half).ok()?,
            1 => scaled.sub(half).ok()?,
            2 => away(scaled).ok()?,
            3 => scaled.trunc(),
            _ => {
                if scaled.trunc().to_i128().is_some_and(|t| t % 2 == 0) {
                    toward_zero(scaled).ok()?
                } else {
                    away(scaled).ok()?
                }
            }
        }
    } else {
        // `scaled ± 0.5`, truncated: the sum rounds to 28-29 digits first, as in Desktop
        // (`Number.Round(12345678901234567890123456789)` → `…790`).
        let half = Dec::parse("0.5")?;
        let moved = if scaled.is_negative() {
            scaled.sub(half)
        } else {
            scaled.add(half)
        };
        match moved {
            Ok(m) => m.trunc(),
            Err(_) => scaled.round_nearest(),
        }
    };
    r.div(p).ok()
}

/// `Number.Round(n, digits, mode)`: decimals, and doubles within the decimal range (converted
/// to 15 significant digits), round as decimals ([`round_dec`]) and give a decimal; NaN,
/// infinities and larger doubles are returned as they are.
pub(crate) fn round_num(n: Num, digits: i64, mode: i64) -> Num {
    match n.dec().and_then(|d| round_dec(d, digits, mode)) {
        Some(d) => Num::D(d),
        None => n,
    }
}

fn digits(v: &Value) -> Res<i64> {
    Ok(match opt_number(v)? {
        None => 0,
        Some(d) if d.fract() == 0.0 && d.abs() < 1e9 => d as i64,
        Some(d) => return expr_err(format!("The number of digits {d} is out of range.")),
    })
}

fn round(_: &mut Interp, a: &[Value]) -> Res<Value> {
    // Desktop returns the value unchanged for fractional digits.
    if let Some(f) = opt_number(arg(a, 1))?
        && f.is_finite()
        && f.fract() != 0.0
    {
        return Ok(arg(a, 0).clone());
    }
    let d = digits(arg(a, 1))?;
    let mode = opt_number(arg(a, 2))?.unwrap_or(4.0) as i64;
    Ok(match opt_num(arg(a, 0))? {
        None => Value::Null,
        Some(n) => Value::Number(round_num(n, d, mode)),
    })
}

/// `Number.RoundUp` and the like: the value scaled by `10^digits`, rounded to an integer by
/// `f`, divided back; decimal arithmetic when the value fits a decimal.
fn scaled(a: &[Value], f: fn(Dec) -> Dec, g: fn(f64) -> f64) -> Res<Value> {
    let d = digits(arg(a, 1))?;
    let Some(n) = opt_num(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    if d > 28 {
        return Ok(Value::Number(n));
    }
    let exact = n.dec().and_then(|x| {
        // Already at that precision: nothing to round.
        if d >= 0 && i64::from(x.scale()) <= d {
            return Some(x);
        }
        let p = Dec::pow10(i32::try_from(d).ok()?)?;
        f(x.mul(p).ok()?).div(p).ok()
    });
    Ok(match exact {
        Some(r) => Value::dec(r),
        None => {
            let p = 10f64.powi(d.clamp(-400, 400) as i32);
            Value::num(g(n.f() * p) / p)
        }
    })
}

fn round_up(_: &mut Interp, a: &[Value]) -> Res<Value> {
    scaled(a, Dec::ceil, f64::ceil)
}

fn round_down(_: &mut Interp, a: &[Value]) -> Res<Value> {
    scaled(a, Dec::floor, f64::floor)
}

fn round_toward_zero(_: &mut Interp, a: &[Value]) -> Res<Value> {
    scaled(a, Dec::trunc, f64::trunc)
}

fn round_away(_: &mut Interp, a: &[Value]) -> Res<Value> {
    scaled(a, Dec::away, |x| x.signum() * x.abs().ceil())
}

fn abs(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(match opt_num(arg(a, 0))? {
        None => Value::Null,
        Some(Num::D(d)) => Value::dec(d.abs()),
        Some(Num::F(x)) => Value::num(x.abs()),
    })
}

fn sign(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, |n| {
        if n > 0.0 {
            1.0
        } else if n < 0.0 {
            -1.0
        } else {
            n
        }
    })
}

fn sqrt(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::sqrt)
}

fn power(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(e) = opt_number(arg(a, 1))? else {
        return Ok(Value::Null);
    };
    map(a, |n| n.powf(e))
}

fn exp(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::exp)
}

fn ln(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::ln)
}

fn log(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let base = opt_number(arg(a, 1))?.unwrap_or(std::f64::consts::E);
    map(a, |n| n.log(base))
}

fn log10(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::log10)
}

fn modulo(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (Some(n), Some(d)) = (opt_number(arg(a, 0))?, opt_number(arg(a, 1))?) else {
        return Ok(Value::Null);
    };
    if d == 0.0 {
        return expr_err("Division by zero.");
    }
    Ok(Value::num(n % d))
}

fn integer_divide(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let (Some(n), Some(d)) = (opt_number(arg(a, 0))?, opt_number(arg(a, 1))?) else {
        return Ok(Value::Null);
    };
    if d == 0.0 {
        return expr_err("Division by zero.");
    }
    Ok(Value::num((n / d).trunc()))
}

fn factorial(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let Some(n) = opt_number(arg(a, 0))? else {
        return Ok(Value::Null);
    };
    if !(n >= 0.0 && n.fract() == 0.0) {
        return expr_err("Number.Factorial takes a whole number 0 or more.");
    }
    // 171! and beyond overflow a double: the loop stops there (Infinity)
    let mut p = 1.0f64;
    let mut k = 2.0f64;
    while k <= n && p.is_finite() {
        p *= k;
        k += 1.0;
    }
    Ok(Value::num(p))
}

fn sin(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::sin)
}

fn cos(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::cos)
}

fn tan(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::tan)
}

fn asin(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::asin)
}

fn acos(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::acos)
}

fn atan(_: &mut Interp, a: &[Value]) -> Res<Value> {
    map(a, f64::atan)
}

fn is_even(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = number(arg(a, 0))?;
    Ok(Value::Logical(n % 2.0 == 0.0))
}

fn is_odd(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let n = number(arg(a, 0))?;
    Ok(Value::Logical(n.abs() % 2.0 == 1.0))
}

fn is_nan(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(number(arg(a, 0))?.is_nan()))
}

fn random(_: &mut Interp, _: &[Value]) -> Res<Value> {
    unsupported("`Number.Random` (non-deterministic)")
}

fn typed_from(interp: &mut Interp, a: &[Value], facet: Facet) -> Res<Value> {
    let c = culture_arg(interp, arg(a, 1))?;
    let n = match convert::to_number(arg(a, 0), &c)? {
        Value::Number(n) => n,
        _ => return Ok(Value::Null),
    };
    match opt_number(arg(a, 2))? {
        Some(mode)
            if matches!(
                facet,
                Facet::Int64 | Facet::Int32 | Facet::Int16 | Facet::Int8 | Facet::Byte
            ) =>
        {
            let r = round_num(n, 0, mode as i64);
            Ok(Value::Number(apply_facet(r, facet)?))
        }
        Some(mode) if facet == Facet::Currency => {
            Ok(Value::Number(apply_facet(round_num(n, 4, mode as i64), facet)?))
        }
        _ => Ok(Value::Number(apply_facet(n, facet)?)),
    }
}

fn double_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Double)
}

fn decimal_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Decimal)
}

fn int64_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Int64)
}

fn int32_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Int32)
}

fn int16_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Int16)
}

fn int8_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Int8)
}

fn byte_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Byte)
}

fn single_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Single)
}

fn currency_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    typed_from(interp, a, Facet::Currency)
}

fn logical_from(interp: &mut Interp, a: &[Value]) -> Res<Value> {
    let c = interp.culture.clone();
    convert::convert_to(arg(a, 0), &MType::prim(Kind::Logical), &c)
}

fn logical_from_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match opt_text(arg(a, 0))? {
        None => Ok(Value::Null),
        Some(t) => match crate::value::parse_bool(t.trim()) {
            Some(b) => Ok(Value::Logical(b)),
            None => conv_err(arg(a, 0), "Logical"),
        },
    }
}

fn logical_to_text(_: &mut Interp, a: &[Value]) -> Res<Value> {
    match arg(a, 0) {
        Value::Null => Ok(Value::Null),
        Value::Logical(b) => Ok(Value::text(if *b { "true" } else { "false" })),
        other => conv_err(other, "Logical"),
    }
}

fn value_equals(_: &mut Interp, a: &[Value]) -> Res<Value> {
    Ok(Value::Logical(crate::m::eval::values_equal(arg(a, 0), arg(a, 1))))
}

fn value_compare(_: &mut Interp, a: &[Value]) -> Res<Value> {
    let o = crate::m::eval::compare_values(arg(a, 0), arg(a, 1), false);
    Ok(Value::num(match o {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}
