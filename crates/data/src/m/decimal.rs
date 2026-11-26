//! Exact decimal numbers, as Power Query keeps the numbers it can: a 96-bit mantissa, a scale
//! (0 to 28 decimal places) and a sign, with the arithmetic and conversions of .NET
//! `System.Decimal` on .NET Framework (see `docs/formats/data-sources.md`, *Numbers*).
//!
//! Every operation is checked: an overflow is `None` or [`DecError`], never a panic.

/// Largest scale (decimal places).
const MAX_SCALE: u32 = 28;
/// Mantissas are below 2^96.
const LIMIT: u128 = 1 << 96;

/// A decimal number: `(-1)^neg × mant × 10^-scale`, with `mant < 2^96` and `scale <= 28`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Dec {
    lo: u64,
    hi: u32,
    scale: u8,
    neg: bool,
    /// Read from text: Desktop converts such a decimal to the double nearest to that text,
    /// not with the .NET decimal conversion.
    parsed: bool,
}

/// Why a decimal operation has no decimal result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecError {
    Overflow,
    DivideByZero,
}

/// Doubles `1e0` … `1e80`, as the .NET conversions use them.
fn pow10_f64(k: u32) -> f64 {
    const T: [f64; 29] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17,
        1e18, 1e19, 1e20, 1e21, 1e22, 1e23, 1e24, 1e25, 1e26, 1e27, 1e28,
    ];
    match T.get(k as usize) {
        Some(v) => *v,
        // Only `VarDecFromR8` asks for more, up to 1e43; each literal is correctly rounded.
        None => format!("1e{k}").parse().unwrap_or(f64::INFINITY),
    }
}

/// `10^k` for `k <= 38`.
fn pow10_u128(k: u32) -> Option<u128> {
    10u128.checked_pow(k)
}

/// A 256-bit unsigned integer (eight 32-bit limbs, least significant first), wide enough for
/// the exact intermediate results of every operation here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wide([u32; 8]);

impl Wide {
    const ZERO: Wide = Wide([0; 8]);

    fn from_u128(v: u128) -> Self {
        let mut w = [0u32; 8];
        for (i, limb) in w.iter_mut().take(4).enumerate() {
            *limb = (v >> (32 * i)) as u32;
        }
        Wide(w)
    }

    fn to_u128(self) -> Option<u128> {
        if self.0.iter().skip(4).any(|l| *l != 0) {
            return None;
        }
        Some(
            self.0
                .iter()
                .take(4)
                .enumerate()
                .fold(0u128, |acc, (i, l)| acc | (u128::from(*l) << (32 * i))),
        )
    }

    /// `self * m`; `None` on overflow.
    fn mul_small(self, m: u32) -> Option<Self> {
        let mut out = [0u32; 8];
        let mut carry = 0u64;
        for (o, l) in out.iter_mut().zip(self.0.iter()) {
            let v = u64::from(*l) * u64::from(m) + carry;
            *o = v as u32;
            carry = v >> 32;
        }
        (carry == 0).then_some(Wide(out))
    }

    /// `self * 10^k`; `None` on overflow.
    fn mul_pow10(mut self, mut k: u32) -> Option<Self> {
        while k > 0 {
            let step = k.min(9);
            self = self.mul_small(10u32.pow(step))?;
            k -= step;
        }
        Some(self)
    }

    fn add(self, o: Self) -> Option<Self> {
        let mut out = [0u32; 8];
        let mut carry = 0u64;
        for ((r, a), b) in out.iter_mut().zip(self.0.iter()).zip(o.0.iter()) {
            let v = u64::from(*a) + u64::from(*b) + carry;
            *r = v as u32;
            carry = v >> 32;
        }
        (carry == 0).then_some(Wide(out))
    }

