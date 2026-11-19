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
//! Reference: Royston, P. (1992). Approximating the Shapiro–Wilk W-test for non-normality. *Statistics and Computing* 2(3), 117–119.
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
//! Reference: Stephens, M. A. (1974). EDF statistics for goodness of fit and some comparisons. *Journal of the American Statistical Association* 69(347), 730–737.
//!
//! Reference: Dallal, G. E. & Wilkinson, L. (1986). An analytic approximation to the distribution of Lilliefors's test statistic for normality. *The American Statistician* 40(4), 294–296. doi:10.2307/2684607
//!
//! Reference: Zar, J. H. (2010). *Biostatistical Analysis*, 5th ed. Pearson Prentice Hall.
//!
//! Reference: Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123. doi:10.1186/1471-2105-7-123
//!
//! Reference: Benjamini, Y. & Hochberg, Y. (1995). Controlling the false discovery rate: a practical and powerful approach to multiple testing. *Journal of the Royal Statistical Society, Series B* 57(1), 289–300.

use std::collections::BTreeMap;

use curvebook_doc::CiBound;
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{
    chi2_isf, chi2_sf, f_sf, lilliefors_d_from_sorted, lilliefors_p_value, norm_cdf, norm_ppf, norm_sf, norm_two_sided_isf, student_t_two_sided_isf,
    student_t_two_sided_p,
};
use curvebook_math::linalg::{DenseMatrix, qr_col_pivoted, svd};
use curvebook_math::roots::{RootOptions, brent_root};
use curvebook_math::special::ln_choose;
use curvebook_math::sum::{NeumaierSum, SummaryMoments, pearson_r};

use crate::error::FitError;
use crate::types::{DataPoint, validated_replicate_moments};

/// Asymptotic covariance, standard errors, symmetric CIs, correlation matrix, and parameter dependency
/// (Bates & Watts 1988, §2.1; Motulsky & Christopoulos 2004).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CovarianceDiagnostics {
    /// Asymptotic parameter standard errors $\sqrt{\mathrm{Cov}_{jj}}$ (`NaN` when variance matrix is unavailable or unidentifiable).
    pub standard_errors: Vec<f64>,
    /// Lower bounds of symmetric asymptotic $t_{\alpha/2,\nu}$ Wald confidence intervals at the configured `ConfidenceLevel` (`NaN` when SE is unavailable).
    pub symmetric_ci_lower: Vec<f64>,
    /// Upper bounds of symmetric asymptotic $t_{\alpha/2,\nu}$ Wald confidence intervals at the configured `ConfidenceLevel` (`NaN` when SE is unavailable).
    pub symmetric_ci_upper: Vec<f64>,
    /// Linearized asymptotic variance–covariance matrix $s^2 (J^\top J)^{-1}$ over free parameters.
    pub covariance_matrix: Vec<Vec<f64>>,
    /// Symmetric parameter correlation matrix $R_{ij} = \mathrm{Cov}_{ij} / \sqrt{\mathrm{Cov}_{ii}\mathrm{Cov}_{jj}}$ in `[-1, 1]` (`NaN` on zero-variance slots).
    pub correlation_matrix: Vec<Vec<f64>>,
    /// Parameter collinearity dependency index $1 - 1 / ((J^\top J)^{-1}_{jj} (J^\top J)_{jj})$ in `[0, 1]` (`NaN` on unidentifiable fits; `> 0.9999` marks [`crate::FitStatus::Ambiguous`]).
    pub dependencies: Vec<f64>,
    /// Numerical column rank of the weighted Jacobian $J$ estimated via SVD singular-value tolerance.
    pub numerical_rank: usize,
    /// True when the Jacobian is rank-deficient or any parameter has `dependency > 0.9999`.
    pub ambiguous: bool,
}

/// Model comparison outcome via Extra-Sum-of-Squares *F*-test (Motulsky & Christopoulos 2004, Ch. 21).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtraSumOfSquaresResult {
    /// Extra-sum-of-squares *F*-ratio $((\mathrm{SS}_{\text{null}} - \mathrm{SS}_{\text{alt}}) / \mathrm{df}_{\text{num}}) / (\mathrm{SS}_{\text{alt}} / \mathrm{df}_{\text{den}})$.
    pub f_stat: f64,
    /// Numerator degrees of freedom $\mathrm{df}_{\text{null}} - \mathrm{df}_{\text{alt}}$ (difference in free parameters).
    pub df_num: usize,
    /// Denominator residual degrees of freedom $\mathrm{df}_{\text{alt}}$ of the more complex model.
    pub df_den: usize,
    /// Upper-tail *F* distribution *P* value testing the null hypothesis that the simpler model is correct.
    pub p_value: f64,
    /// True when `p_value < alpha` (`1 - confidence`), favoring the more complex alternative model.
    pub preferred_complex_model: bool,
}

