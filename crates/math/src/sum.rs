//! Neumaier compensated summation, error-free dot products, and Chan-Golub-LeVeque shifted
//! two-pass summary moments.
//!
//! Retains >= 14-15 significant digits on stiff benchmark series such as NIST StRD `NumAcc1`-`NumAcc4`
//! where `|mean| / sd > 10^8`.
//!
//! References:
//! - Neumaier, A. (1974). Rundungsfehleranalyse einiger Verfahren zur Summation endlicher Summen.
//!   *ZAMM*, 54(1), 39-51.
//! - Chan, T. F., Golub, G. H., & LeVeque, R. J. (1983). Algorithms for computing the sample
//!   variance: Analysis and recommendations. *The American Statistician*, 37(3), 242-247.
//! - Ogita, T., Rump, S. M., & Oishi, S. (2005). Accurate sum and dot product.
//!   *SIAM Journal on Scientific Computing*, 26(6), 1955-1988.

use crate::error::MathError;

/// Streaming Neumaier (improved Kahan-Babuska) compensated summation accumulator.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeumaierSum {
    sum: f64,
    compensation: f64,
    count: usize,
}

impl NeumaierSum {
    /// Create a zeroed compensated accumulator.
    #[must_use]
    pub const fn new() -> Self {
        Self { sum: 0.0, compensation: 0.0, count: 0 }
    }

    /// Accumulate a single `f64` value while preserving IEEE-754 non-finite semantics.
    #[inline]
    pub fn add(&mut self, value: f64) {
        self.count += 1;
        if !value.is_finite() || !self.sum.is_finite() {
            self.sum += value;
            return;
        }
        let tentative = self.sum + value;
        if self.sum.abs() >= value.abs() {
            self.compensation += (self.sum - tentative) + value;
        } else {
            self.compensation += (value - tentative) + self.sum;
        }
        self.sum = tentative;
    }

    /// Accumulate every element from an iterator.
    pub fn extend_iter<I: IntoIterator<Item = f64>>(&mut self, values: I) {
        for v in values {
            self.add(v);
        }
    }

    /// Number of values fed into the accumulator.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Compensated sum of all accumulated terms.
    #[must_use]
    #[inline]
    pub fn total(&self) -> f64 {
        if self.sum.is_finite() { self.sum + self.compensation } else { self.sum }
    }
}

/// Compute the Neumaier compensated sum of a slice.
#[must_use]
pub fn sum(values: &[f64]) -> f64 {
    let mut acc = NeumaierSum::new();
    for &v in values {
        acc.add(v);
    }
    acc.total()
}

/// Compute the Neumaier compensated sum of an arbitrary `f64` iterator.
#[must_use]
pub fn sum_iter<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    let mut acc = NeumaierSum::new();
    acc.extend_iter(values);
    acc.total()
}

/// Compensated inner product `sum_i a_i * b_i` using error-free FMA product splitting (`Dot2`).
///
/// # Errors
/// Returns [`MathError::DimensionMismatch`] if `a.len() != b.len()`.
pub fn dot(a: &[f64], b: &[f64]) -> Result<f64, MathError> {
    if a.len() != b.len() {
        return Err(MathError::DimensionMismatch { expected: a.len(), actual: b.len() });
    }
    let mut prod_acc = NeumaierSum::new();
    let mut err_acc = NeumaierSum::new();
    for (&x, &y) in a.iter().zip(b.iter()) {
        let p = x * y;
        if p.is_finite() {
            let err = x.mul_add(y, -p);
            prod_acc.add(p);
            err_acc.add(err);
        } else {
            prod_acc.add(p);
        }
    }
    let primary = prod_acc.total();
    if primary.is_finite() { Ok(primary + err_acc.total()) } else { Ok(primary) }
}

/// Compensated univariate location and dispersion summary computed via Chan-Golub-LeVeque's
/// shifted two-pass algorithm with Neumaier compensation.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SummaryMoments {
    /// Number of finite observations `n`.
    pub count: usize,
    /// Compensated total `sum_i x_i`.
    pub sum: f64,
    /// Sample arithmetic mean `sum / n`.
    pub mean: f64,
    /// Centered sum of squared deviations `sum_i (x_i - mean)^2 >= 0`.
    pub css: f64,
}