    /// `self - o` for `self >= o`.
    fn sub(self, o: Self) -> Self {
        let mut out = [0u32; 8];
        let mut borrow = 0i64;
        for ((r, a), b) in out.iter_mut().zip(self.0.iter()).zip(o.0.iter()) {
            let mut v = i64::from(*a) - i64::from(*b) - borrow;
            borrow = 0;
            if v < 0 {
                v += 1 << 32;
                borrow = 1;
            }
            *r = v as u32;
        }
        Wide(out)
    }

    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        for (a, b) in self.0.iter().rev().zip(o.0.iter().rev()) {
            match a.cmp(b) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    }

    /// The product of two values below 2^128 (fits: below 2^256).
    fn mul(a: u128, b: u128) -> Self {
        let a = Wide::from_u128(a);
        let b = Wide::from_u128(b);
        let mut out = [0u32; 8];
        for (i, x) in a.0.iter().take(4).enumerate() {
            let mut carry = 0u64;
            for (j, y) in b.0.iter().take(4).enumerate() {
                let Some(slot) = out.get_mut(i + j) else {
                    continue;
                };
                let v = u64::from(*slot) + u64::from(*x) * u64::from(*y) + carry;
                *slot = v as u32;
                carry = v >> 32;
            }
            if let Some(slot) = out.get_mut(i + 4) {
                *slot = carry as u32;
            }
        }
        Wide(out)
    }

    /// `(self / d, self % d)` for `0 < d < 2^96`.
    fn divrem(self, d: u128) -> (Self, u128) {
        let mut q = [0u32; 8];
        let mut rem = 0u128;
        if d == 0 {
            return (Wide::ZERO, 0);
        }
        for i in (0..8).rev() {
            let limb = self.0.get(i).copied().unwrap_or(0);
            // rem < d < 2^96, so the shift cannot overflow.
            let cur = (rem << 32) | u128::from(limb);
            if let Some(slot) = q.get_mut(i) {
                *slot = (cur / d) as u32;
            }
            rem = cur % d;
        }
        (Wide(q), rem)
    }
}

/// Rounds `w × 10^-scale` (exact, with `sticky` set when nonzero digits were already dropped
/// below it) into a decimal: the largest scale at most 28 whose mantissa fits in 96 bits,
/// rounding half to even. `None` when even scale 0 overflows.
fn fit(w: Wide, scale: i64, neg: bool, sticky: bool) -> Option<Dec> {
    let (w, scale) = if scale < 0 {
        (w.mul_pow10(u32::try_from(-scale).ok()?)?, 0)
    } else {
        (w, scale)
    };
    let mut k = (scale - i64::from(MAX_SCALE)).max(0);
    loop {
        if k > scale {
            return None;
        }
        // Drop k digits, remembering the last dropped digit and whether any other was nonzero.
        let mut q = w;
        let mut last = 0u128;
        let mut rest = sticky;
        for _ in 0..k {
            rest |= last != 0;
            let (nq, r) = q.divrem(10);
            q = nq;
            last = r;
        }
        let mut m = match q.to_u128() {
            Some(m) if m < LIMIT => m,
            _ => {
                k += 1;
                continue;
            }
        };
        if k > 0 && (last > 5 || (last == 5 && (rest || m % 2 == 1))) {
            m += 1;
        }
        if m >= LIMIT {
            k += 1;
            continue;
        }
        let s = u8::try_from(scale - k).ok()?;
        return Some(Dec::raw(m, s, neg));
    }
}

impl Dec {
    pub const ZERO: Dec = Dec::raw(0, 0, false);

    /// Builds a decimal from a mantissa below 2^96.
    const fn raw(mant: u128, scale: u8, neg: bool) -> Dec {
        Dec {
            lo: mant as u64,
            hi: (mant >> 64) as u32,
            scale,
            neg,
            parsed: false,
        }
    }

    fn mant(&self) -> u128 {
        u128::from(self.lo) | (u128::from(self.hi) << 64)
    }

    /// An integer, if it fits.
    pub fn from_i128(v: i128) -> Option<Dec> {
        let mant = v.unsigned_abs();
        (mant < LIMIT).then_some(Dec::raw(mant, 0, v < 0))
    }

    pub fn is_zero(&self) -> bool {
        self.mant() == 0
    }