/// Corrected Akaike Information Criterion ($\text{AIC}_c$) and Bayesian Information Criterion ($\text{BIC}$) summary
/// (Burnham & Anderson 2002; Schwarz 1978).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InformationCriteria {
    /// Small-sample corrected Akaike Information Criterion $\text{AIC}_c$ computed under least-squares Gaussian or Poisson deviance likelihood (includes residual variance $\sigma^2$ in Gaussian $K$).
    pub aicc: f64,
    /// Schwarz Bayesian Information Criterion $\text{BIC}$.
    pub bic: f64,
    /// Number of active observations $N$ included in the likelihood evaluation.
    pub n_obs: usize,
    /// Effective free parameter count $K$ counted in the penalty term ($k_{\text{free}} + 1$ for Gaussian least squares to account for estimated $\sigma^2$, or $k_{\text{free}}$ for fixed-scale Poisson).
    pub k_params_plus_sigma: usize,
}

/// Pairwise $\text{AIC}_c$ comparison between two models evaluated on identical observations
/// (Burnham & Anderson 2002, §2.9).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiccComparisonResult {
    /// Corrected AIC value $\text{AIC}_{c,A}$ for candidate model `0`.
    pub aicc_a: f64,
    /// Corrected AIC value $\text{AIC}_{c,B}$ for candidate model `1`.
    pub aicc_b: f64,
    /// Absolute difference $|\text{AIC}_{c,A} - \text{AIC}_{c,B}| \ge 0$ (lower AICc is better).
    pub delta_aicc: f64,
    /// Akaike relative model weight $w_A \in [0, 1]$ with $w_A + w_B = 1$.
    pub weight_a: f64,
    /// Akaike relative model weight $w_B \in [0, 1]$ with $w_A + w_B = 1$.
    pub weight_b: f64,
    /// Relative evidence ratio $\max(w_A, w_B) / \min(w_A, w_B) \ge 1$ in favor of the preferred model (`f64::INFINITY` when underflowed).
    pub evidence_ratio: f64,
    /// Zero-based index of the preferred model (`0` if `aicc_a <= aicc_b`, else `1`).
    pub preferred_index: usize,
}

/// Wald–Wolfowitz runs test for systematic curve-vs-data deviation along sorted `X`
/// (Wald & Wolfowitz 1940; Motulsky & Christopoulos 2004, Ch. 24).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunsTestResult {
    /// Observed count of contiguous same-sign residual runs (zeros dropped per Wald–Wolfowitz).
    pub runs: usize,
    /// Number of strictly positive residuals (`y - y_hat > 0`).
    pub n_above: usize,
    /// Number of strictly negative residuals (`y - y_hat < 0`).
    pub n_below: usize,
    /// Expected number of runs $1 + 2 n_{\text{above}} n_{\text{below}} / (n_{\text{above}} + n_{\text{below}})$ under independence.
    pub expected_runs: f64,
    /// Exact lower-tail probability $P(R \le r_{\text{obs}})$ that residuals cluster into too few runs (`1.0` when degenerate).
    pub p_too_few_runs: f64,
    /// Exact two-sided probability of observing a run count at least as extreme as `runs` relative to `expected_runs`.
    pub p_two_sided: f64,
    /// True when `p_too_few_runs < 0.05` (standard 5% non-randomness clustering threshold).
    pub deviates_significantly: bool,
}

/// Replicates test for lack of fit partitioning residual SS into pure error vs lack of fit
/// (Draper & Smith 1998, §2.5; Motulsky & Christopoulos 2004).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LackOfFitResult {
    /// Pure-error sum of squares within shared-`X` replicate groups in original/weighted fit scale.
    pub ss_pure_error: f64,
    /// Pure-error degrees of freedom $\sum_g (n_g - 1)$.
    pub df_pure_error: usize,
    /// Lack-of-fit sum of squares $\mathrm{SS}_{\text{res}} - \mathrm{SS}_{\text{pure\_error}}$.
    pub ss_lack_of_fit: f64,
    /// Lack-of-fit degrees of freedom $\mathrm{df}_{\text{res}} - \mathrm{df}_{\text{pure\_error}}$.
    pub df_lack_of_fit: usize,
    /// Mean-square ratio $(\mathrm{SS}_{\text{LOF}} / \mathrm{df}_{\text{LOF}}) / (\mathrm{SS}_{\text{PE}} / \mathrm{df}_{\text{PE}})$.
    pub f_stat: f64,
    /// Upper-tail *F*-distribution *P* value testing adequacy of the functional form (`1.0` if `f_stat <= 0`).
    pub p_value: f64,
    /// True when `p_value < 0.05`, indicating statistically significant lack of fit.
    pub significant_lack_of_fit: bool,
}

