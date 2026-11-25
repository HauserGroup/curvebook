//! Pairwise correlations (Pearson *r* with Fisher-*z* CI, Spearman *rho*, Kendall *tau_b*) and
//! correlation matrices with Benjamini–Hochberg FDR multiplicity adjustment.
//!
//! References:
//! - Pearson, K. (1896). Mathematical contributions to the theory of evolution. III. Regression,
//!   heredity, and panmixia. *Philosophical Transactions of the Royal Society A*, 187, 253-318.
//!   `doi:10.1098/rsta.1896.0007`.
//! - Fisher, R. A. (1921). On the "probable error" of a coefficient of correlation deduced from a
//!   small sample. *Metron*, 1, 3-32.
//! - Spearman, C. (1904). The proof and measurement of association between two things. *American
//!   Journal of Psychology*, 15(1), 72-101. `doi:10.2307/1412159`.
//! - Kendall, M. G. (1938). A new measure of rank correlation. *Biometrika*, 30(1-2), 81-93.
//!   `doi:10.1093/biomet/30.1-2.81`.
//! - Kendall, M. G. (1945). The treatment of ties in ranking problems. *Biometrika*, 33(3), 239-251.
//!   `doi:10.1093/biomet/33.3.239`.
//! - Benjamini, Y., & Hochberg, Y. (1995). Controlling the false discovery rate: A practical and
//!   powerful approach to multiple testing. *Journal of the Royal Statistical Society B*, 57(1),
//!   289-300.

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_two_sided_isf, norm_two_sided_p, student_t_two_sided_p};
use curvebook_math::sum::pearson_r;

use super::types::{CorrelationMatrixResult, CorrelationPairResult};
use crate::common::{rank_with_ties, validate_finite_slice};
use crate::error::StatsError;
use crate::multiplicity::adjust_benjamini_hochberg;

/// Pearson product-moment correlation with Fisher-*z* confidence interval and exact *t*-test *p*.
///
/// Pearson (1896) *r*; *t* = `r sqrt((n - 2) / (1 - r^2))` on `n - 2` df; CI from Fisher's (1921)
/// `z = atanh(r)` with standard error `1 / sqrt(n - 3)`.
pub fn pearson_correlation(x: &[f64], y: &[f64], confidence: ConfidenceLevel) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: n });
    }
    let r = pearson_r(x, y)?;
    let df = (n - 2) as f64;
    let (t_stat, p_val) = if (1.0 - r * r).abs() <= 1e-15 {
        (if r >= 0.0 { f64::INFINITY } else { f64::NEG_INFINITY }, 0.0)
    } else {
        let t = r * (df / (1.0 - r * r)).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };

    let ci = if n >= 4 && r.abs() < 1.0 - 1e-12 {
        let z_r = r.atanh();
        let se_z = 1.0 / ((n - 3) as f64).sqrt();
        let z_crit = norm_two_sided_isf(confidence.alpha())?;
        Some(((z_r - z_crit * se_z).tanh(), (z_r + z_crit * se_z).tanh()))
    } else {
        None
    };

    Ok(CorrelationPairResult { r, r_squared: r * r, ci, statistic: t_stat, df: Some(df), p_value: p_val, n })
}

/// Spearman rank-order correlation (`O(n log n)` with exact average tie handling).
///
/// Spearman (1904) *rho* computed as the Pearson correlation of mid-ranks, with the *t*
/// approximation on `n - 2` df for the two-sided *P* (no exact permutation *P*).
pub fn spearman_correlation(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Spearman correlation", required: 3, actual: n });
    }
    let rx = rank_with_ties(x)?.ranks;
    let ry = rank_with_ties(y)?.ranks;
    let rho = pearson_r(&rx, &ry)?;
    let df = (n - 2) as f64;
    let (t_stat, p_val) = if (1.0 - rho * rho).abs() <= 1e-15 {
        (if rho >= 0.0 { f64::INFINITY } else { f64::NEG_INFINITY }, 0.0)
    } else {
        let t = rho * (df / (1.0 - rho * rho)).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };
    Ok(CorrelationPairResult { r: rho, r_squared: rho * rho, ci: None, statistic: t_stat, df: Some(df), p_value: p_val, n })
}

