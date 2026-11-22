//! Asymptotic covariance and Wald intervals, studentized residuals, Poisson log-likelihood and
//! information criteria, replicate lack-of-fit tests, ROUT outlier detection, and residual diagnostics.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123.

use curvebook_doc::{CiBound, FitMethod};
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::special::{deviance_bd0, ln_gamma};
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::{
    AiccComparisonResult, CovarianceDiagnostics, HomoscedasticityResult, InformationCriteria, LackOfFitResult, NormalityTestResult,
    RoutDetectionResult, RunsTestResult, anderson_darling_test, combine_runs_tests, dagostino_pearson_test, homoscedasticity_spearman_test,
    lilliefors_ks_test, rout_detect_outliers_core, shapiro_wilk_test,
};
use crate::error::FitError;
use crate::solver::irls::compute_unscaled_cov;
use crate::solver::layout::ObsRecord;
use crate::solver::lm::{mat_get, mat_set};
use crate::solver::results::{FittedDatasetResult, SolverConfig};
use crate::types::DataPoint;

pub(in crate::solver) fn obs_y_ss(obs: &[ObsRecord]) -> f64 {
    let mut acc = NeumaierSum::new();
    for o in obs {
        acc.add(o.sqrt_weight * o.sqrt_weight * o.point.y * o.point.y);
    }
    acc.total().max(f64::MIN_POSITIVE)
}

/// Symmetric Wald intervals `theta_j +/- t_{df, alpha/2} * SE_j` from the linearised covariance
/// (Bates & Watts 1988; Seber & Wild 1989), reported as open when the fit is ambiguous.
pub(in crate::solver) fn wald_ci_bounds(cov: &CovarianceDiagnostics, df: usize) -> Vec<(CiBound, CiBound)> {
    cov.standard_errors
        .iter()
        .zip(cov.symmetric_ci_lower.iter().zip(cov.symmetric_ci_upper.iter()))
        .map(|(&se, (&lo, &hi))| {
            if !cov.ambiguous && df > 0 && se.is_finite() && lo.is_finite() && hi.is_finite() {
                (CiBound::Finite(lo), CiBound::Finite(hi))
            } else {
                (CiBound::Open, CiBound::Open)
            }
        })
        .collect()
}

pub(in crate::solver) fn unscaled_cov_from_diag_or_j(
    cov: &CovarianceDiagnostics,
    df: usize,
    weighted_ss: f64,
    j_mat: &DenseMatrix,
) -> Option<DenseMatrix> {
    let k = cov.covariance_matrix.len();
    if cov.ambiguous || k == 0 || j_mat.ncols() == 0 {
        return None;
    }
    let sigma2 = if df > 0 { weighted_ss / (df as f64) } else { 0.0 };
    if sigma2 > 0.0 && sigma2.is_finite() {
        let mut mat = DenseMatrix::zeros(k, k).ok()?;
        let inv_s2 = 1.0 / sigma2;
        for (i, row) in cov.covariance_matrix.iter().enumerate().take(k) {
            for (j, &v) in row.iter().enumerate().take(k) {
                mat_set(&mut mat, i, j, v * inv_s2);
            }
        }
        return Some(mat);
    }
    compute_unscaled_cov(j_mat)
}

/// Internally studentised residuals `e_i / (s * sqrt(1 - h_ii))` with tangent-plane leverage
/// `h_ii = J_i (J^T J)^{-1} J_i^T` from the weighted Jacobian (Cook & Weisberg 1982, applied to
/// the linearised model). Leverage is set to zero (plain standardised residuals) when the fit
/// is ambiguous or `K > 64`.
///
/// Reference: Cook, R. D. & Weisberg, S. (1982). *Residuals and Influence in Regression*. Chapman & Hall.
pub(in crate::solver) fn compute_studentized_residuals(
    j_mat: &DenseMatrix,
    w_res: &[f64],
    cov_unscaled: Option<&DenseMatrix>,
    sigma_w: f64,
    ambiguous: bool,
) -> Vec<f64> {
    let n = w_res.len();
    if !sigma_w.is_finite() || sigma_w <= 0.0 {
        return vec![0.0; n];
    }
    let k = j_mat.ncols();
    let use_leverage = !ambiguous && cov_unscaled.is_some() && k > 0 && k <= 64 && j_mat.nrows() == n;
    let mut out = Vec::with_capacity(n);
    for (i, &wr) in w_res.iter().enumerate() {
        let h_ii = if use_leverage && let Some(cov_u) = cov_unscaled {
            let mut h = 0.0_f64;
            for a in 0..k {
                let ja = mat_get(j_mat, i, a);
                for b in 0..k {
                    h += ja * mat_get(cov_u, a, b) * mat_get(j_mat, i, b);
                }
            }
            h.clamp(0.0, 1.0 - 1e-12)
        } else {
            0.0
        };
        out.push(wr / (sigma_w * (1.0 - h_ii).sqrt()));
    }
    out
}