    pub fn is_negative(&self) -> bool {
        self.neg && self.mant() != 0
    }

    /// Digits of the mantissa (`0.0120` → 3; zero → 0).
    pub fn significant_digits(&self) -> u32 {
        let mut m = self.mant();
        let mut n = 0;
        while m > 0 {
            m /= 10;
            n += 1;
        }
        n
    }

    pub fn scale(&self) -> u32 {
        u32::from(self.scale)
    }

    pub fn neg(self) -> Dec {
        Dec {
            neg: !self.neg && self.mant() != 0,
            ..self
        }
    }

    pub fn abs(self) -> Dec {
        Dec { neg: false, ..self }
    }

    /// Removes trailing zeros from the scale (`2.50` → `2.5`).
    pub fn normalize(self) -> Dec {
        let mut m = self.mant();
        let mut s = self.scale;
        while s > 0 && m.is_multiple_of(10) {
            m /= 10;
            s -= 1;
        }
        Dec::raw(m, s, self.neg && m != 0)
    }

    /// Parses `[sign] digits [. digits] [e [sign] digits]` exactly, rounding digits beyond the
    /// decimal's precision half to even and normalizing the result. `None` when the text is
    /// not of that form, is out of range, or is a nonzero value too small to keep (it rounds to
    /// zero): Power Query reads those as doubles.
    pub fn parse(s: &str) -> Option<Dec> {
        let b = s.as_bytes();
        let mut i = 0usize;
        let mut neg = false;
        match b.first() {
            Some(b'-') => {
                neg = true;
                i = 1;
            }
            Some(b'+') => i = 1,
            _ => {}
        }
        let mut digits: Vec<u8> = Vec::new();
        let mut frac_len: i64 = 0;
        let mut seen_digit = false;
        let mut seen_point = false;
        while let Some(&c) = b.get(i) {
            match c {
                b'0'..=b'9' => {
                    seen_digit = true;
                    if seen_point {
                        frac_len = frac_len.checked_add(1)?;
                    }
                    if !(digits.is_empty() && c == b'0') {
                        digits.push(c - b'0');
                    }
                }
                b'.' if !seen_point => seen_point = true,
                _ => break,
            }
            i += 1;
        }
        if !seen_digit {
            return None;
        }
        let mut exp: i64 = 0;
        if let Some(b'e' | b'E') = b.get(i) {
            i += 1;
            let mut eneg = false;
            match b.get(i) {
                Some(b'-') => {
                    eneg = true;
                    i += 1;
                }
                Some(b'+') => i += 1,
                _ => {}
            }
            let mut any = false;
            while let Some(&c) = b.get(i).filter(|c| c.is_ascii_digit()) {
                any = true;
                exp = exp.saturating_mul(10).saturating_add(i64::from(c - b'0'));
                exp = exp.min(100_000);
                i += 1;
            }
            if !any {
                return None;
            }
            if eneg {
                exp = -exp;
            }
        }
        if i != b.len() {
            return None;
        }
        if digits.is_empty() {
            return Some(Dec::ZERO);
        }
        // value = digits × 10^(exp - frac_len); keep at most 60 digits, the rest only matter
        // for rounding.
        let mut e10 = exp.checked_sub(frac_len)?;
        let mut sticky = false;
        if digits.len() > 60 {
            let drop = digits.len() - 60;
            sticky = digits.iter().skip(60).any(|d| *d != 0);
            digits.truncate(60);
            e10 = e10.checked_add(i64::try_from(drop).ok()?)?;
        }
        // Too large: more than 29 integer digits.
        if e10.checked_add(i64::try_from(digits.len()).ok()?)? > 29 {
            return None;
        }
        // Too small to matter: rounds to zero.
        if e10.checked_add(i64::try_from(digits.len()).ok()?)? < -29 {
            return None;
        }
        let mut w = Wide::ZERO;
        for d in &digits {
            w = w.mul_small(10)?.add(Wide::from_u128(u128::from(*d)))?;
        }
        let d = fit(w, -e10, neg, sticky)?;
        if d.is_zero() {
            return None;
        }
        Some(Dec {
            parsed: true,
            ..d.normalize()
        })
    }

