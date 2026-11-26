//! DAX Currency (`Fixed Decimal Number`, TMSL `decimal`) arithmetic for the data preview.
//!
//! A Currency value is a 64-bit count of 1/10 000 units. Operations that return Currency round
//! their exact result half away from zero to 4 decimals; the rules (which operations return
//! Currency) are in docs/formats/dax-functions.md, "Currency arithmetic".

/// Units per 1.
pub(super) const SCALE: f64 = 10_000.0;

/// The number a Currency value stands for.
pub(super) fn to_f64(n: i64) -> f64 {
    n as f64 / SCALE
}

/// A result already expressed in 1/10 000 units (`n as f64 * r`), rounded half away from zero.
/// `None` when it does not fit (the engine reports an overflow).
pub(super) fn from_scaled(y: f64) -> Option<i64> {
    if !y.is_finite() {
        return None;
    }
    let r = y.round();
    // i64::MAX is not exactly representable; stay strictly inside.
    if r.abs() >= 9_223_372_036_854_775_000.0 {
        return None;
    }
    Some(r as i64)
}

/// A double converted to Currency: its exact binary value rounded half away from zero to 4
/// decimals. `CURRENCY(0.00015)` is 0.0001 because the double nearest 0.00015 lies just below
/// the tie, while `CURRENCY(0.0003) / 2` is 0.0002 (exact tie on the scaled integer).
pub(super) fn from_f64(x: f64) -> Option<i64> {
    if !x.is_finite() {
        return None;
    }
    if x == 0.0 {
        return Some(0);
    }
    let bits = x.to_bits();
    let negative = bits >> 63 == 1;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    // x = m · 2^e exactly
    let (m, e) = if exp == 0 {
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), exp - 1075)
    };
    let m = u128::from(m).checked_mul(10_000)?;
    let magnitude: u128 = if e >= 0 {
        if e >= 64 {
            return None;
        }
        m.checked_mul(1u128 << e)?
    } else {
        let k = e.unsigned_abs();
        if k >= 100 {
            // m < 2^67: the value is below half a unit
            0
        } else {
            let q = m >> k;
            let rem = m & ((1u128 << k) - 1);
            if rem >= 1u128 << (k - 1) { q + 1 } else { q }
        }
    };
    let v = i64::try_from(magnitude).ok()?;
    Some(if negative { -v } else { v })
}

/// `a / b` on integers, rounded half away from zero (`None` when `b` is 0 or it overflows).
pub(super) fn div_round(a: i128, b: i128) -> Option<i64> {
    let (q, r) = (a.checked_div(b)?, a.checked_rem(b)?);
    let away = r.checked_abs()?.checked_mul(2)? >= b.checked_abs()?;
    let q = if away {
        if (a < 0) != (b < 0) { q - 1 } else { q + 1 }
    } else {
        q
    };
    i64::try_from(q).ok()
}

/// Rounds a number to 4 decimals the way a Currency result is (for code paths that keep doubles).
pub(super) fn round4(x: f64) -> f64 {
    from_f64(x).map_or(x, to_f64)
}

/// The static type of a number, as far as Currency rules care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Whole number (`Int64`, a count, `TRUE`/`FALSE`, `BLANK` next to a number).
    Int,
    /// Double (`Decimal Number`).
    Real,
    Currency,
}

impl Kind {
    /// The kind of a plain number when only its value is known: integral values count as
    /// whole numbers.
    pub(super) fn of_value(x: f64) -> Kind {
        if x.fract() == 0.0 && x.abs() < 9.0e15 {
            Kind::Int
        } else {
            Kind::Real
        }
    }
}

/// The result of `a op b` when it is Currency, in 1/10 000 units; `None` when the result is
/// not Currency (the caller computes it with doubles). `Some(None)`: Currency overflow.
///
/// Engine rules (Power BI Desktop 2.158): `+`/`-` give Currency for Currency with Currency or
/// with a whole number, a double for Currency with a double; `*` gives Currency for Currency with
/// a whole number or a double, a double for Currency × Currency; `/` gives Currency for Currency
/// divided by a whole number or a double, a double otherwise (`x / Currency`); `^` gives a double.
/// The caller handles blanks and division by zero first.
pub(super) fn binary(op: char, a: f64, ka: Kind, b: f64, kb: Kind) -> Option<Option<i64>> {
    use Kind::{Currency, Int, Real};
    let scaled = |x: f64, k: Kind| match k {
        Currency | Int => from_f64(x),
        Real => None,
    };
    match (op, ka, kb) {
        ('+' | '-', Currency, Currency | Int) | ('+' | '-', Int, Currency) => {
            let (Some(x), Some(y)) = (scaled(a, ka), scaled(b, kb)) else {
                return Some(None);
            };
            Some(if op == '+' {
                x.checked_add(y)
            } else {
                x.checked_sub(y)
            })
        }
        ('*', Currency, Int | Real) => Some(from_f64(a).and_then(|x| from_scaled(x as f64 * b))),
        ('*', Int | Real, Currency) => Some(from_f64(b).and_then(|y| from_scaled(a * y as f64))),
        ('/', Currency, Int | Real) => Some(from_f64(a).and_then(|x| from_scaled(x as f64 / b))),
        _ => None,
    }
}