/// Omnibus residual normality test result (e.g. D'Agostino–Pearson $K^2$, Anderson–Darling $A^2$, or Lilliefors KS).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NormalityTestResult {
    /// Diagnostic test name (`"D'Agostino-Pearson K^2"`, `"Anderson-Darling"`, or `"Lilliefors (Kolmogorov-Smirnov)"`).
    pub test_name: String,
    /// Test statistic value in the native scale of `test_name` (dimensionless).
    pub statistic: f64,
    /// Two-sided / upper-tail *P* value against normality; values below alpha reject Gaussian residuals.
    pub p_value: f64,
    /// True when `p_value >= 0.05` (residuals are consistent with normality at the 5% level).
    pub passed: bool,
}

/// Model comparison outcome via Likelihood Ratio Test ($D = 2(\ln L_{\text{alt}} - \ln L_{\text{null}})$ chi-square test;
/// Neyman & Pearson 1928; Wilks 1938).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LikelihoodRatioResult {
    /// Likelihood-ratio chi-square test statistic $2(\ln L_{\text{alt}} - \ln L_{\text{null}}) \ge 0$.
    pub lr_statistic: f64,
    /// Difference in free parameters $\mathrm{df}_{\text{null}} - \mathrm{df}_{\text{alt}}$ (chi-square degrees of freedom).
    pub df_diff: usize,
    /// Residual degrees of freedom of the simpler nested null model.
    pub df_null: usize,
    /// Residual degrees of freedom of the richer alternative model.
    pub df_alt: usize,
    /// Upper-tail $\chi^2(\mathrm{df}_{\text{diff}})$ *P* value testing the simpler null model.
    pub p_value: f64,
    /// True when `p_value < alpha` (`1 - confidence`), favoring the more complex model.
    pub preferred_complex_model: bool,
}

/// Residual homoscedasticity check via Spearman rank correlation between `|residual|` and predicted `Y`
/// (Spearman 1904; Motulsky & Christopoulos 2004).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HomoscedasticityResult {
    /// Spearman rank correlation coefficient $\rho \in [-1, 1]$ between $|r_i|$ (or weighted $|r_i|$) and $\hat{y}_i$.
    pub spearman_rho: f64,
    /// Student *t* approximation statistic $\rho \sqrt{\mathrm{df} / (1 - \rho^2)}$ (`0.0` at $\rho = 0$, large finite/inf when $|\rho| = 1$).
    pub statistic_t: f64,
    /// Degrees of freedom `n_obs - 2` (`>= 1`).
    pub df: usize,
    /// Two-sided *P* value from the $t(\mathrm{df})$ distribution testing zero rank trend in residual magnitude.
    pub p_value: f64,
    /// True when `p_value >= 0.05` (no significant evidence of heteroscedasticity at the 5% level).
    pub homoscedastic: bool,
}

/// Per-point ROUT outlier audit entry (Motulsky & Brown, *BMC Bioinformatics* 2006).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoutPointAudit {
    /// Zero-based position in the flattened active observation vector fed to the solver.
    pub index: usize,
    /// Originating zero-based dataset index (`0 = A`, `1 = B`, ...).
    pub data_set_index: usize,
    /// Originating zero-based table row index within `data_set_index`.
    pub row: usize,
    /// Zero-based raw Y replicate subcolumn index within `data_set_index`.
    #[serde(default)]
    pub source_subcolumn: usize,
    /// Primary predictor coordinate $x_0$ at this observation.
    pub x0: f64,
    /// Observed response $y_i$ in original table units.
    pub y: f64,
    /// Robust raw residual $y_i - \hat{y}_{\text{robust}, i}$ in original $Y$ units.
    pub residual: f64,
    /// Two-sided Student *t* outlier *P* value $|r_i| / \text{RSDR}$ on `df` degrees of freedom.
    pub p_value: f64,
    /// Benjamini–Hochberg FDR-adjusted *q* value in `[0, 1]` across all observations evaluated together.
    pub q_value: f64,
    /// True if this observation was flagged as an outlier (`q_value <= q_threshold_percent / 100`) and excluded.
    pub is_outlier: bool,
}

/// Result of nonlinear ROUT outlier detection (`Motulsky & Brown 2006`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoutDetectionResult {
    /// Robust Standard Deviation of Residuals (`RSDR`) computed from the 68.27th percentile of $|r_i|$ times $n / (n - k_{\text{free}})$.
    pub rsdr: f64,
    /// Residual degrees of freedom `n_obs - k_free` used for Student *t* outlier *P* values.
    pub df: usize,
    /// Configured false-discovery-rate threshold $Q$ in percent (e.g. `1.0` for `Q = 1%`).
    pub q_threshold_percent: f64,
    /// Sorted flattened indices into the active observation list identified as outliers.
    pub outlier_indices: Vec<usize>,
    /// Complete per-point residual, *P* value, FDR *q* value, and exclusion flag audit trail.
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
    let sigma2 = if df > 0 { (rss / (df as f64)).max(0.0) } else { f64::NAN };
    let t_crit = if df > 0 { student_t_two_sided_isf(confidence.alpha(), df as f64).unwrap_or(f64::NAN) } else { f64::NAN };
    compute_covariance_from_scale(j_mat, sigma2, t_crit, theta)
}