impl SummaryMoments {
    /// Compute compensated moments over a non-empty slice of finite observations.
    ///
    /// # Errors
    /// Returns [`MathError::EmptyInput`] if `values.is_empty()` or [`MathError::DomainError`] if any
    /// element is `NaN` or infinite.
    pub fn from_slice(values: &[f64]) -> Result<Self, MathError> {
        if values.is_empty() {
            return Err(MathError::EmptyInput);
        }
        let mut sum_acc = NeumaierSum::new();
        for &x in values {
            if !x.is_finite() {
                return Err(MathError::DomainError { name: "x", value: x });
            }
            sum_acc.add(x);
        }
        let count = values.len();
        let n_f = count as f64;
        let total = sum_acc.total();
        let mean = total / n_f;
        if !mean.is_finite() {
            return Err(MathError::DomainError { name: "mean", value: mean });
        }

        let mut dev_acc = NeumaierSum::new();
        let mut sq_acc = NeumaierSum::new();
        for &x in values {
            let d = x - mean;
            dev_acc.add(d);
            let sq = d * d;
            let sq_err = d.mul_add(d, -sq);
            sq_acc.add(sq);
            if sq_err.is_finite() {
                sq_acc.add(sq_err);
            }
        }
        let residual = dev_acc.total();
        let css = (sq_acc.total() - (residual * residual) / n_f).max(0.0);
        Ok(Self { count, sum: total, mean, css })
    }

    /// Unbiased sample variance `css / (n - 1)`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] when `count < 2`.
    pub fn sample_variance(&self) -> Result<f64, MathError> {
        if self.count < 2 {
            return Err(MathError::InvalidParameter { name: "n", value: self.count as f64 });
        }
        Ok(self.css / ((self.count - 1) as f64))
    }

    /// Sample standard deviation `sqrt(css / (n - 1))`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] when `count < 2`.
    pub fn sample_sd(&self) -> Result<f64, MathError> {
        Ok(self.sample_variance()?.sqrt())
    }

    /// Standard error of the mean `sample_sd / sqrt(n)`.
    ///
    /// # Errors
    /// Returns [`MathError::InvalidParameter`] when `count < 2`.
    pub fn sem(&self) -> Result<f64, MathError> {
        Ok(self.sample_sd()? / (self.count as f64).sqrt())
    }

    /// Population variance `css / n`.
    #[must_use]
    pub fn population_variance(&self) -> f64 {
        self.css / (self.count as f64)
    }

    /// Population standard deviation `sqrt(css / n)`.
    #[must_use]
    pub fn population_sd(&self) -> f64 {
        self.population_variance().sqrt()
    }
}

/// Compensated arithmetic mean of a non-empty finite slice.
///
/// # Errors
/// Returns [`MathError::EmptyInput`] or [`MathError::DomainError`] on invalid input.
pub fn mean(values: &[f64]) -> Result<f64, MathError> {
    Ok(SummaryMoments::from_slice(values)?.mean)
}

/// Centered sum of squared deviations `sum_i (x_i - mean)^2` via shifted two-pass Neumaier sum.
///
/// # Errors
/// Returns [`MathError::EmptyInput`] or [`MathError::DomainError`] on invalid input.
pub fn centered_sum_of_squares(values: &[f64]) -> Result<f64, MathError> {
    Ok(SummaryMoments::from_slice(values)?.css)
}

/// Unbiased sample variance (`n - 1` denominator) of a finite slice with `n >= 2`.
///
/// # Errors
/// Returns an error if `values.len() < 2` or any value is non-finite.
pub fn sample_variance(values: &[f64]) -> Result<f64, MathError> {
    SummaryMoments::from_slice(values)?.sample_variance()
}

/// Sample standard deviation (`n - 1` denominator) of a finite slice with `n >= 2`.
///
/// # Errors
/// Returns an error if `values.len() < 2` or any value is non-finite.
pub fn sample_sd(values: &[f64]) -> Result<f64, MathError> {
    SummaryMoments::from_slice(values)?.sample_sd()
}