pub(in crate::solver) fn canonical_x_bits(x: &[f64]) -> Vec<u64> {
    x.iter().map(|&v| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() }).collect()
}

pub(in crate::solver) fn poisson_log_likelihood_from_obs(obs: &[ObsRecord], y_hat: &[f64]) -> Option<f64> {
    if obs.len() != y_hat.len() || obs.is_empty() {
        return None;
    }
    let mut sum = NeumaierSum::new();
    for (rec, &mu) in obs.iter().zip(y_hat.iter()) {
        let y = rec.point.y;
        if !y.is_finite() || y < 0.0 || !mu.is_finite() || mu <= 0.0 {
            return None;
        }
        let lg = ln_gamma(y + 1.0).ok()?;
        let y_ln_mu = if y > 0.0 { y * mu.ln() } else { 0.0 };
        sum.add(y_ln_mu - mu - lg);
    }
    let ll = sum.total();
    ll.is_finite().then_some(ll)
}

pub(in crate::solver) fn information_criteria_from_poisson_ll(ll: f64, n_obs: usize, k_free: usize) -> Option<InformationCriteria> {
    // Poisson likelihood fixes dispersion phi = 1, so effective K = k_free (no +1 variance parameter).
    if !ll.is_finite() || n_obs <= k_free + 1 {
        return None;
    }
    let k = k_free as f64;
    let n = n_obs as f64;
    let neg2_ll = -2.0 * ll;
    let aic = neg2_ll + 2.0 * k;
    let aicc = aic + (2.0 * k * (k + 1.0)) / (n - k - 1.0);
    let bic = neg2_ll + k * n.ln();
    Some(InformationCriteria { aicc, bic, n_obs, k_params_plus_sigma: k_free })
}

pub(in crate::solver) fn poisson_information_criteria(obs: &[ObsRecord], y_hat: &[f64], k_free: usize) -> Option<InformationCriteria> {
    poisson_log_likelihood_from_obs(obs, y_hat).and_then(|ll| information_criteria_from_poisson_ll(ll, obs.len(), k_free))
}

pub(in crate::solver) fn compare_poisson_aicc(aicc_a: f64, aicc_b: f64) -> Result<AiccComparisonResult, FitError> {
    if !aicc_a.is_finite() || !aicc_b.is_finite() {
        return Err(FitError::InvalidParameter { name: "comparison", reason: "Poisson AICc values must be finite for model comparison".to_owned() });
    }
    let delta_aicc = aicc_b - aicc_a;
    let min_aicc = aicc_a.min(aicc_b);
    let exp_a = (-0.5 * (aicc_a - min_aicc)).exp();
    let exp_b = (-0.5 * (aicc_b - min_aicc)).exp();
    let denom = exp_a + exp_b;
    let weight_a = exp_a / denom;
    let weight_b = exp_b / denom;
    let preferred_index = usize::from(aicc_b < aicc_a);
    let evidence_ratio = (0.5 * delta_aicc.abs()).exp();
    Ok(AiccComparisonResult { aicc_a, aicc_b, delta_aicc, weight_a, weight_b, evidence_ratio, preferred_index })
}

