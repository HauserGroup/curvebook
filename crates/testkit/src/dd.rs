//! Exact error-free `DoubleDouble` (`hi + lo`, ~106-bit significand / ~31 decimal digits) reference
//! arithmetic used inside `curvebook-testkit` to certify synthetic stiff benchmarks without
//! external multiprecision crates.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Non-overlapping pair `hi + lo` (`|lo| <= 0.5 * ulp(hi)`) providing ~106 bits of precision.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DoubleDouble {
    pub hi: f64,
    pub lo: f64,
}

#[inline]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let v = s - a;
    let e = (a - (s - v)) + (b - v);
    (s, e)
}

#[inline]
fn quick_two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let e = b - (s - a);
    (s, e)
}

#[inline]
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    let e = a.mul_add(b, -p);
    (p, e)
}

impl DoubleDouble {
    pub const ZERO: Self = Self { hi: 0.0, lo: 0.0 };
    pub const ONE: Self = Self { hi: 1.0, lo: 0.0 };

    #[must_use]
    #[inline]
    pub const fn from_f64(x: f64) -> Self {
        Self { hi: x, lo: 0.0 }
    }

    #[must_use]
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    #[must_use]
    pub fn abs(self) -> Self {
        if self.hi < 0.0 || (self.hi == 0.0 && self.lo < 0.0) { -self } else { self }
    }

    #[must_use]
    pub fn sqr(self) -> Self {
        let (p1, p2) = two_prod(self.hi, self.hi);
        let c = p2 + 2.0 * self.hi * self.lo + self.lo * self.lo;
        let (hi, lo) = quick_two_sum(p1, c);
        Self { hi, lo }
    }

    #[must_use]
    pub fn sqrt(self) -> Self {
        if self.hi <= 0.0 && self.lo <= 0.0 {
            return Self::ZERO;
        }
        let x0 = self.hi.sqrt();
        let approx = Self::from_f64(x0);
        // Two Newton refinements reach full 106-bit accuracy: y_{k+1} = 0.5 * (y_k + self / y_k).
        let y1 = (approx + self / approx) * Self::from_f64(0.5);
        (y1 + self / y1) * Self::from_f64(0.5)
    }

    #[must_use]
    pub fn powi(self, exp: i32) -> Self {
        if exp == 0 {
            return Self::ONE;
        }
        let mut base = if exp < 0 { Self::ONE / self } else { self };
        let mut n = exp.unsigned_abs();
        let mut acc = Self::ONE;
        while n > 0 {
            if n & 1 == 1 {
                acc = acc * base;
            }
            n >>= 1;
            if n > 0 {
                base = base.sqr();
            }
        }
        acc
    }
}

impl From<f64> for DoubleDouble {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

impl Neg for DoubleDouble {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self { hi: -self.hi, lo: -self.lo }
    }
}

impl Add for DoubleDouble {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        let (s1, s2) = two_sum(self.hi, rhs.hi);
        let (t1, t2) = two_sum(self.lo, rhs.lo);
        let c = s2 + t1;
        let (v_hi, v_lo) = quick_two_sum(s1, c);
        let w = t2 + v_lo;
        let (hi, lo) = quick_two_sum(v_hi, w);
        Self { hi, lo }
    }
}

impl AddAssign for DoubleDouble {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sub for DoubleDouble {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self + (-rhs)
    }
}

impl SubAssign for DoubleDouble {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul for DoubleDouble {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let (p1, p2) = two_prod(self.hi, rhs.hi);
        let c = p2 + self.hi * rhs.lo + self.lo * rhs.hi;
        let (hi, lo) = quick_two_sum(p1, c);
        Self { hi, lo }
    }
}

impl Div for DoubleDouble {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        let q1 = self.hi / rhs.hi;
        let r = self - Self::from_f64(q1) * rhs;
        let q2 = r.hi / rhs.hi;
        let r2 = r - Self::from_f64(q2) * rhs;
        let q3 = r2.hi / rhs.hi;
        let (s1, s2) = quick_two_sum(q1, q2);
        Self { hi: s1, lo: s2 } + Self::from_f64(q3)
    }
}

/// Compute `(mean, sample_sd, lag1_r)` in 106-bit `DoubleDouble` precision for exact `f64`-input oracles.
#[must_use]
pub fn oracle_univariate_f64(values: &[f64]) -> (f64, f64, f64) {
    if values.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let n = DoubleDouble::from_f64(values.len() as f64);
    let mut total = DoubleDouble::ZERO;
    for &x in values {
        total += DoubleDouble::from_f64(x);
    }
    let mean = total / n;

    let mut css = DoubleDouble::ZERO;
    for &x in values {
        let d = DoubleDouble::from_f64(x) - mean;
        css += d.sqr();
    }
    let sd = if values.len() >= 2 { (css / DoubleDouble::from_f64((values.len() - 1) as f64)).sqrt().to_f64() } else { 0.0 };

    let mut auto_num = DoubleDouble::ZERO;
    for pair in values.windows(2) {
        if let [a, b] = *pair {
            let da = DoubleDouble::from_f64(a) - mean;
            let db = DoubleDouble::from_f64(b) - mean;
            auto_num += da * db;
        }
    }
    let r1 = if css.hi > 0.0 { (auto_num / css).to_f64() } else { f64::NAN };
    (mean.to_f64(), sd, r1)
}