/// The kind of `a op b` (for the fast path, which types its expression tree statically).
pub(super) fn binary_kind(op: char, ka: Kind, kb: Kind) -> Kind {
    use Kind::{Currency, Int, Real};
    match (op, ka, kb) {
        ('+' | '-', Currency, Currency | Int) | ('+' | '-', Int, Currency) => Currency,
        ('*', Currency, Int | Real) | ('*', Int | Real, Currency) | ('/', Currency, Int | Real) => Currency,
        ('+' | '-' | '*', Int, Int) => Int,
        _ => Real,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_rounds_the_exact_double_half_away_from_zero() {
        // What the engine gives (Power BI Desktop 2.158, CURRENCY(x)).
        assert_eq!(from_f64(0.00005), Some(1));
        assert_eq!(from_f64(0.00015), Some(1));
        assert_eq!(from_f64(0.00025), Some(3));
        assert_eq!(from_f64(-0.00005), Some(-1));
        assert_eq!(from_f64(1.00005), Some(10_001));
        assert_eq!(from_f64(1.23456), Some(12_346));
        // A true tie (0.03125 is exact in binary) goes away from zero.
        assert_eq!(from_f64(0.031_25), Some(313));
        assert_eq!(from_f64(-0.031_25), Some(-313));
        assert_eq!(from_f64(0.0), Some(0));
        assert_eq!(from_f64(5e-324), Some(0));
        assert_eq!(from_f64(1e300), None);
        assert_eq!(from_f64(f64::NAN), None);
        assert_eq!(from_f64(922_337_203_685_477.0), Some(9_223_372_036_854_770_000));
        assert_eq!(from_f64(1e15), None);
    }

    #[test]
    fn scaled_results_round_half_away_from_zero() {
        assert_eq!(from_scaled(1.5), Some(2));
        assert_eq!(from_scaled(0.5), Some(1));
        assert_eq!(from_scaled(-2.5), Some(-3));
        assert_eq!(from_scaled(33_333.33), Some(33_333));
        assert_eq!(from_scaled(f64::INFINITY), None);
        assert_eq!(from_scaled(1e19), None);
    }

    #[test]
    fn integer_division_rounds_half_away_from_zero() {
        assert_eq!(div_round(9_824_500, 16), Some(614_031)); // 61.403125 → 61.4031
        assert_eq!(div_round(3, 2), Some(2));
        assert_eq!(div_round(-3, 2), Some(-2));
        assert_eq!(div_round(3, -2), Some(-2));
        assert_eq!(div_round(-25, 10), Some(-3));
        assert_eq!(div_round(7, 0), None);
        assert_eq!(div_round(i128::MAX, 1), None);
        assert_eq!(round4(61.403_125), 61.4031);
    }

    #[test]
    fn binary_follows_the_engine() {
        use Kind::{Currency as C, Int as I, Real as R};
        // Values and types the engine returned (see docs/formats/dax-functions.md).
        assert_eq!(binary('/', 10.0, C, 3.0, I), Some(Some(33_333)));
        assert_eq!(binary('/', 10.0, C, 3.3, R), Some(Some(30_303)));
        assert_eq!(
            binary('/', 10.0, C, 3.0, C),
            None,
            "Currency / Currency is a double"
        );
        assert_eq!(binary('/', 10.0, I, 3.0, C), None, "x / Currency is a double");
        assert_eq!(binary('*', 1.2345, C, 3.0, I), Some(Some(37_035)));
        assert_eq!(binary('*', 1.2345, C, 1.1111, R), Some(Some(13_717)));
        assert_eq!(binary('*', 1.1111, R, 1.2345, C), Some(Some(13_717)));
        assert_eq!(binary('*', 1.2345, C, 1.1111, C), None);
        assert_eq!(binary('+', 1.2345, C, 1.0, I), Some(Some(22_345)));
        assert_eq!(binary('+', 1.0, C, 2.0, C), Some(Some(30_000)));
        assert_eq!(
            binary('+', 1.2345, C, 0.000_01, R),
            None,
            "Currency + double is a double"
        );
        assert_eq!(binary('-', 1.0, C, 0.000_04, R), None);
        assert_eq!(binary('^', 1.5, C, 2.0, I), None);
        // Ties on the scaled value go away from zero.
        assert_eq!(binary('/', 0.0001, C, 2.0, I), Some(Some(1)));
        assert_eq!(binary('/', 0.0003, C, 2.0, I), Some(Some(2)));
        assert_eq!(binary('/', -0.0003, C, 2.0, I), Some(Some(-2)));
        assert_eq!(binary('*', 0.0003, C, 0.5, R), Some(Some(2)));
        assert_eq!(binary('/', -2.5, C, 10_000.0, I), Some(Some(-3)));
        // Overflow.
        assert_eq!(binary('*', 900_000_000_000_000.0, C, 100.0, R), Some(None));
        assert_eq!(binary_kind('/', C, I), C);
        assert_eq!(binary_kind('/', I, I), R);
        assert_eq!(binary_kind('+', I, I), I);
        assert_eq!(binary_kind('+', C, R), R);
        assert_eq!(binary_kind('*', C, C), R);
        assert_eq!(Kind::of_value(3.0), I);
        assert_eq!(Kind::of_value(0.5), R);
    }
}
