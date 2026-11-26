//! Digit-level number formatting: rounding half away from zero on the decimal digits,
//! digit grouping, and the general format (up to 15 significant digits).

/// Most decimals a fixed format writes.
pub const MAX_DECIMALS: usize = 30;

/// The digits of `|x|` rounded half away from zero to `places` decimals: (integer digits without
/// leading zeros, exactly `places` fraction digits). `None` for NaN and infinities.
///
/// The decision uses the double's first 15 significant digits, as Windows' formatting does
/// (`FORMAT(2.675, "0.00")` is `2.68` although the double is just below 2.675).
pub fn fixed_digits(x: f64, places: usize) -> Option<(String, String)> {
    if !x.is_finite() {
        return None;
    }
    let places = places.min(MAX_DECIMALS);
    let x = x.abs();
    if x == 0.0 {
        return Some((String::new(), "0".repeat(places)));
    }
    // 15 significant digits first.
    let sci = format!("{x:.14e}");
    let (mant, exp) = sci.split_once('e')?;
    let exp: i64 = exp.parse().ok()?;
    let digits: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // point: digits before the decimal point
    let point = exp + 1;
    let keep = point + places as i64;
    if keep < 0 {
        return Some((String::new(), "0".repeat(places)));
    }
    let (mut ds, point) = round_at(digits, point, usize::try_from(keep).unwrap_or(0));
    let total = usize::try_from(point + places as i64).unwrap_or(0);
    if ds.len() < total {
        ds.resize(total, 0);
    }
    let text = |d: &[u8]| d.iter().map(|d| char::from(b'0' + d)).collect::<String>();
    if point <= 0 {
        let lead = usize::try_from(-point).unwrap_or(0).min(places);
        let mut frac = "0".repeat(lead);
        frac.push_str(&text(ds.get(..places.saturating_sub(lead)).unwrap_or(&[])));
        frac.truncate(places);
        while frac.len() < places {
            frac.push('0');
        }
        return Some((String::new(), frac));
    }
    let p = usize::try_from(point).unwrap_or(0);
    let int = text(ds.get(..p).unwrap_or(&[]));
    let mut frac = text(ds.get(p..).unwrap_or(&[]));
    frac.truncate(places);
    while frac.len() < places {
        frac.push('0');
    }
    Some((int.trim_start_matches('0').to_string(), frac))
}

/// Keeps the first `keep` digits, rounding half away from zero; a carry adds a digit in front.
fn round_at(mut digits: Vec<u8>, mut point: i64, keep: usize) -> (Vec<u8>, i64) {
    if keep >= digits.len() {
        return (digits, point);
    }
    let up = digits.get(keep).is_some_and(|d| *d >= 5);
    digits.truncate(keep);
    if up {
        let mut i = keep;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                point += 1;
                break;
            }
            i -= 1;
            if let Some(d) = digits.get_mut(i) {
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    break;
                }
            }
        }
    }
    (digits, point)
}

/// `int` (ASCII digits) with `sep` between groups of three.
pub fn group(int: &str, sep: char) -> String {
    let len = int.chars().count();
    let mut out = String::with_capacity(int.len() + len / 3 * 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// A number with up to 15 significant digits and no grouping (DAX `General Number`, .NET
/// `G15`): scientific (`1.5E+20`, `1E-05`) when the exponent is 15 or more, or below -4.
pub fn general(x: f64, decimal: char) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if x == 0.0 {
        return "0".into();
    }
    let sci = format!("{:.14e}", x.abs());
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i64 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut s = String::new();
    if x < 0.0 {
        s.push('-');
    }
    if !(-4..15).contains(&exp) {
        let (first, rest) = digits.split_at(1);
        s.push_str(first);
        if !rest.is_empty() {
            s.push(decimal);
            s.push_str(rest);
        }
        s.push('E');
        s.push(if exp < 0 { '-' } else { '+' });
        s.push_str(&format!("{:02}", exp.unsigned_abs()));
        return s;
    }
    if exp < 0 {
        s.push('0');
        s.push(decimal);
        for _ in 0..(-exp - 1) {
            s.push('0');
        }
        s.push_str(digits);
        return s;
    }
    let int_len = usize::try_from(exp + 1).unwrap_or(1);
    if digits.len() <= int_len {
        s.push_str(digits);
        for _ in digits.len()..int_len {
            s.push('0');
        }
    } else {
        let (a, b) = digits.split_at(int_len);
        s.push_str(a);
        s.push(decimal);
        s.push_str(b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_uses_fifteen_digits() {
        assert_eq!(fixed_digits(2.675, 2), Some(("2".into(), "68".into())));
        assert_eq!(fixed_digits(0.125, 2), Some(("".into(), "13".into())));
        assert_eq!(fixed_digits(-0.5, 0), Some(("1".into(), "".into())));
        assert_eq!(fixed_digits(0.004, 2), Some(("".into(), "00".into())));
        assert_eq!(fixed_digits(9.999, 2), Some(("10".into(), "00".into())));
        assert_eq!(
            fixed_digits(1e20, 2),
            Some(("100000000000000000000".into(), "00".into()))
        );
        assert_eq!(fixed_digits(f64::NAN, 2), None);
        assert_eq!(fixed_digits(1e300, 2).map(|(i, _)| i.len()), Some(301));
    }

    #[test]
    fn grouping_and_general() {
        assert_eq!(group("1234567", ','), "1,234,567");
        assert_eq!(group("123", ','), "123");
        assert_eq!(group("", ','), "");
        assert_eq!(general(1234567.891, ','), "1234567,891");
        assert_eq!(general(1e20, '.'), "1E+20");
        assert_eq!(general(0.000123, '.'), "0.000123");
        assert_eq!(general(0.00001, '.'), "1E-05");
        assert_eq!(general(-0.5, '.'), "-0.5");
        assert_eq!(general(0.1 + 0.2, '.'), "0.3");
    }
}