/// Compute asymptotic covariance, standard errors, and symmetric normal Wald CIs for Poisson deviance
/// regression with fixed dispersion $\phi = 1$ using Fisher information $J^\top J$.
///
/// # Errors
/// Returns [`FitError::Math`] if matrix allocation or linear algebra fails.
pub(crate) fn compute_poisson_covariance_diagnostics(
    j_mat: &DenseMatrix,
    df: usize,
    theta: &[f64],
    confidence: ConfidenceLevel,
) -> Result<CovarianceDiagnostics, FitError> {
    let sigma2 = if df > 0 { 1.0 } else { f64::NAN };
    let z_crit = if df > 0 { norm_two_sided_isf(confidence.alpha()).unwrap_or(f64::NAN) } else { f64::NAN };
    compute_covariance_from_scale(j_mat, sigma2, z_crit, theta)
}

fn compute_covariance_from_scale(j_mat: &DenseMatrix, sigma2: f64, crit: f64, theta: &[f64]) -> Result<CovarianceDiagnostics, FitError> {
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

    let nrows = j_mat.nrows();
    let mut col_norms = vec![0.0_f64; p];
    let mut j_scaled = DenseMatrix::zeros(nrows, p)?;
    for (c, cn_slot) in col_norms.iter_mut().enumerate().take(p) {
        let cn = j_mat.col_norm2(c);
        *cn_slot = cn;
        if cn > 0.0 && cn.is_finite() {
            let inv_cn = 1.0 / cn;
            for r in 0..nrows {
                if let Some(slot) = j_scaled.get_mut(r, c) {
                    *slot = j_mat.get(r, c).unwrap_or(0.0) * inv_cn;
                }
            }
        }
    }

    let pqr = qr_col_pivoted(&j_scaled)?;
    let rank = pqr.rank();

    let (xtx_inv, deps, mut ambiguous) = if rank == p && nrows >= p {
        let inv_scaled = pqr.xtx_inverse()?;
        let mut inv = DenseMatrix::zeros(p, p)?;
        let mut deps = Vec::with_capacity(p);
        for i in 0..p {
            let di = col_norms[i];
            let diag_prod = inv_scaled.get(i, i).unwrap_or(1.0);
            let dep = if !diag_prod.is_finite() {
                1.0
            } else if diag_prod <= 1.0 {
                0.0
            } else {
                (1.0 - 1.0 / diag_prod).clamp(0.0, 1.0)
            };
            deps.push(dep);
            for j in 0..p {
                let dj = col_norms[j];
                if let Some(slot) = inv.get_mut(i, j) {
                    *slot = if di > 0.0 && dj > 0.0 { (inv_scaled.get(i, j).unwrap_or(0.0) / di) / dj } else { f64::NAN };
                }
            }
        }
        let amb = deps.iter().any(|&d| !d.is_finite() || d > 0.9999);
        (inv, deps, amb)
    } else {
        let sv = svd(j_mat)?;
        let inv = sv.ata_pseudoinverse(None)?;
        (inv, vec![1.0; p], true)
    };

    let mut se = Vec::with_capacity(p);
    let mut ci_lo = Vec::with_capacity(p);
    let mut ci_hi = Vec::with_capacity(p);
    let mut cov_vec = vec![vec![0.0; p]; p];
    let mut corr_vec = vec![vec![0.0; p]; p];

    for i in 0..p {
        let se_i = if sigma2.is_finite() {
            let var_i = (sigma2 * xtx_inv.get(i, i).unwrap_or(0.0)).max(0.0);
            let s = var_i.sqrt();
            if !s.is_finite() {
                ambiguous = true;
            }
            s
        } else {
            f64::NAN
        };
        se.push(se_i);
        let half = if crit.is_finite() && se_i.is_finite() { crit * se_i } else { f64::NAN };
        ci_lo.push(theta[i] - half);
        ci_hi.push(theta[i] + half);
        for (j, cell) in cov_vec[i].iter_mut().enumerate().take(p) {
            *cell = if sigma2.is_finite() { sigma2 * xtx_inv.get(i, j).unwrap_or(0.0) } else { f64::NAN };
        }
    }

    for i in 0..p {
        for j in 0..p {
            let denom = se[i] * se[j];
            corr_vec[i][j] = if !sigma2.is_finite() {
                f64::NAN
            } else if i == j {
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
    profile_solve: F,
) -> Vec<(CiBound, CiBound)>
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    compute_profile_likelihood_cis_scaled(theta_hat, ss_min, None, df, cov_diag, bounds, confidence, profile_solve)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_profile_likelihood_cis_scaled<F>(
    theta_hat: &[f64],
    ss_min: f64,
    ss_scale: Option<f64>,
    df: usize,
    cov_diag: &CovarianceDiagnostics,
    bounds: &[(Option<f64>, Option<f64>)],
    confidence: ConfidenceLevel,
    profile_solve: F,
) -> Vec<(CiBound, CiBound)>
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let p = theta_hat.len();
    if p == 0 || df == 0 || !ss_min.is_finite() {
        return vec![(CiBound::Open, CiBound::Open); p];
    }
    let is_exact_fit = if ss_min <= f64::MIN_POSITIVE * 1e6 {
        true
    } else if let Some(scale) = ss_scale.filter(|s| s.is_finite() && *s > 0.0) {
        ss_min <= (1e-28 * scale).max(f64::MIN_POSITIVE * 1e6)
    } else {
        let has_nontrivial_rel_se =
            theta_hat.iter().zip(cov_diag.standard_errors.iter()).any(|(&th, &se)| se.is_finite() && se > 1e-14 * th.abs().max(f64::MIN_POSITIVE));
        let param_sq_scale = theta_hat.iter().map(|&t| t * t).fold(0.0_f64, f64::max).max(1.0);
        !has_nontrivial_rel_se && ss_min <= 1e-28 * param_sq_scale
    };
    if is_exact_fit {
        return theta_hat.iter().map(|&t| (CiBound::Finite(t), CiBound::Finite(t))).collect();
    }
    let Ok(t_crit) = student_t_two_sided_isf(confidence.alpha(), df as f64) else {
        return vec![(CiBound::Open, CiBound::Open); p];
    };
    let f_crit = t_crit * t_crit;
    let ss_crit = ss_min * (1.0 + f_crit / (df as f64));
    trace_all_profile_bounds(theta_hat, ss_min, ss_crit, t_crit, cov_diag, bounds, profile_solve)
}

/// Trace Venzon–Moolgavkar asymmetrical profile-deviance confidence intervals for Poisson
/// deviance regression (fixed dispersion $\phi = 1$) against
/// `D_crit = D_min + chi2_{1}(alpha)`.
pub fn compute_deviance_profile_likelihood_cis<F>(
    theta_hat: &[f64],
    dev_min: f64,
    cov_diag: &CovarianceDiagnostics,
    bounds: &[(Option<f64>, Option<f64>)],
    confidence: ConfidenceLevel,
    profile_solve: F,
) -> Vec<(CiBound, CiBound)>
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let p = theta_hat.len();
    if p == 0 || !dev_min.is_finite() || dev_min < 0.0 {
        return vec![(CiBound::Open, CiBound::Open); p];
    }
    let Ok(chi2_crit) = chi2_isf(confidence.alpha(), 1.0) else {
        return vec![(CiBound::Open, CiBound::Open); p];
    };
    let z_crit = chi2_crit.sqrt();
    let dev_crit = dev_min + chi2_crit;
    trace_all_profile_bounds(theta_hat, dev_min, dev_crit, z_crit, cov_diag, bounds, profile_solve)
}

fn trace_all_profile_bounds<F>(
    theta_hat: &[f64],
    ss_min: f64,
    ss_crit: f64,
    step_crit: f64,
    cov_diag: &CovarianceDiagnostics,
    bounds: &[(Option<f64>, Option<f64>)],
    mut profile_solve: F,
) -> Vec<(CiBound, CiBound)>
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let p = theta_hat.len();
    let mut out = Vec::with_capacity(p);

    for k in 0..p {
        let se_k = cov_diag.standard_errors.get(k).copied().unwrap_or(f64::NAN);
        let base_step = if se_k.is_finite() && se_k > 0.0 { step_crit * se_k } else { 0.1 * theta_hat[k].abs().max(1e-12) };
        let (lb, ub) = bounds.get(k).copied().unwrap_or((None, None));
        let lo = trace_profile_endpoint(k, -1.0, theta_hat, ss_min, ss_crit, base_step, lb, &cov_diag.covariance_matrix, &mut profile_solve);
        let hi = trace_profile_endpoint(k, 1.0, theta_hat, ss_min, ss_crit, base_step, ub, &cov_diag.covariance_matrix, &mut profile_solve);
        out.push((lo, hi));
    }
    out
}