pub(in crate::solver) fn group_pure_error_ss_ls(obs: &[ObsRecord], idxs: &[usize]) -> Option<f64> {
    let mut sum_within = 0.0_f64;
    let mut sum_w = 0.0_f64;
    let mut sum_wy = 0.0_f64;
    for &i in idxs {
        let w = obs[i].sqrt_weight * obs[i].sqrt_weight;
        let y = obs[i].point.y;
        if !w.is_finite() || w < 0.0 || !y.is_finite() || !obs[i].ss_within.is_finite() || obs[i].ss_within < 0.0 {
            return None;
        }
        sum_within += obs[i].ss_within;
        sum_w += w;
        sum_wy += w * y;
    }
    if idxs.len() <= 1 {
        return Some(sum_within);
    }
    if !sum_w.is_finite() || sum_w <= 0.0 || !sum_wy.is_finite() {
        return None;
    }
    let y_bar_w = sum_wy / sum_w;
    let mut ss_between = 0.0_f64;
    for &i in idxs {
        let w = obs[i].sqrt_weight * obs[i].sqrt_weight;
        let diff = obs[i].point.y - y_bar_w;
        ss_between += w * diff * diff;
    }
    let total = sum_within + ss_between;
    total.is_finite().then_some(total)
}

pub(in crate::solver) fn group_pure_error_deviance_poisson(obs: &[ObsRecord], idxs: &[usize]) -> Option<f64> {
    if idxs.len() <= 1 {
        return Some(0.0);
    }
    let mut sum_y = 0.0_f64;
    for &i in idxs {
        let y = obs[i].point.y;
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        sum_y += y;
    }
    let y_bar = sum_y / (idxs.len() as f64);
    if !y_bar.is_finite() || y_bar <= 0.0 {
        return (sum_y == 0.0).then_some(0.0);
    }
    let mut acc = NeumaierSum::new();
    for &i in idxs {
        acc.add(2.0 * deviance_bd0(obs[i].point.y, y_bar));
    }
    let dev = acc.total();
    dev.is_finite().then_some(dev)
}

/// Observation-consistent replicates lack-of-fit test honoring replicate summary moments
/// (`sqrt(n)`, `(n-1)*sd^2`), WLS weights (`1/Y^2`, `1/SD^2`, etc.), and Poisson deviance.
pub(in crate::solver) fn replicates_lack_of_fit_obs(obs: &[ObsRecord], weighted_ss: f64, df: usize, method: FitMethod) -> Option<LackOfFitResult> {
    if obs.is_empty() || !weighted_ss.is_finite() || weighted_ss < 0.0 || df == 0 {
        return None;
    }
    let mut groups: std::collections::BTreeMap<(usize, Vec<u64>), Vec<usize>> = std::collections::BTreeMap::new();
    let mut n_total: usize = 0;
    for (idx, rec) in obs.iter().enumerate() {
        if rec.point.x.iter().any(|x| !x.is_finite()) {
            return None;
        }
        n_total = n_total.saturating_add(rec.rep_n);
        groups.entry((rec.ds_local_idx, canonical_x_bits(&rec.point.x))).or_default().push(idx);
    }
    let n_groups = groups.len();
    let df_pure_error = n_total.saturating_sub(n_groups);
    if df_pure_error == 0 {
        return None;
    }
    let df_lack_of_fit = df.saturating_sub(df_pure_error);
    if df_lack_of_fit == 0 {
        return None;
    }

    let mut pe_acc = NeumaierSum::new();
    for idxs in groups.values() {
        let g_pe =
            if method == FitMethod::PoissonDeviance { group_pure_error_deviance_poisson(obs, idxs)? } else { group_pure_error_ss_ls(obs, idxs)? };
        pe_acc.add(g_pe);
    }
    let ss_pure_error = pe_acc.total();
    if !ss_pure_error.is_finite() {
        return None;
    }
    let ss_lack_of_fit = (weighted_ss - ss_pure_error).max(0.0);
    let ms_pe = ss_pure_error / (df_pure_error as f64);
    let ms_lof = ss_lack_of_fit / (df_lack_of_fit as f64);
    if !ms_pe.is_finite() || !ms_lof.is_finite() || ms_pe <= 0.0 {
        return None;
    }
    let f_stat = ms_lof / ms_pe;
    let p_value = f_sf(f_stat, df_lack_of_fit as f64, df_pure_error as f64).ok()?;
    Some(LackOfFitResult { ss_pure_error, df_pure_error, ss_lack_of_fit, df_lack_of_fit, f_stat, p_value, significant_lack_of_fit: p_value < 0.05 })
}

