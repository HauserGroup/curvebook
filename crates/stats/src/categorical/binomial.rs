//! Binomial proportion confidence intervals (Wilson score, Clopper–Pearson exact, Agresti–Coull)
//! and exact one-sample binomial hypothesis testing.
//!
//! References:
//! - Wilson, E. B. (1927). Probable inference, the law of succession, and statistical inference.
//!   *Journal of the American Statistical Association*, 22(158), 209-212.
//! - Clopper, C. J., & Pearson, E. S. (1934). The use of confidence or fiducial limits illustrated
//!   in the case of the binomial. *Biometrika*, 26(4), 404-413.
//! - Sterne, T. E. (1954). Some remarks on confidence or fiducial limits. *Biometrika*, 41(1-2),
//!   275-278. `doi:10.1093/biomet/41.1-2.275`.
//! - Agresti, A., & Coull, B. A. (1998). Approximate is better than "exact" for interval estimation
//!   of binomial proportions. *The American Statistician*, 52(2), 119-126.

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::special::{ln_choose, reg_beta_inv};

use super::types::ExactBinomialResult;
use crate::error::StatsError;

const EXACT_BINOMIAL_MAX_TOTAL: u64 = 5_000_000;

/// Wilson (1927) score confidence interval for binomial proportion `k / n`.
#[must_use]
pub fn wilson_score_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let nf = n as f64;
    let kf = (k.min(n)) as f64;
    let p = kf / nf;
    let z = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let center = (p + z2 / (2.0 * nf)) / denom;
    let half = (z / denom) * ((p * (1.0 - p) / nf) + z2 / (4.0 * nf * nf)).sqrt();
    ((center - half).clamp(0.0, 1.0), (center + half).clamp(0.0, 1.0))
}

/// Clopper–Pearson (1934) exact binomial confidence interval via inverse Beta quantiles.
pub fn clopper_pearson_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> Result<(f64, f64), StatsError> {
    if n == 0 {
        return Ok((0.0, 1.0));
    }
    if k > n {
        return Err(StatsError::InvalidParameter { name: "binomial_successes", value: k as f64 });
    }
    let alpha = confidence.alpha();
    let half = 0.5 * alpha;
    let lo = if k == 0 { 0.0 } else { reg_beta_inv(k as f64, (n - k + 1) as f64, half)? };
    let hi = if k >= n { 1.0 } else { reg_beta_inv((k + 1) as f64, (n - k) as f64, 1.0 - half)? };
    Ok((lo.clamp(0.0, 1.0), hi.clamp(0.0, 1.0)))
}

/// Agresti–Coull (1998) confidence interval for binomial proportion `k / n`.
#[must_use]
pub fn agresti_coull_ci(k: u64, n: u64, confidence: ConfidenceLevel) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let nf = n as f64;
    let kf = (k.min(n)) as f64;
    let z = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let z2 = z * z;
    let n_tilde = nf + z2;
    let p_tilde = (kf + 0.5 * z2) / n_tilde;
    let half = z * (p_tilde * (1.0 - p_tilde) / n_tilde).sqrt();
    ((p_tilde - half).clamp(0.0, 1.0), (p_tilde + half).clamp(0.0, 1.0))
}

/// Exact one-sample binomial test for `k` successes out of `n` trials against `null_proportion in (0, 1)`.
///
/// `p_value_sterne` is the two-sided *P* of Sterne (1954, eqs. 2-3): the total probability of all
/// outcomes no more probable than the observed count (ties within a `1e-12` relative tolerance).
/// `p_value_doubled_tail` doubles the smaller one-sided tail. The CIs cite their helpers
/// (Clopper & Pearson 1934; Wilson 1927; Agresti & Coull 1998).
pub fn exact_binomial_test(k: u64, n: u64, null_proportion: f64, confidence: ConfidenceLevel) -> Result<ExactBinomialResult, StatsError> {
    if n == 0 {
        return Err(StatsError::EmptyInput);
    }
    if k > n {
        return Err(StatsError::InvalidParameter { name: "binomial_successes", value: k as f64 });
    }
    if n > EXACT_BINOMIAL_MAX_TOTAL {
        return Err(StatsError::InvalidParameter { name: "binomial_max_total", value: n as f64 });
    }
    if !null_proportion.is_finite() || null_proportion <= 0.0 || null_proportion >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "null_proportion", value: null_proportion });
    }

    let ln_p0 = null_proportion.ln();
    let ln_q0 = (1.0 - null_proportion).ln();
    let pmf_at = |j: u64| -> Result<f64, StatsError> {
        let jf = j as f64;
        let rem = (n - j) as f64;
        let log_pmf = ln_choose(n, j)? + jf * ln_p0 + rem * ln_q0;
        Ok(log_pmf.exp())
    };

    let p_obs = pmf_at(k)?;
    let threshold = p_obs * (1.0 + 1e-12);
    let mut p_lower = 0.0_f64;
    let mut p_upper = 0.0_f64;
    let mut p_sterne = 0.0_f64;

    for j in 0..=n {
        let pj = pmf_at(j)?;
        if j <= k {
            p_lower += pj;
        }
        if j >= k {
            p_upper += pj;
        }
        if pj <= threshold {
            p_sterne += pj;
        }
    }

    let p_lower = p_lower.clamp(0.0, 1.0);
    let p_upper = p_upper.clamp(0.0, 1.0);
    let p_sterne = p_sterne.clamp(0.0, 1.0);
    let p_doubled = (2.0 * p_lower.min(p_upper)).clamp(0.0, 1.0);

    Ok(ExactBinomialResult {
        successes: k,
        total: n,
        null_proportion,
        proportion: (k as f64) / (n as f64),
        p_value_lower: p_lower,
        p_value_upper: p_upper,
        p_value_sterne: p_sterne,
        p_value_doubled_tail: p_doubled,
        clopper_pearson_ci: clopper_pearson_ci(k, n, confidence)?,
        wilson_ci: wilson_score_ci(k, n, confidence),
        agresti_coull_ci: agresti_coull_ci(k, n, confidence),
    })
}
