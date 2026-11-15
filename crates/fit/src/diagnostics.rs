//! Uncertainty quantification, multicollinearity & Hougaard nonlinearity diagnostics,
//! Venzon–Moolgavkar profile-likelihood confidence intervals, model comparison (`Extra-SS F`,
//! `AICc`, `BIC`), ROUT outlier detection, and residual assumption checks.
//!
//! # References
//!
//! Short `Author (Year)` citations on individual items refer to the list below.
//!
//! Reference: Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//!
//! Reference: Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//!
//! Reference: Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//!
//! Reference: Draper, N. R. & Smith, H. (1998). *Applied Regression Analysis*, 3rd ed. Wiley.
//!
//! Reference: Hougaard, P. (1985). The appropriateness of the asymptotic distribution in a nonlinear regression model in relation to curvature. *Journal of the Royal Statistical Society, Series B* 47, 103–114.
//!
//! Reference: Ratkowsky, D. A. (1990). *Handbook of Nonlinear Regression Models*. Marcel Dekker.
//!
//! Reference: Venzon, D. J. & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based confidence intervals. *Journal of the Royal Statistical Society, Series C (Applied Statistics)* 37(1), 87–94. doi:10.2307/2347496
//!
//! Reference: Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.
//!
//! Reference: Wilks, S. S. (1938). The large-sample distribution of the likelihood ratio for testing composite hypotheses. *Annals of Mathematical Statistics* 9(1), 60–62. doi:10.1214/aoms/1177732360
//!
//! Reference: McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//!
//! Reference: Akaike, H. (1974). A new look at the statistical model identification. *IEEE Transactions on Automatic Control* 19(6), 716–723. doi:10.1109/TAC.1974.1100705
//!
//! Reference: Hurvich, C. M. & Tsai, C.-L. (1989). Regression and time series model selection in small samples. *Biometrika* 76(2), 297–307. doi:10.1093/biomet/76.2.297
//!
//! Reference: Schwarz, G. (1978). Estimating the dimension of a model. *Annals of Statistics* 6(2), 461–464. doi:10.1214/aos/1176344136
//!
//! Reference: Burnham, K. P. & Anderson, D. R. (2002). *Model Selection and Multimodel Inference: A Practical Information-Theoretic Approach*, 2nd ed. Springer.
//!
//! Reference: Wald, A. & Wolfowitz, J. (1940). On a test whether two samples are from the same population. *Annals of Mathematical Statistics* 11(2), 147–162.
//!
//! Reference: Swed, F. S. & Eisenhart, C. (1943). Tables for testing randomness of grouping in a sequence of alternatives. *Annals of Mathematical Statistics* 14.
//!
//! Reference: Shapiro, S. S. & Wilk, M. B. (1965). An analysis of variance test for normality (complete samples). *Biometrika* 52(3/4), 591–611.
//!
//! Reference: Royston, P. (1995). Remark AS R94: A remark on algorithm AS 181: The W-test for normality. *Journal of the Royal Statistical Society, Series C (Applied Statistics)* 44(4), 547–551.
//!
//! Reference: D'Agostino, R. B., Belanger, A. & D'Agostino, R. B., Jr. (1990). A suggestion for using powerful and informative tests of normality. *The American Statistician* 44(4), 316–321.
//!
//! Reference: Anscombe, F. J. & Glynn, W. J. (1983). Distribution of the kurtosis statistic b2 for normal samples. *Biometrika* 70(1), 227–234.
//!
//! Reference: Anderson, T. W. & Darling, D. A. (1954). A test of goodness of fit. *Journal of the American Statistical Association* 49(268), 765–769.
//!
//! Reference: D'Agostino, R. B. & Stephens, M. A. (eds.) (1986). *Goodness-of-Fit Techniques*. Marcel Dekker.
//!
//! Reference: Lilliefors, H. W. (1967). On the Kolmogorov–Smirnov test for normality with mean and variance unknown. *Journal of the American Statistical Association* 62(318), 399–402.
//!
//! Reference: Dallal, G. E. & Wilkinson, L. (1986). An analytic approximation to the distribution of Lilliefors's test statistic for normality. *The American Statistician* 40(4), 294–296. doi:10.2307/2684607
//!
//! Reference: Zar, J. H. (2010). *Biostatistical Analysis*, 5th ed. Pearson Prentice Hall.
//!
//! Reference: Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
//!
//! Reference: Benjamini, Y. & Hochberg, Y. (1995). Controlling the false discovery rate: a practical and powerful approach to multiple testing. *Journal of the Royal Statistical Society, Series B* 57(1), 289–300.

use std::collections::BTreeMap;

use lumencraft_doc::CiBound;
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, f_sf, norm_cdf, norm_ppf, norm_sf, student_t_two_sided_isf, student_t_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, parameter_dependencies, qr_col_pivoted, svd};
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::ln_choose;
use lumencraft_math::sum::{NeumaierSum, SummaryMoments, pearson_r};

use crate::error::FitError;
use crate::types::DataPoint;

/// Asymptotic covariance, standard errors, symmetric CIs, correlation matrix, and parameter dependency.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CovarianceDiagnostics {
    pub standard_errors: Vec<f64>,
    pub symmetric_ci_lower: Vec<f64>,
    pub symmetric_ci_upper: Vec<f64>,
    pub covariance_matrix: Vec<Vec<f64>>,
    pub correlation_matrix: Vec<Vec<f64>>,
    pub dependencies: Vec<f64>,
    pub numerical_rank: usize,
    pub ambiguous: bool,
}

/// Model comparison outcome via Extra-Sum-of-Squares *F*-test.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtraSumOfSquaresResult {
    pub f_stat: f64,
    pub df_num: usize,
    pub df_den: usize,
    pub p_value: f64,
    pub preferred_complex_model: bool,
}

/// Corrected Akaike Information Criterion ($\text{AIC}_c$) and Bayesian Information Criterion ($\text{BIC}$) summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InformationCriteria {
    pub aicc: f64,
    pub bic: f64,
    pub n_obs: usize,
    pub k_params_plus_sigma: usize,
}

/// Pairwise $\text{AIC}_c$ comparison between two models evaluated on identical observations.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiccComparisonResult {
    pub aicc_a: f64,
    pub aicc_b: f64,
    pub delta_aicc: f64,
    pub weight_a: f64,
    pub weight_b: f64,
    pub evidence_ratio: f64,
    pub preferred_index: usize,
}

/// Runs test for systematic deviation from a fitted curve.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunsTestResult {
    pub runs: usize,
    pub n_above: usize,
    pub n_below: usize,
    pub expected_runs: f64,
    pub p_too_few_runs: f64,
    pub p_two_sided: f64,
    pub deviates_significantly: bool,
}

/// Replicates test for lack of fit (pure error vs lack of fit).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LackOfFitResult {
    pub ss_pure_error: f64,
    pub df_pure_error: usize,
    pub ss_lack_of_fit: f64,
    pub df_lack_of_fit: usize,
    pub f_stat: f64,
    pub p_value: f64,
    pub significant_lack_of_fit: bool,
}

/// Normality test result on regression residuals.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalityTestResult {
    pub test_name: String,
    pub statistic: f64,
    pub p_value: f64,
    pub passed: bool,
}

/// Model comparison outcome via Likelihood Ratio Test (`D = 2(ln L_alt - ln L_null)` chi-square test).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LikelihoodRatioResult {
    pub lr_statistic: f64,
    pub df_diff: usize,
    pub df_null: usize,
    pub df_alt: usize,
    pub p_value: f64,
    pub preferred_complex_model: bool,
}

/// Residual homoscedasticity check via Spearman rank correlation between `|residual|` and predicted `Y`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HomoscedasticityResult {
    pub spearman_rho: f64,
    pub statistic_t: f64,
    pub df: usize,
    pub p_value: f64,
    pub homoscedastic: bool,
}

/// Per-point ROUT outlier audit entry.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoutPointAudit {
    pub index: usize,
    pub data_set_index: usize,
    pub row: usize,
    pub x0: f64,
    pub y: f64,
    pub residual: f64,
    pub p_value: f64,
    pub q_value: f64,
    pub is_outlier: bool,
}