/// ROUT outlier detection per Motulsky & Brown (2006, *BMC Bioinformatics* 7:123): computes RSDR
/// on the base-weighted residuals `r_{w,i} = (y_i - \hat{y}_i) * sqrt(w_i)` so heteroscedastic
/// schemes (such as `1/Y^2`) are standardized to uniform error scale, evaluates two-sided *t*-test
/// *p*-values vs. RSDR, and applies Benjamini–Hochberg step-up FDR **only** across the top 30% of
/// points furthest from the fitted curve (`start = floor(0.70 * N)`). Bottom ~70% residuals are
/// left untested (`q_value = 1.0`, `is_outlier = false`).
pub(in crate::solver) fn rout_detect_outliers_mb2006(
    points: &[DataPoint],
    weighted_residuals: &[f64],
    sqrt_weights: &[f64],
    k_free: usize,
    fdr_q_percent: f64,
) -> Result<RoutDetectionResult, FitError> {
    rout_detect_outliers_core(points, weighted_residuals, sqrt_weights, k_free, fdr_q_percent, "weighted_residuals", "fdr_q_percent")
}

pub(in crate::solver) struct ResidualDiagnosticsBundle {
    pub(in crate::solver) runs_test: Option<RunsTestResult>,
    pub(in crate::solver) lack_of_fit: Option<LackOfFitResult>,
    pub(in crate::solver) normality: Option<NormalityTestResult>,
    pub(in crate::solver) normality_anderson_darling: Option<NormalityTestResult>,
    pub(in crate::solver) normality_lilliefors_ks: Option<NormalityTestResult>,
    pub(in crate::solver) homoscedasticity: Option<HomoscedasticityResult>,
}

pub(in crate::solver) fn compute_residual_diagnostics(
    obs: &[ObsRecord],
    ds_results: &[FittedDatasetResult],
    y_hat_all: &[f64],
    raw_residuals_all: &[f64],
    weighted_ss: f64,
    df: usize,
    config: &SolverConfig,
) -> ResidualDiagnosticsBundle {
    let runs_res = if config.compute_runs_test {
        let per_ds_runs: Vec<RunsTestResult> = ds_results.iter().filter_map(|d| d.runs_test.clone()).collect();
        combine_runs_tests(&per_ds_runs)
    } else {
        None
    };

    let lof_res = if config.compute_lack_of_fit && config.method != FitMethod::RobustCauchy {
        replicates_lack_of_fit_obs(obs, weighted_ss, df, config.method)
    } else {
        None
    };

    let diag_residuals: Vec<f64> = if config.method == FitMethod::PoissonDeviance {
        obs.iter()
            .zip(y_hat_all.iter())
            .zip(raw_residuals_all.iter())
            .map(|((o, &yh), &r)| if yh.is_finite() && yh > 0.0 { r / yh.sqrt() } else { r * o.sqrt_weight })
            .collect()
    } else {
        obs.iter().zip(raw_residuals_all.iter()).map(|(o, &r)| r * o.sqrt_weight).collect()
    };

    let (norm_main, norm_ad, norm_ks) = if config.compute_normality {
        (
            shapiro_wilk_test(&diag_residuals).or_else(|| dagostino_pearson_test(&diag_residuals)),
            anderson_darling_test(&diag_residuals),
            lilliefors_ks_test(&diag_residuals),
        )
    } else {
        (None, None, None)
    };

    let homo_res =
        if config.compute_homoscedasticity { homoscedasticity_spearman_test(y_hat_all, &diag_residuals, config.confidence.alpha()) } else { None };

    ResidualDiagnosticsBundle {
        runs_test: runs_res,
        lack_of_fit: lof_res,
        normality: norm_main,
        normality_anderson_darling: norm_ad,
        normality_lilliefors_ks: norm_ks,
        homoscedasticity: homo_res,
    }
}