    /// The double nearest to the decimal as .NET Framework computes it (`VarR8FromDec`: the
    /// mantissa as a double, divided by the double `10^scale`), which is not always the
    /// correctly rounded value.
    ///
    /// A decimal read from text converts to the double nearest to its value instead.
    pub fn to_f64(self) -> f64 {
        if self.parsed
            && let Ok(x) = self.to_text('.').parse::<f64>()
        {
            return x;
        }
        let (lo, hi) = (self.lo, self.hi);
        let v = (lo as f64 + hi as f64 * 18_446_744_073_709_551_616.0) / pow10_f64(self.scale());
        if self.neg { -v } else { v }
    }

    /// A double as a decimal, the way .NET Framework converts it (`VarDecFromR8`): rounded to
    /// 15 significant digits, trailing zeros removed. `None` for NaN, infinities and values
    /// beyond the decimal range; values below it give zero.
    pub fn from_f64(input: f64) -> Option<Dec> {
        if !input.is_finite() {
            return None;
        }
        let bits = input.to_bits();
        let exp = ((bits >> 52) & 0x7FF) as i32 - 1022;
        if exp < -94 {
            return Some(Dec::ZERO);
        }
        if exp > 96 {
            return None;
        }
        let neg = input < 0.0;
        let mut dbl = input.abs();
        let mut power: i32 = 14 - ((exp * 19728) >> 16);
        if power >= 0 {
            if power > MAX_SCALE as i32 {
                power = MAX_SCALE as i32;
            }
            dbl *= pow10_f64(power as u32);
        } else if power != -1 || dbl >= 1e15 {
            dbl /= pow10_f64((-power) as u32);
        } else {
            power = 0;
        }
        if dbl < 1e14 && power < MAX_SCALE as i32 {
            dbl *= 10.0;
            power += 1;
        }
        // Round to an integer, half to even.
        let trunc = dbl.trunc();
        let mut mant = trunc as u64;
        let diff = dbl - trunc;
        if diff > 0.5 || (diff == 0.5 && mant & 1 == 1) {
            mant = mant.checked_add(1)?;
        }
        if mant == 0 {
            return Some(Dec::ZERO);
        }
        if power < 0 {
            let m = u128::from(mant).checked_mul(pow10_u128((-power) as u32)?)?;
            if m >= LIMIT {
                return None;
            }
            return Some(Dec::raw(m, 0, neg));
        }
        // Factor out powers of ten (at most 14, and never below scale 0).
        let mut lmax = power.min(14);
        for step in [8, 4, 2, 1] {
            if lmax >= step {
                let den = 10u64.pow(step as u32);
                if mant.is_multiple_of(den) {
                    mant /= den;
                    power -= step;
                    lmax -= step;
                }
            }
        }
        Some(Dec::raw(u128::from(mant), u8::try_from(power).ok()?, neg))
    }

    fn wide(&self) -> Wide {
        Wide::from_u128(self.mant())
    }

    /// `self + o`, exact when it fits, else rounded half to even.
    pub fn add(self, o: Dec) -> Result<Dec, DecError> {
        let s = self.scale.max(o.scale);
        let a = self
            .wide()
            .mul_pow10(u32::from(s - self.scale))
            .ok_or(DecError::Overflow)?;
        let b = o
            .wide()
            .mul_pow10(u32::from(s - o.scale))
            .ok_or(DecError::Overflow)?;
        let (w, neg) = if self.neg == o.neg {
            (a.add(b).ok_or(DecError::Overflow)?, self.neg)
        } else if a.cmp(&b) != std::cmp::Ordering::Less {
            (a.sub(b), self.neg)
        } else {
            (b.sub(a), o.neg)
        };
        fit(w, i64::from(s), neg, false).ok_or(DecError::Overflow)
    }

    pub fn sub(self, o: Dec) -> Result<Dec, DecError> {
        self.add(o.neg())
    }