/// Result of nonlinear ROUT outlier detection (`Motulsky & Brown 2006`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoutDetectionResult {
    pub rsdr: f64,
    pub df: usize,
    pub q_threshold_percent: f64,
    pub outlier_indices: Vec<usize>,
    pub audit: Vec<RoutPointAudit>,
}

/// Compute asymptotic covariance, standard errors, symmetric `t`-CIs, correlation matrix, and
/// parameter dependencies from the weighted Jacobian `j_mat` (`N x p_free`) at convergence.
///
/// Linearised (asymptotic) covariance `Cov = s^2 (J^T J)^{-1}`, `s^2 = SS / df`, with symmetric
/// Wald intervals `theta_j +/- t_{df, alpha/2} SE_j` (Bates & Watts 1988; Seber & Wild 1989).
/// `(J^T J)^{-1}` comes from a column-pivoted QR of `J` (no explicit normal equations); when `J`
/// is rank-deficient an SVD pseudo-inverse is used and the fit is flagged ambiguous.
/// Dependency is `1 - 1 / ((J^T J)_{jj} [(J^T J)^{-1}]_{jj})`, i.e. the uncentred `R^2` of
/// column `j` on the others, as described by Motulsky & Christopoulos (2004); a dependency
/// above `0.9999` marks the fit ambiguous.
///
/// # Errors
/// Returns [`FitError::Math`] if matrix allocation or linear algebra fails.
pub fn compute_covariance_diagnostics(
    j_mat: &DenseMatrix,
    rss: f64,
    df: usize,
    theta: &[f64],
    confidence: ConfidenceLevel,
) -> Result<CovarianceDiagnostics, FitError> {
    let p = theta.len();
    if p == 0 {
        return Ok(CovarianceDiagnostics {
            standard_errors: Vec::new(),
            symmetric_ci_lower: Vec::new(),
            symmetric_ci_upper: Vec::new(),
            covariance_matrix: Vec::new(),
            correlation_matrix: Vec::new(),
            dependencies: Vec::new(),
            numerical_rank: 0,
            ambiguous: false,
        });
    }

    let pqr = qr_col_pivoted(j_mat)?;
    let rank = pqr.rank();
    let sigma2 = if df > 0 { (rss / (df as f64)).max(0.0) } else { 0.0 };

    let (xtx_inv, deps, mut ambiguous) = if rank == p && j_mat.nrows() >= p {
        let inv = pqr.xtx_inverse()?;
        let deps = parameter_dependencies(j_mat).unwrap_or_else(|_| vec![1.0; p]);
        let amb = deps.iter().any(|&d| !d.is_finite() || d > 0.9999);
        (inv, deps, amb)
    } else {
        let sv = svd(j_mat)?;
        let inv = sv.ata_pseudoinverse(None)?;
        (inv, vec![1.0; p], true)
    };

    let t_crit = if df > 0 { student_t_two_sided_isf(confidence.alpha(), df as f64).unwrap_or(1.96) } else { f64::NAN };
    let mut se = Vec::with_capacity(p);
    let mut ci_lo = Vec::with_capacity(p);
    let mut ci_hi = Vec::with_capacity(p);
    let mut cov_vec = vec![vec![0.0; p]; p];
    let mut corr_vec = vec![vec![0.0; p]; p];

    for i in 0..p {
        let var_i = (sigma2 * xtx_inv.get(i, i).unwrap_or(0.0)).max(0.0);
        let se_i = var_i.sqrt();
        if !se_i.is_finite() {
            ambiguous = true;
        }
        se.push(se_i);
        let half = if t_crit.is_finite() && se_i.is_finite() { t_crit * se_i } else { f64::NAN };
        ci_lo.push(theta[i] - half);
        ci_hi.push(theta[i] + half);
        for (j, cell) in cov_vec[i].iter_mut().enumerate().take(p) {
            *cell = sigma2 * xtx_inv.get(i, j).unwrap_or(0.0);
        }
    }

    for i in 0..p {
        for j in 0..p {
            let denom = se[i] * se[j];
            corr_vec[i][j] = if i == j {
                1.0
            } else if denom > 0.0 && denom.is_finite() {
                (cov_vec[i][j] / denom).clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
    }

    Ok(CovarianceDiagnostics {
        standard_errors: se,
        symmetric_ci_lower: ci_lo,
        symmetric_ci_upper: ci_hi,
        covariance_matrix: cov_vec,
        correlation_matrix: corr_vec,
        dependencies: deps,
        numerical_rank: rank,
        ambiguous,
    })
}

/// Compute Hougaard's skewness measure `g_{1,i}` for each free parameter using second-order
/// finite differences of the model prediction vector `eval_fn(theta) -> Option<Vec<f64>>`.
///
/// Hougaard (1985) skewness in the form given by Ratkowsky (1990):
/// `g_{1,i} = -s * sum_{jkl} L_ij L_ik L_il (W_jkl + W_kjl + W_ljk) / L_ii^{3/2}`, with
/// `L = (J^T J)^{-1}` and `W_jkl = sum_m (d f_m / d theta_j)(d^2 f_m / d theta_k d theta_l)`
/// (weighted). Second derivatives use central differences with step `1e-4 * max(|theta|, 1)`.
pub fn compute_hougaard_skewness<F>(
    theta: &[f64],
    sqrt_weights: &[f64],
    j_mat: &DenseMatrix,
    cov_unscaled: &DenseMatrix,
    sigma: f64,
    mut eval_fn: F,
) -> Vec<f64>
where
    F: FnMut(&[f64]) -> Option<Vec<f64>>,
{
    compute_hougaard_skewness_into(theta, sqrt_weights, j_mat, cov_unscaled, sigma, |th, out| {
        if let Some(vals) = eval_fn(th)
            && vals.len() == out.len()
        {
            out.copy_from_slice(&vals);
            true
        } else {
            false
        }
    })
}

/// Zero-allocation Hougaard skewness helper writing predictions directly into caller buffers.
pub(crate) fn compute_hougaard_skewness_into<F>(
    theta: &[f64],
    sqrt_weights: &[f64],
    j_mat: &DenseMatrix,
    cov_unscaled: &DenseMatrix,
    sigma: f64,
    mut eval_into: F,
) -> Vec<f64>
where
    F: FnMut(&[f64], &mut [f64]) -> bool,
{
    let p = theta.len();
    let n = j_mat.nrows();
    if p == 0 || n == 0 || !sigma.is_finite() || sigma <= 0.0 {
        return vec![0.0; p];
    }
    let mut f0 = vec![0.0_f64; n];
    if !eval_into(theta, &mut f0) {
        return vec![0.0; p];
    }

    let steps: Vec<f64> = theta.iter().map(|&t| 1e-4 * t.abs().max(1.0)).collect();
    let Some(w_flat) = build_hougaard_w_tensor(theta, &f0, &steps, sqrt_weights, j_mat, &mut eval_into) else {
        return vec![0.0; p];
    };

    let mut out = Vec::with_capacity(p);
    for i in 0..p {
        let l_ii = cov_unscaled.get(i, i).unwrap_or(0.0);
        if !l_ii.is_finite() || l_ii <= 0.0 {
            out.push(0.0);
            continue;
        }
        let mut s_acc = NeumaierSum::new();
        for j in 0..p {
            let l_ij = cov_unscaled.get(i, j).unwrap_or(0.0);
            for k in 0..p {
                let l_ik = cov_unscaled.get(i, k).unwrap_or(0.0);
                for l in 0..p {
                    let l_il = cov_unscaled.get(i, l).unwrap_or(0.0);
                    let w_jkl = w_flat[(j * p + k) * p + l];
                    let w_kjl = w_flat[(k * p + j) * p + l];
                    let w_ljk = w_flat[(l * p + j) * p + k];
                    s_acc.add(l_ij * l_ik * l_il * (w_jkl + w_kjl + w_ljk));
                }
            }
        }
        let g1 = -sigma * s_acc.total() / l_ii.powf(1.5);
        out.push(if g1.is_finite() { g1 } else { 0.0 });
    }
    out
}

struct HougaardAccum<'a> {
    p: usize,
    n: usize,
    sqrt_weights: &'a [f64],
    j_mat: &'a DenseMatrix,
    w_flat: &'a mut [f64],
    accs: Vec<NeumaierSum>,
}

impl<'a> HougaardAccum<'a> {
    fn new(p: usize, n: usize, sqrt_weights: &'a [f64], j_mat: &'a DenseMatrix, w_flat: &'a mut [f64]) -> Self {
        Self { p, n, sqrt_weights, j_mat, w_flat, accs: vec![NeumaierSum::new(); p] }
    }

    fn record_diag(&mut self, j: usize, inv_h2: f64, f0: &[f64], fp: &[f64], fm: &[f64]) {
        for acc in &mut self.accs {
            *acc = NeumaierSum::new();
        }
        for m in 0..self.n {
            let d2 = (fp[m] - 2.0 * f0[m] + fm[m]) * inv_h2;
            let wd = self.sqrt_weights[m] * d2;
            for (g, acc) in self.accs.iter_mut().enumerate() {
                acc.add(self.j_mat.get(m, g).unwrap_or(0.0) * wd);
            }
        }
        for (g, acc) in self.accs.iter().enumerate() {
            self.w_flat[(g * self.p + j) * self.p + j] = acc.total();
        }
    }

    fn record_mixed(&mut self, pair: (usize, usize), inv_4hk: f64, samples: [&[f64]; 4]) {
        let (j, k) = pair;
        let [fpp, fpm, fmp, fmm] = samples;
        for acc in &mut self.accs {
            *acc = NeumaierSum::new();
        }
        for m in 0..self.n {
            let d2 = (fpp[m] - fpm[m] - fmp[m] + fmm[m]) * inv_4hk;
            let wd = self.sqrt_weights[m] * d2;
            for (g, acc) in self.accs.iter_mut().enumerate() {
                acc.add(self.j_mat.get(m, g).unwrap_or(0.0) * wd);
            }
        }
        for (g, acc) in self.accs.iter().enumerate() {
            let val = acc.total();
            self.w_flat[(g * self.p + j) * self.p + k] = val;
            self.w_flat[(g * self.p + k) * self.p + j] = val;
        }
    }
}

fn build_hougaard_w_tensor<F>(
    theta: &[f64],
    f0: &[f64],
    steps: &[f64],
    sqrt_weights: &[f64],
    j_mat: &DenseMatrix,
    eval_into: &mut F,
) -> Option<Vec<f64>>
where
    F: FnMut(&[f64], &mut [f64]) -> bool,
{
    let p = theta.len();
    let n = f0.len();
    let mut w_flat = vec![0.0_f64; p * p * p];
    let mut accum = HougaardAccum::new(p, n, sqrt_weights, j_mat, &mut w_flat);

    let mut work = theta.to_vec();
    let mut fp = vec![0.0_f64; n];
    let mut fm = vec![0.0_f64; n];
    let mut fmp = vec![0.0_f64; n];
    let mut fmm = vec![0.0_f64; n];

    for j in 0..p {
        let hj = steps[j];
        work[j] = theta[j] + hj;
        if !eval_into(&work, &mut fp) {
            return None;
        }
        work[j] = theta[j] - hj;
        if !eval_into(&work, &mut fm) {
            return None;
        }
        work[j] = theta[j];
        accum.record_diag(j, 1.0 / (hj * hj), f0, &fp, &fm);

        for k in (j + 1)..p {
            let hk = steps[k];
            work[j] = theta[j] + hj;
            work[k] = theta[k] + hk;
            if !eval_into(&work, &mut fp) {
                return None;
            }
            work[k] = theta[k] - hk;
            if !eval_into(&work, &mut fm) {
                return None;
            }
            work[j] = theta[j] - hj;
            work[k] = theta[k] + hk;
            if !eval_into(&work, &mut fmp) {
                return None;
            }
            work[k] = theta[k] - hk;
            if !eval_into(&work, &mut fmm) {
                return None;
            }
            work[j] = theta[j];
            work[k] = theta[k];
            accum.record_mixed((j, k), 1.0 / (4.0 * hj * hk), [&fp, &fm, &fmp, &fmm]);
        }
    }
    Some(w_flat)
}

/// Trace Venzon–Moolgavkar asymmetrical profile-likelihood confidence intervals for every free
/// parameter, returning [`CiBound::Open`] if the profile sum of squares asymptotes below
/// `SS_crit = SS_min * (1 + F_{1, df}(alpha) / df)`.
///
/// The endpoint criterion is the profile-`t` interval of Bates & Watts (1988):
/// `|tau(theta_k)| <= t_{df, alpha/2}` with `tau^2 = (S~(theta_k) - SS_min) / s^2`, which is
/// exactly `S~(theta_k) <= SS_crit` above (`F_{1,df} = t_{df}^2`). These are the same
/// profile-likelihood endpoints targeted by Venzon & Moolgavkar (1988).
///
/// Deviation: endpoints are not found with Venzon & Moolgavkar's modified Newton–Raphson
/// system. Each side is bracketed by geometric step expansion from the Wald half-width and then
/// refined with Brent (1973) root-finding on `S~(theta_k) - SS_crit`. Nuisance parameters are
/// warm-started by the linear covariance regression `theta_j + (C_jk / C_kk) * delta`.
pub fn compute_profile_likelihood_cis<F>(
    theta_hat: &[f64],
    ss_min: f64,
    df: usize,
    cov_diag: &CovarianceDiagnostics,
    bounds: &[(Option<f64>, Option<f64>)],
    confidence: ConfidenceLevel,
    mut profile_solve: F,
) -> Vec<(CiBound, CiBound)>
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let p = theta_hat.len();
    if p == 0 || df == 0 || !ss_min.is_finite() {
        return vec![(CiBound::Open, CiBound::Open); p];
    }
    if ss_min <= 1e-30 {
        return theta_hat.iter().map(|&t| (CiBound::Finite(t), CiBound::Finite(t))).collect();
    }
    let Ok(t_crit) = student_t_two_sided_isf(confidence.alpha(), df as f64) else {
        return vec![(CiBound::Open, CiBound::Open); p];
    };
    let f_crit = t_crit * t_crit;
    let ss_crit = ss_min * (1.0 + f_crit / (df as f64));
    let mut out = Vec::with_capacity(p);

    for k in 0..p {
        let se_k = cov_diag.standard_errors.get(k).copied().unwrap_or(f64::NAN);
        let base_step = if se_k.is_finite() && se_k > 0.0 { t_crit * se_k } else { 0.1 * theta_hat[k].abs().max(1.0) };
        let (lb, ub) = bounds.get(k).copied().unwrap_or((None, None));
        let lo = trace_profile_endpoint(k, -1.0, theta_hat, ss_min, ss_crit, base_step, lb, &cov_diag.covariance_matrix, &mut profile_solve);
        let hi = trace_profile_endpoint(k, 1.0, theta_hat, ss_min, ss_crit, base_step, ub, &cov_diag.covariance_matrix, &mut profile_solve);
        out.push((lo, hi));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn trace_profile_endpoint<F>(
    k: usize,
    dir: f64,
    theta_hat: &[f64],
    ss_min: f64,
    ss_crit: f64,
    base_step: f64,
    box_limit: Option<f64>,
    cov: &[Vec<f64>],
    profile_solve: &mut F,
) -> CiBound
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let mut prev_val = theta_hat[k];
    let mut prev_diff = ss_min - ss_crit;
    let mut warm_theta = theta_hat.to_vec();
    let cov_kk = cov.get(k).and_then(|r| r.get(k)).copied().unwrap_or(0.0);

    let mut step = base_step.max(1e-8 * theta_hat[k].abs().max(1.0));
    let mut bracket: Option<(f64, f64)> = None;
    let mut last_ss = ss_min;

    for iter in 0..24 {
        let mut cand = theta_hat[k] + dir * step;
        let hit_box = match (dir < 0.0, box_limit) {
            (true, Some(b)) if cand <= b => {
                cand = b;
                true
            }
            (false, Some(b)) if cand >= b => {
                cand = b;
                true
            }
            _ => false,
        };
        if (cand - prev_val).abs() <= 1e-15 * prev_val.abs().max(1.0) && hit_box {
            return CiBound::Open;
        }
        // Linear (covariance-regression) warm start for the nuisance parameters.
        let delta = cand - theta_hat[k];
        if cov_kk > 0.0 {
            for (j, slot) in warm_theta.iter_mut().enumerate() {
                if j != k {
                    let cov_jk = cov.get(j).and_then(|r| r.get(k)).copied().unwrap_or(0.0);
                    *slot = theta_hat[j] + (cov_jk / cov_kk) * delta;
                }
            }
        }
        let Some((ss_cand, best_nuisance)) = profile_solve(k, cand, &warm_theta) else {
            return CiBound::Open;
        };
        warm_theta = best_nuisance;
        let diff = ss_cand - ss_crit;
        if diff >= 0.0 {
            bracket = Some(if prev_val <= cand { (prev_val, cand) } else { (cand, prev_val) });
            break;
        }
        // Detect flat asymptote plateau below ss_crit after expanding far beyond 16x SE.
        if iter >= 8 && (ss_cand - last_ss).abs() <= 1e-8 * ss_crit && ss_cand < ss_crit * 0.99 {
            return CiBound::Open;
        }
        if hit_box {
            return CiBound::Open;
        }
        prev_val = cand;
        prev_diff = diff;
        last_ss = ss_cand;
        step *= if iter < 4 { 1.8 } else { 2.5 };
    }

    let _ = prev_diff;
    let Some((a, b)) = bracket else {
        return CiBound::Open;
    };

    let tol = 1e-7 * base_step.max(1e-12);
    let opts = RootOptions { rel_tol: 1e-7, abs_tol: tol, max_iters: 48 };
    let mut warm_brent = warm_theta;
    let root = brent_root(
        |val| {
            if let Some((ss_v, next_w)) = profile_solve(k, val, &warm_brent) {
                warm_brent = next_w;
                ss_v - ss_crit
            } else {
                f64::NAN
            }
        },
        a,
        b,
        opts,
    );
    match root {
        Ok(r) if r.is_finite() => CiBound::Finite(r),
        _ => CiBound::Finite(0.5 * (a + b)),
    }
}

/// Compare two nested regression models using the Extra-Sum-of-Squares *F*-test (`model_1` simpler
/// with `df_1 > df_2`, `model_2` more complex).
///
/// `F = ((SS_1 - SS_2) / (df_1 - df_2)) / (SS_2 / df_2)` on `(df_1 - df_2, df_2)` degrees of
/// freedom (Seber & Wild 1989; Motulsky & Christopoulos 2004). Exact for nested linear models;
/// approximate for nonlinear ones.
///
/// # Errors
/// Returns [`FitError::InvalidParameter`] if degrees of freedom or sums of squares are invalid.
pub fn compare_models_f_test(
    ss_simple: f64,
    df_simple: usize,
    ss_complex: f64,
    df_complex: usize,
    alpha: f64,
) -> Result<ExtraSumOfSquaresResult, FitError> {
    if df_complex == 0 || df_simple <= df_complex {
        return Err(FitError::InvalidParameter { name: "df", reason: format!("requires df_simple ({df_simple}) > df_complex ({df_complex}) > 0") });
    }
    if ss_simple < 0.0 || ss_complex < 0.0 || !ss_simple.is_finite() || !ss_complex.is_finite() {
        return Err(FitError::InvalidParameter { name: "ss", reason: String::from("SS must be finite and non-negative") });
    }
    let df_num = df_simple - df_complex;
    let df_den = df_complex;
    let ms_den = ss_complex / (df_den as f64);
    let ms_num = (ss_simple - ss_complex) / (df_num as f64);
    let f_stat = if ms_den > 0.0 {
        (ms_num / ms_den).max(0.0)
    } else if ms_num > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_value = if f_stat.is_infinite() { 0.0 } else { f_sf(f_stat, df_num as f64, df_den as f64)? };
    Ok(ExtraSumOfSquaresResult { f_stat, df_num, df_den, p_value, preferred_complex_model: p_value < alpha })
}

/// Compare two nested regression models using the Gaussian Likelihood Ratio Test
/// ($D = 2(\ln L_{\text{alt}} - \ln L_{\text{null}}) = N \ln(\text{SS}_{\text{null}} / \text{SS}_{\text{alt}}) \sim \chi^2_{\Delta k}$).
///
/// The `N ln(SS_null / SS_alt)` form uses the Gaussian log-likelihood maximised over `sigma^2`;
/// the asymptotic chi-square reference distribution is Wilks (1938).
///
/// # Errors
/// Returns [`FitError::InvalidParameter`] if degrees of freedom or sums of squares are invalid.
pub fn compare_models_lrt(
    ss_null: f64,
    df_null: usize,
    ss_alt: f64,
    df_alt: usize,
    n_obs: usize,
    alpha: f64,
) -> Result<LikelihoodRatioResult, FitError> {
    if n_obs == 0 || df_alt == 0 || df_null <= df_alt {
        return Err(FitError::InvalidParameter { name: "df", reason: format!("requires df_null ({df_null}) > df_alt ({df_alt}) > 0 and n_obs > 0") });
    }
    if ss_null < 0.0 || ss_alt < 0.0 || !ss_null.is_finite() || !ss_alt.is_finite() {
        return Err(FitError::InvalidParameter { name: "ss", reason: String::from("SS must be finite and non-negative") });
    }
    let df_diff = df_null - df_alt;
    let lr_statistic = if ss_alt == 0.0 && ss_null == 0.0 {
        0.0
    } else if ss_alt <= 0.0 {
        (n_obs as f64) * (ss_null.max(1e-300) / 1e-300).ln()
    } else {
        ((n_obs as f64) * (ss_null / ss_alt).ln()).max(0.0)
    };
    let p_value = chi2_sf(lr_statistic, df_diff as f64)?.clamp(0.0, 1.0);
    Ok(LikelihoodRatioResult { lr_statistic, df_diff, df_null, df_alt, p_value, preferred_complex_model: p_value < alpha })
}

/// Likelihood ratio test from deviances: `D_null - D_alt ~ chi^2_{df_null - df_alt}`
/// (McCullagh & Nelder 1989; Wilks 1938).
pub(crate) fn compare_models_lrt_deviance(
    dev_null: f64,
    df_null: usize,
    dev_alt: f64,
    df_alt: usize,
    alpha: f64,
) -> Result<LikelihoodRatioResult, FitError> {
    if df_alt == 0 || df_null <= df_alt {
        return Err(FitError::InvalidParameter { name: "df", reason: format!("requires df_null ({df_null}) > df_alt ({df_alt}) > 0") });
    }
    if dev_null < 0.0 || dev_alt < 0.0 || !dev_null.is_finite() || !dev_alt.is_finite() {
        return Err(FitError::InvalidParameter { name: "deviance", reason: String::from("Deviance must be finite and non-negative") });
    }
    let df_diff = df_null - df_alt;
    let lr_statistic = (dev_null - dev_alt).max(0.0);
    let p_value = chi2_sf(lr_statistic, df_diff as f64)?.clamp(0.0, 1.0);
    Ok(LikelihoodRatioResult { lr_statistic, df_diff, df_null, df_alt, p_value, preferred_complex_model: p_value < alpha })
}

/// Compute $\text{AIC}_c$ and $\text{BIC}$ for least-squares residuals (`K = p_free + 1` including $\sigma^2$).
///
/// Least-squares forms (additive constants dropped): `AIC = N ln(SS / N) + 2K` (Akaike 1974;
/// Burnham & Anderson 2002), `AICc = AIC + 2K(K + 1) / (N - K - 1)` (Hurvich & Tsai 1989), and
/// `BIC = N ln(SS / N) + K ln N` (Schwarz 1978). Only valid when `ss` is a (weighted) Gaussian
/// residual sum of squares.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if `n_obs <= k_free + 2` (so $N - K - 1 \le 0$).
pub fn compute_information_criteria(ss: f64, n_obs: usize, p_free: usize) -> Result<InformationCriteria, FitError> {
    let k = p_free + 1;
    if n_obs <= k + 1 {
        return Err(FitError::InsufficientData { required: k + 2, actual: n_obs });
    }
    let n_f = n_obs as f64;
    let k_f = k as f64;
    let mse_ratio = (ss / n_f).max(1e-300);
    let aic = n_f * mse_ratio.ln() + 2.0 * k_f;
    let aicc = aic + (2.0 * k_f * (k_f + 1.0)) / ((n_obs - k - 1) as f64);
    let bic = n_f * mse_ratio.ln() + k_f * n_f.ln();
    Ok(InformationCriteria { aicc, bic, n_obs, k_params_plus_sigma: k })
}

/// Compare two models on identical observations via $\text{AIC}_c$, Akaike weights, and evidence ratio.
///
/// Akaike weights `w_i = exp(-Delta_i / 2) / sum exp(-Delta_j / 2)` and evidence ratio
/// `exp(|Delta AICc| / 2)` per Burnham & Anderson (2002).
///
/// # Errors
/// Propagates [`compute_information_criteria`] errors if sample size is too small.
pub fn compare_models_aicc(ss_a: f64, p_free_a: usize, ss_b: f64, p_free_b: usize, n_obs: usize) -> Result<AiccComparisonResult, FitError> {
    let ic_a = compute_information_criteria(ss_a, n_obs, p_free_a)?;
    let ic_b = compute_information_criteria(ss_b, n_obs, p_free_b)?;
    let min_aicc = ic_a.aicc.min(ic_b.aicc);
    let wa_raw = (-0.5 * (ic_a.aicc - min_aicc)).exp();
    let wb_raw = (-0.5 * (ic_b.aicc - min_aicc)).exp();
    let sum_w = wa_raw + wb_raw;
    let weight_a = wa_raw / sum_w;
    let weight_b = wb_raw / sum_w;
    let delta = (ic_a.aicc - ic_b.aicc).abs();
    let evidence_ratio = (0.5 * delta).exp();
    let preferred_index = usize::from(ic_b.aicc < ic_a.aicc);
    Ok(AiccComparisonResult {
        aicc_a: ic_a.aicc,
        aicc_b: ic_b.aicc,
        delta_aicc: ic_b.aicc - ic_a.aicc,
        weight_a,
        weight_b,
        evidence_ratio,
        preferred_index,
    })
}

/// Wald–Wolfowitz runs test on ordered residuals (`runs` of positive vs negative residuals).
///
/// Wald & Wolfowitz (1940). For `n <= 200` the p-value uses the exact null distribution of the
/// number of runs (as tabulated by Swed & Eisenhart 1943); otherwise the normal approximation
/// with mean `1 + 2 n1 n2 / n`, variance `2 n1 n2 (2 n1 n2 - n) / (n^2 (n - 1))`, and a 0.5
/// continuity correction. Zero residuals are dropped. Use in curve fitting follows
/// Motulsky & Christopoulos (2004); `deviates_significantly` uses the one-sided "too few runs"
/// p-value at a fixed `0.05`.
#[must_use]
pub fn runs_test(residuals: &[f64]) -> RunsTestResult {
    let signs: Vec<bool> = residuals.iter().filter(|&&r| r.is_finite() && r != 0.0).map(|&r| r > 0.0).collect();
    let n_above = signs.iter().filter(|&&s| s).count();
    let n_below = signs.len() - n_above;
    if n_above == 0 || n_below == 0 {
        return RunsTestResult {
            runs: usize::from(!signs.is_empty()),
            n_above,
            n_below,
            expected_runs: if signs.is_empty() { 0.0 } else { 1.0 },
            p_too_few_runs: 1.0,
            p_two_sided: 1.0,
            deviates_significantly: false,
        };
    }

    let mut runs = 1_usize;
    for pair in signs.windows(2) {
        if pair[0] != pair[1] {
            runs += 1;
        }
    }

    let n1 = n_above as f64;
    let n2 = n_below as f64;
    let n = n1 + n2;
    let expected = 1.0 + (2.0 * n1 * n2) / n;

    let p_too_few = if signs.len() <= 200 {
        exact_runs_cdf(runs, n_above, n_below)
    } else {
        let var = (2.0 * n1 * n2 * (2.0 * n1 * n2 - n)) / (n * n * (n - 1.0));
        let sd = var.max(1e-12).sqrt();
        let z = ((runs as f64) + 0.5 - expected) / sd;
        1.0 - norm_sf(z)
    }
    .clamp(0.0, 1.0);

    let p_two_sided = if signs.len() <= 200 {
        exact_runs_two_sided(runs, n_above, n_below)
    } else {
        let var = (2.0 * n1 * n2 * (2.0 * n1 * n2 - n)) / (n * n * (n - 1.0));
        let sd = var.max(1e-12).sqrt();
        let z = (((runs as f64) - expected).abs() - 0.5).max(0.0) / sd;
        2.0 * norm_sf(z)
    }
    .clamp(0.0, 1.0);

    RunsTestResult {
        runs,
        n_above,
        n_below,
        expected_runs: expected,
        p_too_few_runs: p_too_few,
        p_two_sided,
        deviates_significantly: p_too_few < 0.05,
    }
}

fn exact_runs_prob(r: usize, n1: usize, n2: usize) -> f64 {
    if r < 2 || n1 == 0 || n2 == 0 {
        return 0.0;
    }
    let Ok(ln_denom) = ln_choose((n1 + n2) as u64, n1 as u64) else {
        return 0.0;
    };
    let k = r / 2;
    if r.is_multiple_of(2) {
        if k == 0 || k > n1 || k > n2 {
            return 0.0;
        }
        let Ok(c1) = ln_choose((n1 - 1) as u64, (k - 1) as u64) else {
            return 0.0;
        };
        let Ok(c2) = ln_choose((n2 - 1) as u64, (k - 1) as u64) else {
            return 0.0;
        };
        (2.0_f64.ln() + c1 + c2 - ln_denom).exp()
    } else {
        let mut p = 0.0;
        if k >= 1
            && k <= n1
            && k < n2
            && let (Ok(c1), Ok(c2)) = (ln_choose((n1 - 1) as u64, (k - 1) as u64), ln_choose((n2 - 1) as u64, k as u64))
        {
            p += (c1 + c2 - ln_denom).exp();
        }
        if k >= 1
            && k < n1
            && k <= n2
            && let (Ok(c1), Ok(c2)) = (ln_choose((n1 - 1) as u64, k as u64), ln_choose((n2 - 1) as u64, (k - 1) as u64))
        {
            p += (c1 + c2 - ln_denom).exp();
        }
        p
    }
}

fn exact_runs_cdf(r_obs: usize, n1: usize, n2: usize) -> f64 {
    let max_r = 2 * n1.min(n2) + usize::from(n1 != n2);
    let mut acc = NeumaierSum::new();
    for r in 2..=r_obs.min(max_r) {
        acc.add(exact_runs_prob(r, n1, n2));
    }
    acc.total().clamp(0.0, 1.0)
}

fn exact_runs_two_sided(r_obs: usize, n1: usize, n2: usize) -> f64 {
    let p_obs = exact_runs_prob(r_obs, n1, n2);
    let max_r = 2 * n1.min(n2) + usize::from(n1 != n2);
    let mut acc = NeumaierSum::new();
    for r in 2..=max_r {
        let p = exact_runs_prob(r, n1, n2);
        if p <= p_obs * (1.0 + 1e-12) {
            acc.add(p);
        }
    }
    acc.total().clamp(0.0, 1.0)
}

/// Replicates test for lack of fit (partitions residual sum of squares into pure error at
/// replicate `X` values and lack-of-fit).
///
/// Pure-error / lack-of-fit partition and `F = MS_LOF / MS_PE` per Draper & Smith (1998) and
/// Motulsky & Christopoulos (2004). Pure error is computed from unweighted replicate `Y`
/// values (or `(n - 1) SD^2` for pre-summarised points), so `ss_res` should be on the same
/// unweighted scale for the partition to be exact.
#[must_use]
pub fn replicates_lack_of_fit(points: &[DataPoint], ss_res: f64, df_res: usize) -> Option<LackOfFitResult> {
    if points.is_empty() || df_res == 0 || !ss_res.is_finite() || ss_res < 0.0 {
        return None;
    }
    let mut groups: BTreeMap<u64, Vec<f64>> = BTreeMap::new();
    let mut has_summarized = false;
    let mut ss_pe_acc = NeumaierSum::new();
    let mut df_pe = 0_usize;

    for pt in points {
        if let (Some(sd), Some(n)) = (pt.sd, pt.n) {
            has_summarized = true;
            if !n.is_finite() || !(1.0..=1e9).contains(&n) || !sd.is_finite() {
                return None;
            }
            if n > 1.0 {
                let df_i = (n - 1.0).round() as usize;
                ss_pe_acc.add((n - 1.0) * sd * sd);
                df_pe = df_pe.checked_add(df_i)?;
            }
        } else {
            let key = pt.x0().to_bits();
            groups.entry(key).or_default().push(pt.y);
        }
    }

    if !has_summarized {
        for ys in groups.values() {
            if ys.len() > 1
                && let Ok(m) = SummaryMoments::from_slice(ys)
            {
                ss_pe_acc.add(m.css);
                df_pe = df_pe.checked_add(ys.len() - 1)?;
            }
        }
    }

    if df_pe == 0 || df_res <= df_pe {
        return None;
    }
    let ss_pure_error = ss_pe_acc.total().max(0.0);
    if !ss_pure_error.is_finite() {
        return None;
    }
    let ss_lack_of_fit = (ss_res - ss_pure_error).max(0.0);
    let df_lack_of_fit = df_res - df_pe;
    let ms_pe = ss_pure_error / (df_pe as f64);
    let ms_lof = ss_lack_of_fit / (df_lack_of_fit as f64);
    let f_stat = if ms_pe > 0.0 {
        ms_lof / ms_pe
    } else if ms_lof > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p_value = if f_stat.is_infinite() { 0.0 } else { f_sf(f_stat, df_lack_of_fit as f64, df_pe as f64).unwrap_or(1.0) };
    Some(LackOfFitResult {
        ss_pure_error,
        df_pure_error: df_pe,
        ss_lack_of_fit,
        df_lack_of_fit,
        f_stat,
        p_value,
        significant_lack_of_fit: p_value < 0.05,
    })
}

/// Shapiro–Wilk normality test (Royston 1995 AS R94) on residuals (`3 <= n <= 5000`).
///
/// Shapiro & Wilk (1965) `W` with Royston's (1995) polynomial approximations for the
/// coefficients `a_n`, `a_{n-1}` and the normalising transformations for the p-value
/// (exact arcsine form at `n = 3`).
///
/// Deviation: for `4 <= n <= 5` the coefficients are the normalised Blom scores `m / ||m||`
/// rather than AS R94's polynomial-corrected `a_n`; `n = 3` coincides with AS R94.
#[must_use]
pub fn shapiro_wilk_test(values: &[f64]) -> Option<NormalityTestResult> {
    let n = values.len();
    if !(3..=5000).contains(&n) {
        return None;
    }
    let mut sorted = values.to_vec();
    for &v in &sorted {
        if !v.is_finite() {
            return None;
        }
    }
    sorted.sort_by(f64::total_cmp);
    let moments = SummaryMoments::from_slice(&sorted).ok()?;
    if moments.css <= 0.0 {
        return None;
    }

    let nf = n as f64;
    let mut m = Vec::with_capacity(n);
    for i in 1..=n {
        let p = ((i as f64) - 0.375) / (nf + 0.25);
        m.push(norm_ppf(p).ok()?);
    }
    let m_ss: f64 = m.iter().map(|&v| v * v).sum();
    let u = 1.0 / nf.sqrt();
    let mut a = vec![0.0_f64; n];

    if n <= 5 {
        let denom = m_ss.sqrt();
        for (slot, &mi) in a.iter_mut().zip(m.iter()) {
            *slot = mi / denom;
        }
    } else {
        let c_last = m[n - 1] / m_ss.sqrt();
        let c_prev = m[n - 2] / m_ss.sqrt();
        let a_n = royston_poly_an(u) + c_last;
        let a_n1 = royston_poly_an1(u) + c_prev;
        let (phi, start_idx) = if n > 5 {
            let num = m_ss - 2.0 * m[n - 1] * m[n - 1] - 2.0 * m[n - 2] * m[n - 2];
            let den = 1.0 - 2.0 * a_n * a_n - 2.0 * a_n1 * a_n1;
            ((num / den.max(1e-12)).sqrt(), 2)
        } else {
            let num = m_ss - 2.0 * m[n - 1] * m[n - 1];
            let den = 1.0 - 2.0 * a_n * a_n;
            ((num / den.max(1e-12)).sqrt(), 1)
        };
        a[n - 1] = a_n;
        a[0] = -a_n;
        if start_idx == 2 {
            a[n - 2] = a_n1;
            a[1] = -a_n1;
        }
        for i in start_idx..(n - start_idx) {
            a[i] = m[i] / phi;
        }
    }

    let mut num_acc = NeumaierSum::new();
    for (&ai, &xi) in a.iter().zip(sorted.iter()) {
        num_acc.add(ai * xi);
    }
    let num = num_acc.total();
    let w = ((num * num) / moments.css).clamp(0.0, 1.0);
    let p_value = royston_sw_p_value(w, nf);
    Some(NormalityTestResult { test_name: String::from("Shapiro-Wilk"), statistic: w, p_value, passed: p_value >= 0.05 })
}

fn royston_poly_an(u: f64) -> f64 {
    const C: [f64; 6] = [0.0, 0.221_157, -0.147_981, -2.071_19, 4.434_685, -2.706_056];
    let mut acc = C[5];
    for &c in C[..5].iter().rev() {
        acc = acc * u + c;
    }
    acc
}

fn royston_poly_an1(u: f64) -> f64 {
    const C: [f64; 6] = [0.0, 0.042_981, -0.293_762, -1.752_461, 5.682_633, -3.582_633];
    let mut acc = C[5];
    for &c in C[..5].iter().rev() {
        acc = acc * u + c;
    }
    acc
}

fn royston_sw_p_value(w: f64, nf: f64) -> f64 {
    let one_minus_w = (1.0 - w).max(1e-16);
    if nf == 3.0 {
        let sqrtw = w.sqrt();
        let pi6 = 6.0 / core::f64::consts::PI;
        let p = pi6 * (sqrtw.clamp(-1.0, 1.0).asin() - core::f64::consts::FRAC_PI_3);
        return p.clamp(0.0, 1.0);
    }
    let y = one_minus_w.ln();
    let z = if nf <= 11.0 {
        let gamma = 0.459 * nf - 2.273;
        let mu = -0.000_671_4 * nf.powi(3) + 0.025_054 * nf.powi(2) - 0.399_78 * nf + 0.544;
        let sigma = (-0.002_032_2 * nf.powi(3) + 0.062_767 * nf.powi(2) - 0.778_57 * nf + 1.382_2).exp();
        let w_trans = -(gamma - y).max(1e-16).ln();
        (w_trans - mu) / sigma
    } else {
        let lnn = nf.ln();
        let mu = 0.003_891_5 * lnn.powi(3) - 0.083_751 * lnn.powi(2) - 0.310_82 * lnn - 1.586_1;
        let sigma = (0.003_030_2 * lnn.powi(2) - 0.082_676 * lnn - 0.480_3).exp();
        (y - mu) / sigma
    };
    norm_sf(z).clamp(0.0, 1.0)
}

/// D'Agostino–Pearson $K^2$ omnibus normality test (`n >= 8`).
///
/// `K^2 = Z_1^2 + Z_2^2 ~ chi^2_2`, with the skewness transformation `Z_1` (D'Agostino 1970) and
/// the kurtosis transformation `Z_2` of Anscombe & Glynn (1983), as assembled by
/// D'Agostino, Belanger & D'Agostino (1990).
///
/// Reference: D'Agostino, R. B. (1970). Transformation to normality of the null distribution of g1. *Biometrika* 57(3), 679–681.
#[must_use]
pub fn dagostino_pearson_test(values: &[f64]) -> Option<NormalityTestResult> {
    let n = values.len();
    if n < 8 {
        return None;
    }
    let moments = SummaryMoments::from_slice(values).ok()?;
    if moments.css <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let m2 = moments.css / nf;
    let mut m3_acc = NeumaierSum::new();
    let mut m4_acc = NeumaierSum::new();
    for &x in values {
        let d = x - moments.mean;
        let d2 = d * d;
        m3_acc.add(d2 * d);
        m4_acc.add(d2 * d2);
    }
    let m3 = m3_acc.total() / nf;
    let m4 = m4_acc.total() / nf;
    let sqrt_b1 = m3 / m2.powf(1.5);
    let b2 = m4 / (m2 * m2);

    // Skewness Z1 (D'Agostino 1970)
    let y = sqrt_b1 * ((nf + 1.0) * (nf + 3.0) / (6.0 * (nf - 2.0))).sqrt();
    let beta2 = (3.0 * (nf * nf + 27.0 * nf - 70.0) * (nf + 1.0) * (nf + 3.0)) / ((nf - 2.0) * (nf + 5.0) * (nf + 7.0) * (nf + 9.0));
    let w2 = -1.0 + (2.0 * (beta2 - 1.0)).sqrt();
    let delta = 1.0 / (0.5 * w2.ln()).sqrt();
    let alpha = (2.0 / (w2 - 1.0)).sqrt();
    let z1 = delta * ((y / alpha) + ((y / alpha).powi(2) + 1.0).sqrt()).ln();

    // Kurtosis Z2 (Anscombe & Glynn 1983)
    let e_b2 = 3.0 * (nf - 1.0) / (nf + 1.0);
    let var_b2 = (24.0 * nf * (nf - 2.0) * (nf - 3.0)) / ((nf + 1.0).powi(2) * (nf + 3.0) * (nf + 5.0));
    let x_k = (b2 - e_b2) / var_b2.sqrt();
    let sqrt_beta1 =
        (6.0 * (nf * nf - 5.0 * nf + 2.0) / ((nf + 7.0) * (nf + 9.0))) * ((6.0 * (nf + 3.0) * (nf + 5.0)) / (nf * (nf - 2.0) * (nf - 3.0))).sqrt();
    let a_k = 6.0 + (8.0 / sqrt_beta1) * (2.0 / sqrt_beta1 + (1.0 + 4.0 / (sqrt_beta1 * sqrt_beta1)).sqrt());
    let term = (1.0 - 2.0 / a_k) / (1.0 + x_k * (2.0 / (a_k - 4.0)).sqrt()).max(1e-12);
    let z2 = ((1.0 - 2.0 / (9.0 * a_k)) - term.cbrt()) / (2.0 / (9.0 * a_k)).sqrt();

    let k2 = z1 * z1 + z2 * z2;
    let p_value = chi2_sf(k2, 2.0).unwrap_or(1.0).clamp(0.0, 1.0);
    Some(NormalityTestResult { test_name: String::from("D'Agostino-Pearson K2"), statistic: k2, p_value, passed: p_value >= 0.05 })
}

/// Anderson–Darling normality test on residuals (`n >= 8`) with Stephens small-sample adjustment $A^{*2}$.
///
/// `A^2` of Anderson & Darling (1954) with mean and SD estimated, the case-3 modification
/// `A*^2 = A^2 (1 + 0.75/n + 2.25/n^2)`, and the piecewise exponential p-value approximation
/// tabulated in D'Agostino & Stephens (1986).
#[must_use]
pub fn anderson_darling_test(values: &[f64]) -> Option<NormalityTestResult> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = sorted.len();
    if n < 8 {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let moments = SummaryMoments::from_slice(&sorted).ok()?;
    let sd = moments.sample_sd().ok()?;
    if sd <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let mut sum_acc = NeumaierSum::new();
    for i in 0..n {
        let z_lo = (sorted[i] - moments.mean) / sd;
        let z_hi = (sorted[n - 1 - i] - moments.mean) / sd;
        let p_lo = norm_cdf(z_lo).clamp(1e-16, 1.0 - 1e-16);
        let q_hi = norm_sf(z_hi).clamp(1e-16, 1.0 - 1e-16);
        sum_acc.add(((2 * i + 1) as f64) * (p_lo.ln() + q_hi.ln()));
    }
    let a2 = (-nf - sum_acc.total() / nf).max(0.0);
    let a2_star = a2 * (1.0 + 0.75 / nf + 2.25 / (nf * nf));

    let p_value = if a2_star >= 0.6 {
        (1.2937 - 5.709 * a2_star + 0.0186 * a2_star * a2_star).exp()
    } else if a2_star > 0.34 {
        (0.9177 - 4.279 * a2_star - 1.38 * a2_star * a2_star).exp()
    } else if a2_star > 0.20 {
        1.0 - (-8.318 + 42.796 * a2_star - 59.938 * a2_star * a2_star).exp()
    } else {
        1.0 - (-13.436 + 101.14 * a2_star - 223.73 * a2_star * a2_star).exp()
    }
    .clamp(0.0, 1.0);

    Some(NormalityTestResult { test_name: String::from("Anderson-Darling A2*"), statistic: a2_star, p_value, passed: p_value >= 0.05 })
}

/// Lilliefors-adjusted Kolmogorov–Smirnov normality test on residuals (`n >= 5`).
///
/// Statistic `D` per Lilliefors (1967). P-values below `0.10` use the analytic approximation of
/// Dallal & Wilkinson (1986), which those authors only claim for upper-tail probabilities below
/// `0.10`.
///
/// Uncited: the `n > 100` rescaling `D (n / 100)^0.49` and the quartic polynomials in the
/// modified statistic `D (sqrt(n) - 0.01 + 0.85 / sqrt(n))` used when the Dallal–Wilkinson value
/// exceeds `0.10` have no published source recorded here; they need a verified reference or
/// an independent oracle before this branch satisfies rule D1.
#[must_use]
pub fn lilliefors_ks_test(values: &[f64]) -> Option<NormalityTestResult> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = sorted.len();
    if n < 5 {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let moments = SummaryMoments::from_slice(&sorted).ok()?;
    let sd = moments.sample_sd().ok()?;
    if sd <= 0.0 {
        return None;
    }
    let nf = n as f64;
    let mut d_max = 0.0_f64;
    for (i, &x) in sorted.iter().enumerate() {
        let f_z = norm_cdf((x - moments.mean) / sd);
        let d_plus = ((i + 1) as f64) / nf - f_z;
        let d_minus = f_z - (i as f64) / nf;
        d_max = d_max.max(d_plus).max(d_minus);
    }

    let (d_eff, n_eff) = if n > 100 { (d_max * (nf / 100.0).powf(0.49), 100.0) } else { (d_max, nf) };
    let sqrt_n_term = (n_eff + 2.78019).sqrt();
    let p_dw =
        (-7.01256 * d_eff * d_eff * (n_eff + 2.78019) + 2.99587 * d_eff * sqrt_n_term - 0.122_119 + 0.974_598 / n_eff.sqrt() + 1.67997 / n_eff).exp();

    let p_value = if p_dw <= 0.10 {
        p_dw.clamp(0.0, 1.0)
    } else {
        let k = d_max * (nf.sqrt() - 0.01 + 0.85 / nf.sqrt());
        if k <= 0.302 {
            1.0
        } else if k <= 0.500 {
            (2.767_73 - 19.828_315 * k + 80.709_644 * k * k - 138.551_52 * k.powi(3) + 81.218_052 * k.powi(4)).clamp(0.10, 1.0)
        } else if k <= 0.900 {
            (-4.901_232 + 40.662_806 * k - 97.490_286 * k * k + 94.029_866 * k.powi(3) - 32.355_711 * k.powi(4)).clamp(0.0, 1.0)
        } else {
            p_dw.clamp(0.0, 1.0)
        }
    };

    Some(NormalityTestResult { test_name: String::from("Kolmogorov-Smirnov (Lilliefors)"), statistic: d_max, p_value, passed: p_value >= 0.05 })
}

fn average_ranks(vals: &[f64]) -> Vec<f64> {
    let n = vals.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| vals[a].total_cmp(&vals[b]));
    let mut ranks = vec![0.0_f64; n];
    let mut i = 0_usize;
    while i < n {
        let mut j = i + 1;
        while j < n && vals[order[j]] == vals[order[i]] {
            j += 1;
        }
        let avg_rank = 0.5 * ((i + 1 + j) as f64);
        for &idx in &order[i..j] {
            ranks[idx] = avg_rank;
        }
        i = j;
    }
    ranks
}