/// Kendall's *tau_b* rank correlation with tie-adjusted asymptotic normal *p*-value.
///
/// Kendall (1938) *tau*, with the *tau_b* tie adjustment of Kendall (1945).
pub fn kendall_tau_b(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    validate_finite_slice(x)?;
    validate_finite_slice(y)?;
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Kendall tau-b correlation", required: 3, actual: n });
    }
    let mut conc = 0.0_f64;
    let mut disc = 0.0_f64;
    let mut ties_x = 0.0_f64;
    let mut ties_y = 0.0_f64;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[i] - x[j];
            let dy = y[i] - y[j];
            if dx == 0.0 && dy == 0.0 {
                ties_x += 1.0;
                ties_y += 1.0;
            } else if dx == 0.0 {
                ties_x += 1.0;
            } else if dy == 0.0 {
                ties_y += 1.0;
            } else if dx * dy > 0.0 {
                conc += 1.0;
            } else {
                disc += 1.0;
            }
        }
    }
    let nf = n as f64;
    let n0 = 0.5 * nf * (nf - 1.0);
    let denom = ((n0 - ties_x) * (n0 - ties_y)).sqrt();
    let tau = if denom > 0.0 { ((conc - disc) / denom).clamp(-1.0, 1.0) } else { 0.0 };

    // Tie-corrected variance for S = C - D (Kendall 1970).
    let rx = rank_with_ties(x)?;
    let ry = rank_with_ties(y)?;
    let sum_t = |counts: &[usize]| -> (f64, f64, f64) {
        let mut v = 0.0;
        let mut s2 = 0.0;
        let mut s1 = 0.0;
        for &c in counts {
            let cf = c as f64;
            v += cf * (cf - 1.0) * (2.0 * cf + 5.0);
            s2 += cf * (cf - 1.0) * (cf - 2.0);
            s1 += cf * (cf - 1.0);
        }
        (v, s2, s1)
    };
    let (vt, t2, t1) = sum_t(&rx.tie_counts);
    let (vu, u2, u1) = sum_t(&ry.tie_counts);
    let v0 = nf * (nf - 1.0) * (2.0 * nf + 5.0);
    let term1 = (v0 - vt - vu) / 18.0;
    let term2 = (t2 * u2) / (9.0 * nf * (nf - 1.0) * (nf - 2.0));
    let term3 = (t1 * u1) / (2.0 * nf * (nf - 1.0));
    let var_s = (term1 + term2 + term3).max(1e-12);
    let z = (conc - disc) / var_s.sqrt();
    let p_val = norm_two_sided_p(z);

    Ok(CorrelationPairResult { r: tau, r_squared: tau * tau, ci: None, statistic: z, df: None, p_value: p_val, n })
}

/// Pairwise correlation matrix across columns (using pairwise complete finite observations)
/// with Benjamini–Hochberg FDR *q*-values across the upper triangle.
pub fn correlation_matrix(columns: &[(&str, &[Option<f64>])], spearman: bool) -> Result<CorrelationMatrixResult, StatsError> {
    let p = columns.len();
    if p < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "correlation matrix variables", required: 2, actual: p });
    }
    let mut r_mat = vec![vec![1.0_f64; p]; p];
    let mut p_mat = vec![vec![0.0_f64; p]; p];
    let mut q_mat = vec![vec![0.0_f64; p]; p];
    let mut n_mat = vec![vec![0_usize; p]; p];

    let mut upper_pairs = Vec::new();
    let mut raw_p = Vec::new();

    for i in 0..p {
        n_mat[i][i] = columns[i].1.iter().filter(|v| v.is_some_and(f64::is_finite)).count();
        for j in (i + 1)..p {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (&a, &b) in columns[i].1.iter().zip(columns[j].1.iter()) {
                if let (Some(av), Some(bv)) = (a, b)
                    && av.is_finite()
                    && bv.is_finite()
                {
                    xs.push(av);
                    ys.push(bv);
                }
            }
            n_mat[i][j] = xs.len();
            n_mat[j][i] = xs.len();
            let res = if xs.len() >= 3 {
                if spearman { spearman_correlation(&xs, &ys) } else { pearson_correlation(&xs, &ys, ConfidenceLevel::DEFAULT_95) }
            } else {
                Err(StatsError::InsufficientSampleSize { context: "pairwise correlation", required: 3, actual: xs.len() })
            };
            let (r_val, p_val) = match res {
                Ok(c) => (c.r, c.p_value),
                Err(_) => (f64::NAN, f64::NAN),
            };
            r_mat[i][j] = r_val;
            r_mat[j][i] = r_val;
            p_mat[i][j] = p_val;
            p_mat[j][i] = p_val;
            upper_pairs.push((i, j));
            raw_p.push(p_val);
        }
    }

    let q_vals = adjust_benjamini_hochberg(&raw_p);
    for (&(i, j), q) in upper_pairs.iter().zip(q_vals) {
        q_mat[i][j] = q;
        q_mat[j][i] = q;
    }

    Ok(CorrelationMatrixResult {
        labels: columns.iter().map(|(l, _)| (*l).to_owned()).collect(),
        r_matrix: r_mat,
        p_matrix: p_mat,
        fdr_q_matrix: q_mat,
        n_matrix: n_mat,
        spearman,
    })
}