    /// `self × o`, exact when it fits, else rounded half to even.
    pub fn mul(self, o: Dec) -> Result<Dec, DecError> {
        let w = Wide::mul(self.mant(), o.mant());
        let neg = self.neg != o.neg;
        let s = i64::from(self.scale) + i64::from(o.scale);
        match fit(w, s, neg, false) {
            Some(d) => Ok(d),
            None => Err(DecError::Overflow),
        }
    }

    /// `self / o` the way .NET does it: the exact quotient at the smallest scale (not below
    /// `self.scale - o.scale`, nor 0) that holds it, else the quotient rounded half to even at
    /// the largest scale that fits.
    pub fn div(self, o: Dec) -> Result<Dec, DecError> {
        if o.mant() == 0 {
            return Err(DecError::DivideByZero);
        }
        let neg = self.neg != o.neg;
        if self.mant() == 0 {
            let s = (i64::from(self.scale) - i64::from(o.scale)).max(0);
            return Ok(Dec::raw(0, u8::try_from(s).unwrap_or(0), neg));
        }
        let s0 = i64::from(self.scale) - i64::from(o.scale);
        let t0 = s0.max(0);
        // Mantissa at scale t: self.mant() × 10^(t - s0) / o.mant().
        let at = |t: i64| -> Option<(Wide, u128)> {
            let k = u32::try_from(t - s0).ok()?;
            Some(self.wide().mul_pow10(k)?.divrem(o.mant()))
        };
        // The largest scale whose integer quotient fits.
        let mut t = i64::from(MAX_SCALE);
        let (q, r) = loop {
            if t < t0 {
                return Err(DecError::Overflow);
            }
            match at(t) {
                Some((q, r)) if q.to_u128().is_some_and(|m| m < LIMIT) => break (q, r),
                _ => t -= 1,
            }
        };
        let mut m = q.to_u128().unwrap_or(0);
        if r == 0 {
            // Exact: drop trailing zeros down to the natural scale.
            while t > t0 && m.is_multiple_of(10) {
                m /= 10;
                t -= 1;
            }
            return Ok(Dec::raw(m, u8::try_from(t).map_err(|_| DecError::Overflow)?, neg));
        }
        // Round half to even on the remainder.
        let twice = Wide::from_u128(r).mul_small(2).ok_or(DecError::Overflow)?;
        let up = match twice.cmp(&Wide::from_u128(o.mant())) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => m % 2 == 1,
            std::cmp::Ordering::Less => false,
        };
        if up {
            m += 1;
        }
        if m >= LIMIT {
            // Rounding carried past 96 bits: one digit less.
            return fit(Wide::from_u128(m), t, neg, false).ok_or(DecError::Overflow);
        }
        Ok(Dec::raw(m, u8::try_from(t).map_err(|_| DecError::Overflow)?, neg))
    }

    /// `10^k` (`k >= 0`) or `10^k` as `1` at scale `-k`; `None` beyond the decimal range.
    pub fn pow10(k: i32) -> Option<Dec> {
        if k >= 0 {
            let mant = pow10_u128(u32::try_from(k).ok()?)?;
            (mant < LIMIT).then_some(Dec::raw(mant, 0, false))
        } else {
            let s = u8::try_from(-k).ok()?;
            (u32::from(s) <= MAX_SCALE).then_some(Dec::raw(1, s, false))
        }
    }

    fn unit(&self) -> u128 {
        pow10_u128(self.scale()).unwrap_or(1)
    }

    /// The integer part (toward zero), at scale 0.
    pub fn trunc(self) -> Dec {
        Dec::raw(self.mant() / self.unit(), 0, self.neg)
    }

    /// Whether the fractional part is exactly one half.
    pub fn is_half(&self) -> bool {
        self.scale > 0 && self.mant() % self.unit() * 2 == self.unit()
    }

    fn has_fraction(&self) -> bool {
        !self.mant().is_multiple_of(self.unit())
    }

    /// `trunc` moved one unit away from zero.
    fn trunc_away(self) -> Dec {
        let t = self.trunc();
        let m = t.mant() + 1;
        if m >= LIMIT {
            return t;
        }
        Dec::raw(m, 0, self.neg)
    }

    /// The nearest integer for a value that is not a tie (scale 0).
    pub fn round_nearest(self) -> Dec {
        let frac = self.mant() % self.unit();
        if self.scale > 0 && frac * 2 > self.unit() {
            self.trunc_away()
        } else {
            self.trunc()
        }
    }

    /// Smallest integer not below the value (scale 0).
    pub fn ceil(self) -> Dec {
        if self.has_fraction() && !self.neg {
            self.trunc_away()
        } else {
            self.trunc()
        }
    }

    /// Largest integer not above the value (scale 0).
    pub fn floor(self) -> Dec {
        if self.has_fraction() && self.neg {
            self.trunc_away()
        } else {
            self.trunc()
        }
    }

    /// Rounds away from zero to an integer (scale 0).
    pub fn away(self) -> Dec {
        if self.has_fraction() {
            self.trunc_away()
        } else {
            self.trunc()
        }
    }

    /// The value as an integer, when it is one.
    pub fn to_i128(self) -> Option<i128> {
        if self.has_fraction() {
            return None;
        }
        let m = i128::try_from(self.mant() / self.unit()).ok()?;
        Some(if self.neg { -m } else { m })
    }

    /// The mantissa's digits and the number of them before the decimal point (zero or
    /// negative for values below 0.1): `2.680` → (`[2, 6, 8, 0]`, 1).
    pub fn digits(&self) -> (Vec<u8>, i64) {
        let s = self.mant().to_string();
        let mut d: Vec<u8> = s.bytes().map(|b| b - b'0').collect();
        let scale = i64::from(self.scale);
        // Leading zeros so that the digits reach the decimal point.
        let len = i64::try_from(d.len()).unwrap_or(0);
        if len <= scale {
            let pad = usize::try_from(scale - len + 1).unwrap_or(0);
            let mut z = vec![0u8; pad];
            z.extend(d);
            d = z;
        }
        let len = i64::try_from(d.len()).unwrap_or(0);
        (d, len - scale)
    }

    /// The text .NET writes for the decimal (`ToString()`): every digit of the scale, no
    /// exponent, and no sign on zero.
    pub fn to_text(self, decimal: char) -> String {
        let (d, point) = self.digits();
        let mut s = String::new();
        if self.is_negative() {
            s.push('-');
        }
        let point = usize::try_from(point).unwrap_or(0);
        for (i, c) in d.iter().enumerate() {
            if i == point {
                s.push(decimal);
            }
            s.push(char::from(b'0' + c));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    fn t(x: Dec) -> String {
        x.to_text('.')
    }

    #[test]
    fn parse_and_print() {
        assert_eq!(t(d("2.50")), "2.5");
        assert_eq!(t(d("123456789012345678")), "123456789012345678");
        assert_eq!(
            t(d("0.12345678901234567890123456789")),
            "0.1234567890123456789012345679"
        );
        assert_eq!(
            t(d("79228162514264337593543950335")),
            "79228162514264337593543950335"
        );
        assert_eq!(Dec::parse("79228162514264337593543950335.6"), None);
        assert_eq!(
            t(d("79228162514264337593543950335.4")),
            "79228162514264337593543950335"
        );
        assert_eq!(Dec::parse("1e-29"), None);
        assert_eq!(t(d("1e-28")), "0.0000000000000000000000000001");
        assert_eq!(t(d("1.23456789e-25")), "0.0000000000000000000000001235");
        assert_eq!(t(d("1.5e20")), "150000000000000000000");
        assert_eq!(t(d("12.5e-1")), "1.25");
        assert_eq!(t(d("0e5")), "0");
        assert_eq!(t(d("-0.0")), "0");
        assert_eq!(Dec::parse("1e29"), None);
        assert_eq!(t(d("7e28")), "70000000000000000000000000000");
        assert_eq!(Dec::parse("8e28"), None);
        assert_eq!(Dec::parse("1.2.3"), None);
        assert_eq!(Dec::parse(""), None);
        assert_eq!(Dec::parse("1e"), None);
        assert_eq!(Dec::parse(&"9".repeat(400)), None);
        assert_eq!(Dec::parse(&format!("0.{}1", "0".repeat(400))), None);
    }

    #[test]
    fn to_f64_matches_dotnet() {
        // Checked with .NET Framework 4.8 ((double)decimal).
        let q = |a: i128, b: i128| {
            Dec::from_i128(a)
                .unwrap()
                .div(Dec::from_i128(b).unwrap())
                .unwrap()
                .to_f64()
        };
        assert_eq!(format!("{:?}", q(7, 3)), "2.3333333333333335");
        assert_eq!(format!("{:?}", q(29, 7)), "4.142857142857142");
        assert_eq!(d("0.1").to_f64(), 0.1);
        assert_eq!(d("123456789012345678").to_f64(), 123456789012345680.0);
    }

    #[test]
    fn from_f64_matches_dotnet() {
        let f = |x: f64| t(Dec::from_f64(x).unwrap());
        assert_eq!(f(2.5), "2.5");
        assert_eq!(f(1.0 / 3.0), "0.333333333333333");
        assert_eq!(f(0.1 + 0.2), "0.3");
        assert_eq!(f(1e20), "100000000000000000000");
        assert_eq!(f(1e20 / 3.0), "33333333333333300000");
        assert_eq!(f(1.005), "1.005");
        assert_eq!(f(-2.675), "-2.675");
        assert_eq!(f(1e-30), "0");
        assert_eq!(f(0.0), "0");
        assert_eq!(Dec::from_f64(1e30), None);
        assert_eq!(Dec::from_f64(f64::NAN), None);
        assert_eq!(f(9.223372036854776e18), "9223372036854780000");
    }

    #[test]
    fn arithmetic_scales() {
        assert_eq!(t(d("1.1").add(d("2.2")).unwrap()), "3.3");
        assert_eq!(t(d("2.68").sub(d("0.68")).unwrap()), "2.00");
        let r = |m: u128, s: u8| Dec::raw(m, s, false);
        assert_eq!(t(r(2680, 3).mul(d("10")).unwrap()), "26.800");
        assert_eq!(t(r(2680, 3).div(d("2")).unwrap()), "1.340");
        assert_eq!(t(r(268_000, 3).div(d("100")).unwrap()), "2.680");
        assert_eq!(t(r(2200, 2).div(d("10")).unwrap()), "2.20");
        assert_eq!(t(r(12_000, 3).div(d("0.01")).unwrap()), "1200.0");
        assert_eq!(t(d("1234").div(d("0.1")).unwrap()), "12340");
        assert_eq!(t(d("7").div(d("3")).unwrap()), "2.3333333333333333333333333333");
        assert_eq!(t(d("1").div(d("8")).unwrap()), "0.125");
        assert_eq!(
            t(d("4.75").div(d("3")).unwrap()),
            "1.5833333333333333333333333333"
        );
        assert_eq!(
            d("79228162514264337593543950335").add(d("1")),
            Err(DecError::Overflow)
        );
        assert_eq!(d("1").div(Dec::ZERO), Err(DecError::DivideByZero));
        assert_eq!(
            t(d("0.1234567890123456785").mul(d("1000000000000000000")).unwrap()),
            "123456789012345678.50000000000"
        );
    }

    #[test]
    fn rounding_helpers() {
        assert_eq!(t(d("267.5").trunc()), "267");
        assert!(d("267.500").is_half());
        assert!(!d("267.4").is_half());
        assert_eq!(t(d("-2.5").ceil()), "-2");
        assert_eq!(t(d("-2.5").floor()), "-3");
        assert_eq!(t(d("2.4").away()), "3");
        assert_eq!(t(d("2.6").round_nearest()), "3");
        assert_eq!(d("123").to_i128(), Some(123));
        assert_eq!(d("1.5").to_i128(), None);
    }
}