/// Homoscedasticity check via Spearman rank correlation between `|residual|` and predicted `Y`.
///
/// Spearman rank correlation (average ranks for ties) tested with the `t = r sqrt((n - 2) /
/// (1 - r^2))` approximation on `n - 2` df (Zar 2010). Applying it to `|residual|` versus fitted
/// `Y` is a heuristic screen for a monotone variance trend, not a named published test.
#[must_use]
pub fn homoscedasticity_spearman_test(y_hat: &[f64], residuals: &[f64], alpha: f64) -> Option<HomoscedasticityResult> {
    let mut yh_vals = Vec::with_capacity(y_hat.len().min(residuals.len()));
    let mut abs_r = Vec::with_capacity(yh_vals.capacity());
    for (&yh, &r) in y_hat.iter().zip(residuals.iter()) {
        if yh.is_finite() && r.is_finite() {
            yh_vals.push(yh);
            abs_r.push(r.abs());
        }
    }
    let n = yh_vals.len();
    if n < 3 {
        return None;
    }
    let rx = average_ranks(&yh_vals);
    let ry = average_ranks(&abs_r);
    let rho = pearson_r(&rx, &ry).ok()?;
    let df = n - 2;
    let one_m_r2 = 1.0 - rho * rho;
    let (statistic_t, p_value) = if one_m_r2.abs() <= 1e-15 {
        (if rho >= 0.0 { f64::INFINITY } else { f64::NEG_INFINITY }, 0.0)
    } else {
        let t = rho * ((df as f64) / one_m_r2).sqrt();
        let p = student_t_two_sided_p(t, df as f64).ok()?.clamp(0.0, 1.0);
        (t, p)
    };
    Some(HomoscedasticityResult { spearman_rho: rho, statistic_t, df, p_value, homoscedastic: p_value >= alpha })
}