struct ProfileBracket {
    x_neg: f64,
    f_neg: f64,
    w_neg: Vec<f64>,
    x_pos: f64,
    f_pos: f64,
    w_pos: Vec<f64>,
}

fn refine_profile_bracket<F>(k: usize, ss_crit: f64, base_step: f64, br: &ProfileBracket, profile_solve: &mut F) -> CiBound
where
    F: FnMut(usize, f64, &[f64]) -> Option<(f64, Vec<f64>)>,
{
    let (a, b) = if br.x_neg <= br.x_pos { (br.x_neg, br.x_pos) } else { (br.x_pos, br.x_neg) };
    let span = br.x_pos - br.x_neg;
    let tol = (1e-7 * base_step).max(f64::MIN_POSITIVE);
    let opts = RootOptions { rel_tol: 1e-7, abs_tol: tol, max_iters: 48 };
    let mut interp_w = Vec::with_capacity(br.w_pos.len());
    let root = brent_root(
        |val| {
            if val == br.x_neg {
                return br.f_neg;
            }
            if val == br.x_pos {
                return br.f_pos;
            }
            let t = if span.abs() > 0.0 { ((val - br.x_neg) / span).clamp(0.0, 1.0) } else { 0.5 };
            interp_w.clear();
            for (j, (&wn, &wp)) in br.w_neg.iter().zip(br.w_pos.iter()).enumerate() {
                interp_w.push(if j == k { val } else { wn + t * (wp - wn) });
            }
            profile_solve(k, val, &interp_w).map_or(f64::NAN, |(ss_v, _)| ss_v - ss_crit)
        },
        a,
        b,
        opts,
    );
    match root {
        Ok(r) if r.is_finite() => CiBound::Finite(r),
        _ => CiBound::Open,
    }
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
    let target_rise = (ss_crit - ss_min).max(0.0);
    let mut prev_val = theta_hat[k];
    let mut prev_diff = ss_min - ss_crit;
    let mut prev_warm = theta_hat.to_vec();
    let mut warm_theta = theta_hat.to_vec();
    let cov_kk = cov.get(k).and_then(|r| r.get(k)).copied().unwrap_or(0.0);

    let min_step = if theta_hat[k] != 0.0 { 1e-12 * theta_hat[k].abs() } else { f64::MIN_POSITIVE };
    let mut step = base_step.max(min_step);
    let mut bracket: Option<ProfileBracket> = None;
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
        if (cand - prev_val).abs() <= 1e-15 * prev_val.abs().max(base_step).max(f64::MIN_POSITIVE) && hit_box {
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
        if target_rise > 0.0 && diff.abs() <= 1e-11 * target_rise {
            return CiBound::Finite(cand);
        }
        if diff >= 0.0 {
            bracket = Some(ProfileBracket { x_neg: prev_val, f_neg: prev_diff, w_neg: prev_warm, x_pos: cand, f_pos: diff, w_pos: warm_theta });
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
        prev_warm = warm_theta.clone();
        last_ss = ss_cand;
        step *= if iter < 4 { 1.8 } else { 2.5 };
    }

    let Some(br) = bracket else {
        return CiBound::Open;
    };
    refine_profile_bracket(k, ss_crit, base_step, &br, profile_solve)
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

#[derive(Clone, Copy)]
struct ReplicateRow {
    y: f64,
    n_f: f64,
    ss_within: f64,
}

fn canonical_x_bits(x: &[f64]) -> Vec<u64> {
    x.iter().map(|&v| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() }).collect()
}

fn group_pure_error_ss(rows: &[ReplicateRow]) -> Option<f64> {
    let mut within_acc = NeumaierSum::new();
    let mut n_sum_acc = NeumaierSum::new();
    let mut ny_sum_acc = NeumaierSum::new();
    for row in rows {
        within_acc.add(row.ss_within);
        n_sum_acc.add(row.n_f);
        ny_sum_acc.add(row.n_f * row.y);
    }
    let within_ss = within_acc.total();
    if rows.len() <= 1 {
        return within_ss.is_finite().then_some(within_ss.max(0.0));
    }
    let n_sum = n_sum_acc.total();
    if !n_sum.is_finite() || n_sum <= 0.0 {
        return None;
    }
    let y_bar = ny_sum_acc.total() / n_sum;
    if !y_bar.is_finite() {
        return None;
    }
    let mut between_acc = NeumaierSum::new();
    for row in rows {
        let diff = row.y - y_bar;
        between_acc.add(row.n_f * diff * diff);
    }
    let total = within_ss + between_acc.total();
    total.is_finite().then_some(total.max(0.0))
}

/// Replicates test for lack of fit (partitions residual sum of squares into pure error at
/// replicate `(data_set_index, x)` coordinates and lack-of-fit).
///
/// Pure-error / lack-of-fit partition and `F = MS_LOF / MS_PE` per Draper & Smith (1998) and
/// Motulsky & Christopoulos (2004). Groups raw, pre-summarised (`Mean/SD/N`), or mixed
/// observations by `(data_set_index, x)` so each group contributes both within-row
/// `(n_i - 1) SD_i^2` and between-row `n_i (y_i - ybar_g)^2` pure error with
/// `df_PE = sum_i n_i - n_groups`. Pure error is unweighted (`w = 1`), so `ss_res` must be
/// on the same unweighted scale (weighted / Poisson fits use weighted deviance/SS partitions).
#[must_use]
pub fn replicates_lack_of_fit(points: &[DataPoint], ss_res: f64, df_res: usize) -> Option<LackOfFitResult> {
    if points.is_empty() || df_res == 0 || !ss_res.is_finite() || ss_res < 0.0 {
        return None;
    }
    let mut groups: BTreeMap<(usize, Vec<u64>), Vec<ReplicateRow>> = BTreeMap::new();
    let mut n_total = 0_usize;

    for pt in points {
        if !pt.y.is_finite() || pt.x.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let (n_f, n_u, ss_within) = validated_replicate_moments(pt).ok()?;
        n_total = n_total.checked_add(n_u)?;
        groups.entry((pt.data_set_index, canonical_x_bits(&pt.x))).or_default().push(ReplicateRow { y: pt.y, n_f, ss_within });
    }

    let df_pe = n_total.saturating_sub(groups.len());
    if df_pe == 0 || df_res <= df_pe {
        return None;
    }

    let mut ss_pe_acc = NeumaierSum::new();
    for rows in groups.values() {
        ss_pe_acc.add(group_pure_error_ss(rows)?);
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

/// Shapiro–Wilk normality test (Royston 1992; 1995 AS R94) on residuals (`3 <= n <= 5000`).
///
/// Shapiro & Wilk (1965) `W` with Royston's (1992, 1995 AS R94) polynomial approximations for
/// `a_n` (`n >= 4`) and `a_{n-1}` (`n >= 6`), exact `a = [-1/sqrt(2), 0, 1/sqrt(2)]` and arcsine
/// p-value at `n = 3`, and Royston's normalising transformations for `4 <= n <= 5000`.
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

    let a = royston_sw_weights(n)?;
    let mut num_acc = NeumaierSum::new();
    for i in 0..(n / 2) {
        num_acc.add(a[n - 1 - i] * (sorted[n - 1 - i] - sorted[i]));
    }
    let num = num_acc.total();
    let w = ((num * num) / moments.css).clamp(0.0, 1.0);
    let p_value = royston_sw_p_value(w, n as f64);
    Some(NormalityTestResult { test_name: String::from("Shapiro-Wilk"), statistic: w, p_value, passed: p_value >= 0.05 })
}

fn royston_sw_weights(n: usize) -> Option<Vec<f64>> {
    if n == 3 {
        let s = core::f64::consts::FRAC_1_SQRT_2;
        return Some(vec![-s, 0.0, s]);
    }
    let nf = n as f64;
    let mut m = Vec::with_capacity(n);
    let mut m_ss = 0.0_f64;
    for i in 1..=n {
        let p = ((i as f64) - 0.375) / (nf + 0.25);
        let mi = norm_ppf(p).ok()?;
        m_ss += mi * mi;
        m.push(mi);
    }
    let m_norm = m_ss.sqrt();
    let u = 1.0 / nf.sqrt();
    let a_n = royston_poly_an(u) + m[n - 1] / m_norm;

    let mut a = vec![0.0_f64; n];
    a[n - 1] = a_n;
    a[0] = -a_n;

    if n <= 5 {
        let num = m_ss - 2.0 * m[n - 1] * m[n - 1];
        let den = (1.0 - 2.0 * a_n * a_n).max(1e-12);
        let phi = (num / den).sqrt();
        for i in 1..(n - 1) {
            a[i] = m[i] / phi;
        }
    } else {
        let a_n1 = royston_poly_an1(u) + m[n - 2] / m_norm;
        a[n - 2] = a_n1;
        a[1] = -a_n1;
        let num = m_ss - 2.0 * m[n - 1] * m[n - 1] - 2.0 * m[n - 2] * m[n - 2];
        let den = (1.0 - 2.0 * a_n * a_n - 2.0 * a_n1 * a_n1).max(1e-12);
        let phi = (num / den).sqrt();
        for i in 2..(n - 2) {
            a[i] = m[i] / phi;
        }
    }
    Some(a)
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
    if w >= 1.0 {
        return 1.0;
    }
    if nf == 3.0 {
        let sqrtw = w.max(0.0).sqrt();
        let pi6 = 6.0 / core::f64::consts::PI;
        let p = pi6 * (sqrtw.clamp(-1.0, 1.0).asin() - core::f64::consts::FRAC_PI_3);
        return p.clamp(0.0, 1.0);
    }
    let y = (1.0 - w).max(1e-16).ln();
    let z = if nf <= 11.0 {
        let gamma = 0.459 * nf - 2.273;
        if y >= gamma {
            return 0.0;
        }
        let mu = -0.000_671_4 * nf.powi(3) + 0.025_054 * nf.powi(2) - 0.399_78 * nf + 0.544;
        let sigma = (-0.002_032_2 * nf.powi(3) + 0.062_767 * nf.powi(2) - 0.778_57 * nf + 1.382_2).exp();
        let w_trans = -(gamma - y).ln();
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
/// Statistic `D` per Lilliefors (1967) and *p*-value via [`curvebook_math::dist::lilliefors_p_value`]
/// (Stephens 1974 `sqrt(n)` scaling + monotone PCHIP over an original seeded Monte Carlo table with
/// `1 / sqrt(n) -> 0` asymptotic extrapolation; max absolute error `0.0015` across `n >= 5`,
/// verified against Dallal & Wilkinson 1986 Table 1 and hold-out simulations including `n > 100`).
#[must_use]
pub fn lilliefors_ks_test(values: &[f64]) -> Option<NormalityTestResult> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let n = sorted.len();
    if n < 5 {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let d_max = lilliefors_d_from_sorted(&sorted).ok()?;
    let p_value = lilliefors_p_value(d_max, n).ok()?;

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
/// 3. Apply Benjamini–Hochberg (1995) step-up FDR at caller threshold `Q%` (`q_percent / 100`)
///    **only** across the top ~30% of points furthest from the fitted curve (`m_test = N - floor(0.70 N)`,
///    thresholds `Q j / N` for `j = 1..=m_test`). The remaining ~70% closest points are not tested
///    (`q_value = 1.0`, `is_outlier = false`), matching Motulsky & Brown (2006).
///
/// # Errors
/// Returns [`FitError::InsufficientData`] if `points.len() <= k_free`, or
/// [`FitError::InvalidParameter`] if `residuals` length/finiteness or `q_percent` is invalid.
pub fn rout_detect_outliers(points: &[DataPoint], residuals: &[f64], k_free: usize, q_percent: f64) -> Result<RoutDetectionResult, FitError> {
    let n = points.len();
    if n <= k_free {
        return Err(FitError::InsufficientData { required: k_free + 1, actual: n });
    }
    if residuals.len() != n {
        return Err(FitError::InvalidParameter { name: "residuals", reason: String::from("length mismatch") });
    }
    if !q_percent.is_finite() || !(0.0..=100.0).contains(&q_percent) {
        return Err(FitError::InvalidParameter { name: "q_percent", reason: format!("FDR Q% must lie in [0, 100], got {q_percent}") });
    }
    if residuals.iter().any(|r| !r.is_finite()) {
        return Err(FitError::InvalidParameter { name: "residuals", reason: String::from("ROUT residuals must all be finite") });
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
    order.sort_by(|&a, &b| p_vals[a].total_cmp(&p_vals[b]).then_with(|| abs_res[b].total_cmp(&abs_res[a])).then_with(|| a.cmp(&b)));

    // Motulsky & Brown (2006): test only the top ~30% furthest points (`m_test = N - floor(0.70 * N)`).
    let start = ((0.70 * (n as f64)).floor() as usize).min(n - 1);
    let m_test = n - start;

    let mut bh_q = vec![1.0_f64; n];
    let mut running_min = 1.0_f64;
    for rank_rev in (0..m_test).rev() {
        let idx = order[rank_rev];
        let rank_1 = (rank_rev + 1) as f64;
        let raw_q = (p_vals[idx] * (n as f64) / rank_1).min(1.0);
        running_min = running_min.min(raw_q);
        bh_q[idx] = running_min;
    }

    let mut max_reject_rank: Option<usize> = None;
    for (rank0, &idx) in order.iter().take(m_test).enumerate() {
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
            source_subcolumn: points[i].source_subcolumn,
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
            DataPoint {
                x: vec![1.0],
                y: 2.0,
                sd: Some(0.5),
                n: Some(f64::INFINITY),
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row: 0,
            },
            DataPoint {
                x: vec![2.0],
                y: 3.0,
                sd: Some(0.5),
                n: Some(f64::INFINITY),
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row: 1,
            },
        ];
        assert!(replicates_lack_of_fit(&inf_pts, 10.0, 4).is_none());

        let huge_pts = vec![
            DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(1e10), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 3.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 1 },
        ];
        assert!(replicates_lack_of_fit(&huge_pts, 10.0, 4).is_none());

        let valid_pts = vec![
            DataPoint { x: vec![1.0], y: 2.0, sd: Some(0.5), n: Some(1.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 },
            DataPoint { x: vec![2.0], y: 3.0, sd: Some(0.5), n: Some(3.0), explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 1 },
        ];
        let res = replicates_lack_of_fit(&valid_pts, 5.0, 4).unwrap();
        assert_eq!(res.df_pure_error, 2);
        assert_eq!(res.df_lack_of_fit, 2);
    }
}
