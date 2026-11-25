//! Inter-rater agreement statistics: Cohen's kappa (unweighted, linear-weighted, and
//! quadratic-weighted) and Fleiss' multi-rater kappa.
//!
//! References:
//! - Cohen, J. (1960). A coefficient of agreement for nominal scales. *Educational and Psychological
//!   Measurement*, 20(1), 37-46. `doi:10.1177/001316446002000104`.
//! - Cohen, J. (1968). Weighted kappa: Nominal scale agreement with provision for scaled disagreement
//!   or partial credit. *Psychological Bulletin*, 70(4), 213-220. `doi:10.1037/h0026256`.
//! - Fleiss, J. L., Cohen, J., & Everitt, B. S. (1969). Large sample standard errors of kappa and
//!   weighted kappa. *Psychological Bulletin*, 72(5), 323-327. `doi:10.1037/h0028106`.
//! - Fleiss, J. L. (1971). Measuring nominal scale agreement among many raters. *Psychological
//!   Bulletin*, 76(5), 378-382. `doi:10.1037/h0031619`.
//! - Fleiss, J. L., Nee, J. C. M., & Landis, J. R. (1979). Large sample variance of kappa in the case
//!   of different sets of raters. *Psychological Bulletin*, 86(5), 974-977.
//!   `doi:10.1037/0033-2909.86.5.974`.
//! - Fleiss, J. L., Levin, B., & Paik, M. C. (2003). *Statistical Methods for Rates and Proportions*
//!   (3rd ed.). Wiley. (Non-null asymptotic standard errors for Fleiss's multi-rater kappa, ch. 18).
//! - Gwet, K. L. (2008). Computing inter-rater reliability and its variance in the presence of high
//!   agreement. *British Journal of Mathematical and Statistical Psychology*, 61(1), 29-48.
//!   `doi:10.1348/000711006X126600`.

use curvebook_doc::KappaWeighting;
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_sf, norm_two_sided_isf};
use curvebook_math::sum::NeumaierSum;

use super::contingency::{checked_sum_u64, checked_table_margins};
use super::types::KappaResult;
use crate::error::StatsError;

/// Cohen's kappa (unweighted, linear-weighted, or quadratic-weighted) on a square `K x K` matrix.
///
/// Cohen (1960) unweighted and Cohen (1968) weighted kappa, with the large-sample standard errors
/// (non-null and under `H0: kappa = 0`) of Fleiss, Cohen & Everitt (1969).
pub fn cohens_kappa(counts: &[Vec<u64>], weighting: KappaWeighting, confidence: ConfidenceLevel) -> Result<KappaResult, StatsError> {
    let k = counts.len();
    if k < 2 || counts.iter().any(|r| r.len() != k) {
        return Err(StatsError::InvalidParameter { name: "square_matrix_k", value: k as f64 });
    }
    let (row_sums, _col_sums, total) = checked_table_margins(counts)?;
    if total == 0 {
        return Err(StatsError::EmptyInput);
    }
    let nf = total as f64;
    let row_p: Vec<f64> = row_sums.into_iter().map(|s| (s as f64) / nf).collect();
    let mut col_p = vec![0.0_f64; k];
    for row in counts {
        for (j, &v) in row.iter().enumerate() {
            col_p[j] += (v as f64) / nf;
        }
    }

    let weight = |i: usize, j: usize| -> f64 {
        let diff = i.abs_diff(j) as f64;
        let max_d = (k - 1) as f64;
        match weighting {
            KappaWeighting::Unweighted => {
                if i == j {
                    1.0
                } else {
                    0.0
                }
            }
            KappaWeighting::Linear => 1.0 - diff / max_d,
            KappaWeighting::Quadratic => 1.0 - (diff / max_d).powi(2),
        }
    };

    let mut po = 0.0_f64;
    let mut pe = 0.0_f64;
    let mut w_i_bar = vec![0.0_f64; k];
    let mut w_j_bar = vec![0.0_f64; k];
    for i in 0..k {
        for j in 0..k {
            let w = weight(i, j);
            let pij = (counts[i][j] as f64) / nf;
            po += w * pij;
            pe += w * row_p[i] * col_p[j];
            w_i_bar[i] += w * col_p[j];
            w_j_bar[j] += w * row_p[i];
        }
    }

    if (1.0 - pe).abs() <= 1e-12 {
        return Err(StatsError::ZeroVariance { context: "Cohen's kappa (expected agreement p_e = 1)" });
    }
    let kappa = (po - pe) / (1.0 - pe);

    // Asymptotic SE (Fleiss, Cohen & Everitt 1969, Psychol. Bull. 72:323-327):
    // subtract the squared expectation [p_o - 2 * p_e * (1 - kappa)]^2 before normalizing by N(1 - p_e)^2.
    let mut var_sum = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            let pij = (counts[i][j] as f64) / nf;
            let term = weight(i, j) - (w_i_bar[i] + w_j_bar[j]) * (1.0 - kappa);
            var_sum += pij * term * term;
        }
    }
    let denom_sq = (1.0 - pe) * (1.0 - pe);
    let mean_term = po - 2.0 * pe * (1.0 - kappa);
    let se = ((var_sum - mean_term * mean_term).max(0.0) / (nf * denom_sq)).sqrt();

    let z_crit = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let ci = ((kappa - z_crit * se).clamp(-1.0, 1.0), (kappa + z_crit * se).clamp(-1.0, 1.0));

    // SE under null kappa = 0 for H0: kappa = 0 test.
    let mut var0 = 0.0_f64;
    for i in 0..k {
        for j in 0..k {
            let term = weight(i, j) - (w_i_bar[i] + w_j_bar[j]);
            var0 += row_p[i] * col_p[j] * term * term;
        }
    }
    let se0 = ((var0 - pe * pe).max(0.0) / (nf * denom_sq)).sqrt();
    let z_stat = if se0 > 0.0 { kappa / se0 } else { 0.0 };
    let p_val = (2.0 * norm_sf(z_stat.abs())).clamp(0.0, 1.0);

    Ok(KappaResult { kappa, observed_agreement: po, expected_agreement: pe, se, ci, z_statistic: z_stat, p_value: p_val })
}