/// Compute the 68.27th percentile of absolute residuals multiplied by `N / (N - K)`
/// (`RSDR` scale per Motulsky & Brown 2006).
///
/// `RSDR = P68 * N / (N - K)`, where `P68` is the 68.27th percentile of `|r_i|` by linear
/// interpolation, matching the definition in Motulsky & Brown (2006).
#[must_use]
pub fn compute_rsdr(abs_residuals: &[f64], k_free: usize) -> f64 {
    let n = abs_residuals.len();
    if n == 0 {
        return 0.0;
    }
    let mut sorted = abs_residuals.to_vec();
    sorted.sort_by(f64::total_cmp);
    // 68.2689492th percentile via linear interpolation on 0..(n-1).
    let pos = 0.682_689_492 * ((n - 1) as f64);
    let lo = (pos.floor() as usize).min(n - 1);
    let hi = (pos.ceil() as usize).min(n - 1);
    let frac = pos - (lo as f64);
    let p68 = sorted[lo] * (1.0 - frac) + sorted[hi] * frac;
    let df = n.saturating_sub(k_free).max(1);
    p68 * (n as f64) / (df as f64)
}

/// Nonlinear ROUT outlier detection (`Motulsky & Brown 2006`):
/// 1. Compute `RSDR` with `K = k_free` adjusted degrees of freedom from the Cauchy robust fit residuals,
/// 2. Compute two-sided Student-*t* *p*-values `t_i = |r_i| / RSDR` with `df = N - K`,
/// 3. Apply Benjamini–Hochberg FDR at caller threshold `Q%` (`q_percent / 100`).
///
/// Motulsky & Brown's rule `alpha_i = Q (N - (i - 1)) / N` over residuals ranked by size is the
/// Benjamini & Hochberg (1995) step-up threshold `Q j / N` on the `j`-th smallest p-value, which
/// is what this function applies.
///
/// Deviation: Motulsky & Brown only test the 30% of points furthest from the curve (`i` from
/// `int(0.70 N)` to `N`); this implementation applies the step-up rule across all `N` points,
/// so with a large `Q` it can flag points the paper's procedure would never test.
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if `points.len() <= k_free`.
pub fn rout_detect_outliers(points: &[DataPoint], residuals: &[f64], k_free: usize, q_percent: f64) -> Result<RoutDetectionResult, FitError> {
    let n = points.len();
    if n <= k_free {
        return Err(FitError::InsufficientData { required: k_free + 1, actual: n });
    }
    if residuals.len() != n {
        return Err(FitError::InvalidParameter { name: "residuals", reason: String::from("length mismatch") });
    }
    let df = n - k_free;
    let abs_res: Vec<f64> = residuals.iter().map(|r| r.abs()).collect();
    let y_scale = points.iter().map(|p| p.y.abs()).fold(0.0_f64, f64::max).max(1e-12);
    let rsdr = compute_rsdr(&abs_res, k_free).max(1e-10 * y_scale);
    let q_fdr = (q_percent / 100.0).clamp(1e-6, 0.5);

    let mut p_vals = Vec::with_capacity(n);
    for &ar in &abs_res {
        let p = if rsdr <= 1e-30 {
            if ar <= 1e-30 { 1.0 } else { 0.0 }
        } else {
            let t = ar / rsdr;
            student_t_two_sided_p(t, df as f64).unwrap_or(1.0)
        };
        p_vals.push(p);
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| p_vals[a].total_cmp(&p_vals[b]));

    // Benjamini-Hochberg step-up adjusted q-values & cutoff.
    let mut bh_q = vec![1.0_f64; n];
    let mut running_min = 1.0_f64;
    for rank_rev in (0..n).rev() {
        let idx = order[rank_rev];
        let rank_1 = (rank_rev + 1) as f64;
        let raw_q = (p_vals[idx] * (n as f64) / rank_1).min(1.0);
        running_min = running_min.min(raw_q);
        bh_q[idx] = running_min;
    }

    let mut max_reject_rank: Option<usize> = None;
    for (rank0, &idx) in order.iter().enumerate() {
        let threshold = ((rank0 + 1) as f64) / (n as f64) * q_fdr;
        if p_vals[idx] <= threshold {
            max_reject_rank = Some(rank0);
        }
    }

    let mut is_outlier = vec![false; n];
    if let Some(m) = max_reject_rank {
        for &idx in &order[..=m] {
            is_outlier[idx] = true;
        }
    }

    let mut outlier_indices = Vec::new();
    let mut audit = Vec::with_capacity(n);
    for i in 0..n {
        if is_outlier[i] {
            outlier_indices.push(i);
        }
        audit.push(RoutPointAudit {
            index: i,
            data_set_index: points[i].data_set_index,
            row: points[i].row,
            x0: points[i].x0(),
            y: points[i].y,
            residual: residuals[i],
            p_value: p_vals[i],
            q_value: bh_q[i],
            is_outlier: is_outlier[i],
        });
    }

    Ok(RoutDetectionResult { rsdr, df, q_threshold_percent: q_percent, outlier_indices, audit })
}

#[cfg(test)]
mod tests {
    use crate::types::DataPoint;

    use super::replicates_lack_of_fit;

    #[test]
    fn replicates_lack_of_fit_guards_non_finite_and_extreme_n() {
        let inf_pts = vec![
            DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(f64::INFINITY), explicit_weight: None, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 3.0, sd: Some(0.5), n: Some(f64::INFINITY), explicit_weight: None, data_set_index: 0, row: 1 },
        ];
        assert!(replicates_lack_of_fit(&inf_pts, 10.0, 4).is_none());

        let huge_pts = vec![
            DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(1e10), explicit_weight: None, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 3.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, data_set_index: 0, row: 1 },
        ];
        assert!(replicates_lack_of_fit(&huge_pts, 10.0, 4).is_none());

        let valid_pts = vec![
            DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(1.0), explicit_weight: None, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 3.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, data_set_index: 0, row: 1 },
        ];
        let res = replicates_lack_of_fit(&valid_pts, 5.0, 4).unwrap();
        assert_eq!(res.df_pure_error, 2);
        assert_eq!(res.df_lack_of_fit, 2);
    }
}
