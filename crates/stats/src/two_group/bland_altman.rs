//! Bland–Altman method-comparison bias and Limits of Agreement.
//!
//! # References
//!
//! - Bland, J. M., & Altman, D. G. (1986). Statistical methods for assessing agreement between two
//!   methods of clinical measurement. *Lancet* 327(8476):307–310. doi:10.1016/S0140-6736(86)90837-8
//! - Bland, J. M., & Altman, D. G. (1999). Measuring agreement in method comparison studies.
//!   *Statistical Methods in Medical Research* 8(2):135–160. doi:10.1177/096228029900800204

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::common::{t_crit_two_sided, validate_finite_slice};
use crate::error::StatsError;

/// Bland–Altman method-comparison bias and Limits of Agreement (Bland & Altman 1986, 1999).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlandAltmanResult {
    /// Number of paired method measurements included.
    pub n: usize,
    /// True if differences are expressed as percentages `100 * (a - b) / ((a + b) / 2)`; false for raw difference `a - b`.
    pub percent_difference: bool,
    /// Mean bias `mean(d)` across paired differences (in raw difference units or percent).
    pub bias: f64,
    /// Sample standard deviation of paired differences `sd(d)`.
    pub sd_bias: f64,
    /// Standard error of mean bias `sd(d) / sqrt(n)`.
    pub se_bias: f64,
    /// Two-sided `t(n - 1)` confidence interval `(lower, upper)` for mean `bias` at configured `ConfidenceLevel`.
    pub bias_ci: (f64, f64),
    /// Lower Limit of Agreement `bias - z_{alpha/2} * sd_bias`.
    pub loa_lower: f64,
    /// Upper Limit of Agreement `bias + z_{alpha/2} * sd_bias`.
    pub loa_upper: f64,
    /// Two-sided Bland–Altman (1999) `t(n - 1)` confidence interval for `loa_lower` using `SE_LoA = sd_bias * sqrt(1/n + z^2 / (2(n - 1)))`.
    pub loa_lower_ci: (f64, f64),
    /// Two-sided Bland–Altman (1999) `t(n - 1)` confidence interval for `loa_upper` using `SE_LoA`.
    pub loa_upper_ci: (f64, f64),
    /// `(average, difference_or_percent_diff)` points for Bland–Altman plotting.
    pub points: Vec<(f64, f64)>,
}

/// Compute Bland–Altman agreement (`method_a - method_b` vs `(method_a + method_b) / 2`).
///
/// Bias `mean(d)` with a *t* interval and limits of agreement `mean(d) ± z * sd(d)` (Bland & Altman
/// 1986), where `z` is the normal quantile for the requested confidence (1.96 at 95%). The limits'
/// intervals use the approximate standard error `sd(d) * sqrt(1/n + z^2 / (2(n - 1)))` with a
/// *t*(`n - 1`) multiplier (Bland & Altman 1999; the 1986 paper's `sqrt(3 sd^2 / n)` is the
/// `z ≈ 2` simplification). The optional percent difference `100 (a - b) / mean(a, b)` is discussed in
/// Bland & Altman (1999) and requires strictly positive paired measurements (`a > 0` and `b > 0`).
pub fn bland_altman(
    method_a: &[f64],
    method_b: &[f64],
    percent_difference: bool,
    confidence: ConfidenceLevel,
) -> Result<BlandAltmanResult, StatsError> {
    validate_finite_slice(method_a)?;
    validate_finite_slice(method_b)?;
    if method_a.len() != method_b.len() {
        return Err(StatsError::DimensionMismatch { expected: method_a.len(), actual: method_b.len() });
    }
    let n = method_a.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Bland-Altman", required: 2, actual: n });
    }
    let mut points = Vec::with_capacity(n);
    let mut diffs = Vec::with_capacity(n);
    for (&a, &b) in method_a.iter().zip(method_b.iter()) {
        let avg = if (a + b).is_finite() { 0.5 * (a + b) } else { 0.5 * a + 0.5 * b };
        if !avg.is_finite() {
            return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: avg });
        }
        let d = if percent_difference {
            if avg <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "average", value: avg });
            }
            if a <= 0.0 || b <= 0.0 {
                return Err(StatsError::InvalidParameter { name: "percent_difference_measurement", value: a.min(b) });
            }
            if (a - b).is_finite() {
                (100.0 / avg) * (a - b)
            } else {
                let scale = a.abs().max(b.abs());
                100.0 * ((a / scale - b / scale) / (avg / scale))
            }
        } else {
            a - b
        };
        if !d.is_finite() {
            return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: d });
        }
        points.push((avg, d));
        diffs.push(d);
    }

    let max_abs = diffs.iter().fold(0.0_f64, |acc, &v| acc.max(v.abs()));
    let (bias, sd_bias, se_bias) = if max_abs == 0.0 {
        (0.0, 0.0, 0.0)
    } else {
        let scaled: Vec<f64> = diffs.iter().map(|&d| d / max_abs).collect();
        let md = SummaryMoments::from_slice(&scaled)?;
        (md.mean * max_abs, md.sample_sd()? * max_abs, md.sem()? * max_abs)
    };
    let nf = n as f64;
    let df = nf - 1.0;
    let z_loa = norm_two_sided_isf(confidence.alpha())?;
    let t_crit = t_crit_two_sided(confidence, df)?;

    let loa_lower = bias - z_loa * sd_bias;
    let loa_upper = bias + z_loa * sd_bias;
    let se_loa = sd_bias * (1.0 / nf + (z_loa * z_loa) / (2.0 * df)).sqrt();
    let bias_ci = (bias - t_crit * se_bias, bias + t_crit * se_bias);
    let loa_lower_ci = (loa_lower - t_crit * se_loa, loa_lower + t_crit * se_loa);
    let loa_upper_ci = (loa_upper - t_crit * se_loa, loa_upper + t_crit * se_loa);
    let outputs_finite =
        [bias, sd_bias, se_bias, loa_lower, loa_upper, bias_ci.0, bias_ci.1, loa_lower_ci.0, loa_lower_ci.1, loa_upper_ci.0, loa_upper_ci.1]
            .iter()
            .all(|v| v.is_finite());
    if !outputs_finite {
        return Err(StatsError::InvalidParameter { name: "bland_altman_overflow", value: loa_upper });
    }

    Ok(BlandAltmanResult { n, percent_difference, bias, sd_bias, se_bias, bias_ci, loa_lower, loa_upper, loa_lower_ci, loa_upper_ci, points })
}