/// Shifted two-pass compensated sample covariance (`n - 1` denominator).
///
/// # Errors
/// Returns an error if `x.len() != y.len()`, `x.len() < 2`, or any element is non-finite.
pub fn covariance(x: &[f64], y: &[f64]) -> Result<f64, MathError> {
    if x.len() != y.len() {
        return Err(MathError::DimensionMismatch { expected: x.len(), actual: y.len() });
    }
    if x.len() < 2 {
        return Err(MathError::InvalidParameter { name: "n", value: x.len() as f64 });
    }
    let mx = mean(x)?;
    let my = mean(y)?;
    let n_f = x.len() as f64;

    let mut dx_acc = NeumaierSum::new();
    let mut dy_acc = NeumaierSum::new();
    let mut cross_acc = NeumaierSum::new();
    for (&xi, &yi) in x.iter().zip(y.iter()) {
        if !yi.is_finite() {
            return Err(MathError::DomainError { name: "y", value: yi });
        }
        let dx = xi - mx;
        let dy = yi - my;
        dx_acc.add(dx);
        dy_acc.add(dy);
        let p = dx * dy;
        let err = dx.mul_add(dy, -p);
        cross_acc.add(p);
        if err.is_finite() {
            cross_acc.add(err);
        }
    }
    let cross_centered = cross_acc.total() - (dx_acc.total() * dy_acc.total()) / n_f;
    Ok(cross_centered / ((x.len() - 1) as f64))
}

/// Compensated Pearson product-moment correlation coefficient `r` in `[-1, 1]`.
///
/// # Errors
/// Returns [`MathError::SingularMatrix`] if either series has zero variance, or a dimension/domain error.
pub fn pearson_r(x: &[f64], y: &[f64]) -> Result<f64, MathError> {
    if x.len() != y.len() {
        return Err(MathError::DimensionMismatch { expected: x.len(), actual: y.len() });
    }
    if x.len() < 2 {
        return Err(MathError::InvalidParameter { name: "n", value: x.len() as f64 });
    }
    let mx = SummaryMoments::from_slice(x)?;
    let my = SummaryMoments::from_slice(y)?;
    if mx.css == 0.0 || my.css == 0.0 {
        return Err(MathError::DomainError { name: "variance", value: 0.0 });
    }
    let cov = covariance(x, y)?;
    let denom = (mx.sample_variance()? * my.sample_variance()?).sqrt();
    Ok((cov / denom).clamp(-1.0, 1.0))
}