/// Fleiss' multi-rater kappa across `N` subjects (`rows`) and `K` categories (`cols`), where each
/// row contains the integer counts of `m` raters assigning the subject into each category.
///
/// Fleiss (1971) point estimate, non-null subject-level asymptotic SE and CI (Fleiss, Levin & Paik
/// 2003, ch. 18; Gwet 2008), and the null-hypothesis variance of Fleiss, Nee & Landis (1979)
/// strictly for the *z*-test against $H_0: \kappa = 0$.
pub fn fleiss_kappa(rater_counts: &[Vec<u64>], confidence: ConfidenceLevel) -> Result<KappaResult, StatsError> {
    let n = rater_counts.len();
    let k = rater_counts.first().map_or(0, Vec::len);
    if n < 2 || k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fleiss kappa", required: 2, actual: n.min(k) });
    }
    let m = checked_sum_u64(&rater_counts[0], "fleiss_raters_per_subject")?;
    if m < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Fleiss raters per subject", required: 2, actual: m as usize });
    }
    for row in rater_counts {
        if row.len() != k || checked_sum_u64(row, "fleiss_raters_per_subject")? != m {
            return Err(StatsError::InvalidParameter { name: "raters_per_subject", value: m as f64 });
        }
    }

    let nf = n as f64;
    let mf = m as f64;
    let mut col_sums = vec![0.0_f64; k];
    let mut p_i_vals = Vec::with_capacity(n);
    let mut p_bar_sum = NeumaierSum::new();
    for row in rater_counts {
        let mut sq_sum = 0.0_f64;
        for (j, &c) in row.iter().enumerate() {
            let cf = c as f64;
            col_sums[j] += cf;
            sq_sum += cf * cf;
        }
        let p_i = (sq_sum - mf) / (mf * (mf - 1.0));
        p_i_vals.push(p_i);
        p_bar_sum.add(p_i);
    }
    let p_bar = p_bar_sum.total() / nf;

    let total_assignments = nf * mf;
    let p_j: Vec<f64> = col_sums.iter().map(|&s| s / total_assignments).collect();
    let pe: f64 = p_j.iter().map(|&pj| pj * pj).sum();
    if (1.0 - pe).abs() <= 1e-12 {
        return Err(StatsError::ZeroVariance { context: "Fleiss' kappa (expected agreement p_e = 1)" });
    }
    let kappa = (p_bar - pe) / (1.0 - pe);

    // Non-null subject-level asymptotic SE (Fleiss, Levin & Paik 2003, ch. 18; Gwet 2008) for displayed SE & CI.
    let v_bar = p_bar - 2.0 * pe * (1.0 - kappa);
    let mut ss_v = NeumaierSum::new();
    for (row, &p_i) in rater_counts.iter().zip(p_i_vals.iter()) {
        let mut p_bar_i = 0.0_f64;
        for (&c, &pj) in row.iter().zip(p_j.iter()) {
            p_bar_i += pj * ((c as f64) / mf);
        }
        let v_i = p_i - 2.0 * (1.0 - kappa) * p_bar_i;
        let dv = v_i - v_bar;
        ss_v.add(dv * dv);
    }
    let one_minus_pe_sq = (1.0 - pe).powi(2);
    let var_non_null = ss_v.total() / (nf * (nf - 1.0) * one_minus_pe_sq);
    let se = var_non_null.max(0.0).sqrt();
    let z_crit = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let ci = ((kappa - z_crit * se).clamp(-1.0, 1.0), (kappa + z_crit * se).clamp(-1.0, 1.0));

    // Fleiss, Nee & Landis (1979) asymptotic variance under H0: kappa = 0 strictly for z-test.
    let sum_pj3: f64 = p_j.iter().map(|&pj| pj * pj * pj).sum();
    let denom0 = nf * mf * (mf - 1.0) * one_minus_pe_sq;
    let var0 = if denom0 > 0.0 { 2.0 * (pe - (2.0 * mf - 3.0) * pe * pe + 2.0 * (mf - 2.0) * sum_pj3) / denom0 } else { 0.0 };
    let se0 = var0.max(0.0).sqrt();
    let z_stat = if se0 > 0.0 { kappa / se0 } else { 0.0 };
    let p_val = 2.0 * norm_sf(z_stat.abs());

    Ok(KappaResult { kappa, observed_agreement: p_bar, expected_agreement: pe, se, ci, z_statistic: z_stat, p_value: p_val.clamp(0.0, 1.0) })
}