/// Sample lag-1 autocorrelation coefficient matching the NIST StRD univariate definition:
/// `r_1 = sum_{i=0}^{n-2} (x_i - mean)(x_{i+1} - mean) / sum_{i=0}^{n-1} (x_i - mean)^2`.
///
/// # Errors
/// Returns an error if `values.len() < 2`, any value is non-finite, or all values are identical.
pub fn lag1_autocorrelation(values: &[f64]) -> Result<f64, MathError> {
    if values.len() < 2 {
        return Err(MathError::InvalidParameter { name: "n", value: values.len() as f64 });
    }
    let moments = SummaryMoments::from_slice(values)?;
    if moments.css == 0.0 {
        return Err(MathError::DomainError { name: "variance", value: 0.0 });
    }
    let m = moments.mean;
    let mut dev_all = NeumaierSum::new();
    for &x in values {
        dev_all.add(x - m);
    }
    let delta = dev_all.total() / (values.len() as f64);

    let mut acc = NeumaierSum::new();
    let mut pair_dev_acc = NeumaierSum::new();
    for pair in values.windows(2) {
        if let [a, b] = *pair {
            let da = a - m;
            let db = b - m;
            pair_dev_acc.add(da);
            pair_dev_acc.add(db);
            let p = da * db;
            let err = da.mul_add(db, -p);
            acc.add(p);
            if err.is_finite() {
                acc.add(err);
            }
        }
    }
    acc.add(-delta * pair_dev_acc.total());
    acc.add(((values.len() - 1) as f64) * delta * delta);
    Ok((acc.total() / moments.css).clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::{NeumaierSum, SummaryMoments, covariance, dot, lag1_autocorrelation, pearson_r, sample_sd, sum};

    #[test]
    fn neumaier_beats_cancellation() {
        let vals = [1.0, 1e100, 1.0, -1e100];
        assert_eq!(sum(&vals), 2.0);
    }

    #[test]
    fn neumaier_preserves_ieee_infinities_and_nan() {
        let mut acc = NeumaierSum::new();
        acc.add(f64::INFINITY);
        acc.add(1.0);
        assert!(acc.total().is_infinite() && acc.total().is_sign_positive());

        let mut nan_acc = NeumaierSum::new();
        nan_acc.add(1.0);
        nan_acc.add(f64::NAN);
        assert!(nan_acc.total().is_nan());
    }

    #[test]
    fn compensated_dot_resolves_ill_conditioned_cancellation() {
        let a = [1e50, 1.0, -1e50, 3.0];
        let b = [1e50, 2.0, 1e50, 4.0];
        assert_eq!(dot(&a, &b).unwrap(), 14.0);
    }

    #[test]
    fn stiff_variance_numacc1_and_numacc4_exact() {
        // NIST StRD NumAcc1: [10000001, 10000002, 10000003] -> mean = 10000002, sd = 1.0
        let numacc1 = [10_000_001.0, 10_000_002.0, 10_000_003.0];
        let m1 = SummaryMoments::from_slice(&numacc1).unwrap();
        assert!((m1.mean - 10_000_002.0).abs() <= 1e-15 * m1.mean);
        assert!((m1.sample_sd().unwrap() - 1.0).abs() <= 1e-15);

        // Binary-exact stiff benchmark at mean / sd = 8e8 (> 10^8) retains full 15+ digits:
        let mut stiff_bin = Vec::with_capacity(1001);
        let base = 100_000_000.25_f64;
        stiff_bin.push(base);
        for _ in 0..500 {
            stiff_bin.push(base - 0.125);
            stiff_bin.push(base + 0.125);
        }
        assert!((sample_sd(&stiff_bin).unwrap() - 0.125).abs() < 1e-15);

        // NIST StRD NumAcc4: 1001 observations centered at 10_000_000.2 with decimal sd = 0.1.
        // Because 10_000_000.1_f64 and 10_000_000.3_f64 round at 2^-29 (~1.86e-9), the exact SD of
        // the IEEE-754 f64 vector is sqrt(0.5 * ((x_hi - m)^2 + (x_lo - m)^2)) = 0.100000000558793545.
        let mut numacc4 = Vec::with_capacity(1001);
        numacc4.push(10_000_000.2);
        for _ in 0..500 {
            numacc4.push(10_000_000.1);
            numacc4.push(10_000_000.3);
        }
        let sd4 = sample_sd(&numacc4).unwrap();
        let exact_f64_sd4 = (0.5 * ((10_000_000.3_f64 - 10_000_000.2_f64).powi(2) + (10_000_000.2_f64 - 10_000_000.1_f64).powi(2))).sqrt();
        assert!((sd4 - exact_f64_sd4).abs() < 1e-15, "sd4 = {sd4:.17e} vs {exact_f64_sd4:.17e}");
        assert!((sd4 - 0.1).abs() < 1e-9, "sd4 = {sd4:.17e}");
        let r1 = lag1_autocorrelation(&numacc1).unwrap();
        assert!(r1.is_finite());
    }

    #[test]
    fn covariance_and_pearson_r_shifted_invariance() {
        let x = [1e9 + 1.0, 1e9 + 2.0, 1e9 + 3.0, 1e9 + 4.0];
        let y = [2e9 + 2.0, 2e9 + 4.0, 2e9 + 6.0, 2e9 + 8.0];
        assert!((covariance(&x, &y).unwrap() - (10.0 / 3.0)).abs() < 1e-14);
        assert!((pearson_r(&x, &y).unwrap() - 1.0).abs() < 1e-15);
    }
}
