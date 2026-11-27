//! Generalized Linear Model extensions: Weighted Least Squares (WLS), row-level influence
//! diagnostics (hat-matrix leverage, standardized/studentized/Pearson/deviance residuals, Cook's
//! distance, DFBETAs), quasi-Poisson dispersion and Huber–White HC sandwich covariance,
//! Venzon–Moolgavkar profile-likelihood confidence intervals, Hosmer–Lemeshow calibration bins,
//! and out-of-sample prediction intervals.
//!
//! # References
//!
//! - Golub, G. (1965). Numerical methods for solving linear least squares problems. *Numerische
//!   Mathematik*, 7(3), 206–216. `doi:10.1007/bf01436075`.
//! - Björck, Å. (1967). Iterative refinement of linear least squares solutions I. *BIT*, 7(4),
//!   257–278. `doi:10.1007/bf01939321`.
//! - Cook, R. D. (1977). Detection of influential observation in linear regression.
//!   *Technometrics*, 19(1), 15–18. `doi:10.1080/00401706.1977.10489493`.
//! - Belsley, D. A., Kuh, E., & Welsch, R. E. (1980). *Regression Diagnostics: Identifying
//!   Influential Data and Sources of Collinearity*. Wiley.
//! - White, H. (1980). A heteroskedasticity-consistent covariance matrix estimator and a direct
//!   test for heteroskedasticity. *Econometrica*, 48(4), 817–838. `doi:10.2307/1912934`.
//! - Pregibon, D. (1981). Logistic regression diagnostics. *The Annals of Statistics*, 9(4),
//!   705–724. `doi:10.1214/aos/1176345513`.
//! - MacKinnon, J. G., & White, H. (1985). Some heteroskedasticity-consistent covariance matrix
//!   estimators with improved finite sample properties. *Journal of Econometrics*, 29(3), 305–325.
//!   `doi:10.1016/0304-4076(85)90158-7`.
//! - Venzon, D. J., & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based
//!   confidence intervals. *Journal of the Royal Statistical Society, Series C (Applied
//!   Statistics)*, 37(1), 87–94. `doi:10.2307/2347496`.
//! - McCullagh, P., & Nelder, J. A. (1989). *Generalized Linear Models* (2nd ed.). Chapman & Hall.
//! - Hurvich, C. M., & Tsai, C.-L. (1989). Regression and time series model selection in small
//!   samples. *Biometrika*, 76(2), 297–307. `doi:10.1093/biomet/76.2.297`.
//! - Hosmer, D. W., & Lemeshow, S. (2000). *Applied Logistic Regression* (2nd ed.). Wiley.
//! - Burnham, K. P., & Anderson, D. R. (2002). *Model Selection and Multimodel Inference* (2nd
//!   ed.). Springer.

use curvebook_doc::{GlmDispersionMode, GlmWeightSpec, WeightingScheme};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{f_isf, f_sf, norm_two_sided_isf, student_t_two_sided_isf, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares};
use curvebook_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::common::validate_finite_slice;
use crate::error::StatsError;
use crate::regression::{LinearTermEstimate, MultipleLinearResult};

/// Full row-level linear regression (OLS / WLS) influence and residual diagnostics
/// (Cook 1977; Belsley, Kuh & Welsch 1980).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearRowDiagnostics {
    /// Hat-matrix diagonal leverage `h_ii = w_i x_i^T (X^T W X)^{-1} x_i in [0, 1]`.
    pub leverage: Vec<f64>,
    /// Raw residuals `y_i - yhat_i`.
    pub raw_residuals: Vec<f64>,
    /// Internally standardized residuals `sqrt(w_i) r_i / (sigma * sqrt(1 - h_ii))`.
    pub standardized_residuals: Vec<f64>,
    /// Externally studentized (deleted) residuals `sqrt(w_i) r_i / (sigma_{(-i)} * sqrt(1 - h_ii))`.
    pub studentized_residuals: Vec<f64>,
    /// Cook's distance `D_i`.
    pub cooks_distance: Vec<f64>,
    /// Per-observation DFBETAs (`n x p`), measuring standardized leave-one-out coefficient change.
    pub dfbetas: Vec<Vec<f64>>,
}

/// Complete Weighted / Ordinary Multiple Linear Regression result with full diagnostics and
/// parameter correlation matrix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGlmExtendedResult {
    /// Core multiple linear regression estimates, goodness-of-fit, and covariance matrix.
    pub core: MultipleLinearResult,
    /// Full row-level diagnostics including standardized/studentized residuals and DFBETAs.
    pub diagnostics: LinearRowDiagnostics,
    /// Parameter correlation matrix (`p x p`) derived from `core.cov_beta`.
    pub correlation_matrix: Vec<Vec<f64>>,
    /// Observation weights applied (`vec![1.0; n]` when unweighted).
    pub weights: Vec<f64>,
    /// Dispersion / covariance mode used (`Canonical` or `RobustSandwich`).
    pub dispersion_mode: GlmDispersionMode,
}

/// Row-level GLM influence diagnostics for Logistic and Poisson models (Pregibon 1981).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlmRowDiagnostics {
    /// Hat-matrix diagonal leverage `h_ii = w_i x_i^T (X^T W X)^{-1} x_i`.
    pub leverage: Vec<f64>,
    /// Pearson residuals `(y_i - mu_i) / sqrt(V(mu_i))`.
    pub pearson_residuals: Vec<f64>,
    /// Signed deviance residuals.
    pub deviance_residuals: Vec<f64>,
    /// Standardized Pearson residuals `r_P / sqrt(1 - h_ii)`.
    pub standardized_pearson: Vec<f64>,
    /// Standardized deviance residuals `r_D / sqrt(1 - h_ii)`.
    pub standardized_deviance: Vec<f64>,
    /// Cook's distance `D_i = (r_P^2 / (phi * p)) * h_ii / (1 - h_ii)^2`.
    pub cooks_distance: Vec<f64>,
    /// One-step Pregibon / `lm.influence` DFBETAs (`n x p`).
    pub dfbetas: Vec<Vec<f64>>,
}

/// Out-of-sample prediction row for Linear GLM (with CI and PI).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearOosPrediction {
    /// Point prediction `x_0^T beta`.
    pub fit: f64,
    /// Standard error of the fitted mean `sqrt(x_0^T Cov x_0)`.
    pub se_fit: f64,
    /// Confidence interval for the mean response `E[Y | x_0]`.
    pub ci: (f64, f64),
    /// Prediction interval for a new observation `Y_new | x_0` (at unit observation weight).
    pub pi: (f64, f64),
}

/// Out-of-sample prediction row for Logistic GLM.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogisticOosPrediction {
    /// Linear predictor `eta = x_0^T beta`.
    pub eta: f64,
    /// Standard error of the linear predictor `sqrt(x_0^T Cov x_0)`.
    pub se_eta: f64,
    /// Predicted event probability `logistic(eta) in (0, 1)`.
    pub probability: f64,
    /// Confidence interval for the predicted probability mapped through `logistic`.
    pub prob_ci: (f64, f64),
    /// Predicted binary class (`1.0` if `probability >= cutoff`, else `0.0`).
    pub predicted_class: f64,
}

/// Out-of-sample prediction row for Poisson GLM.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PoissonOosPrediction {
    /// Log-link linear predictor `eta = x_0^T beta`.
    pub eta: f64,
    /// Standard error of the linear predictor `sqrt(x_0^T Cov x_0)`.
    pub se_eta: f64,
    /// Predicted expected count / rate `exp(eta)`.
    pub rate: f64,
    /// Confidence interval for the expected count `exp(eta +/- crit * se_eta)`.
    pub rate_ci: (f64, f64),
}

/// Single Hosmer–Lemeshow calibration bin summarizing observed vs expected events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HosmerLemeshowBin {
    /// Bin label (e.g. `"Decile 1"`).
    pub label: String,
    /// Number of observations in this bin.
    pub count: usize,
    /// Observed count of events (`sum y_i`).
    pub observed_events: f64,
    /// Expected count of events (`sum p_hat_i`).
    pub expected_events: f64,
    /// Observed event proportion `observed_events / count`.
    pub observed_rate: f64,
    /// Expected event proportion `expected_events / count`.
    pub expected_rate: f64,
}

/// Derive a symmetric `p x p` parameter correlation matrix from a covariance matrix (`Vec<Vec<f64>>`).
#[must_use]
pub fn cov_to_correlation_rows(cov_beta: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let p = cov_beta.len();
    let mut corr = vec![vec![f64::NAN; p]; p];
    let inv_sds: Vec<f64> = (0..p)
        .map(|j| {
            let v = cov_beta[j].get(j).copied().unwrap_or(0.0);
            if v.is_finite() && v > 0.0 { 1.0 / v.sqrt() } else { 0.0 }
        })
        .collect();
    for j in 0..p {
        for k in 0..p {
            if j == k {
                corr[j][k] = if inv_sds[j] > 0.0 { 1.0 } else { f64::NAN };
            } else if inv_sds[j] > 0.0 && inv_sds[k] > 0.0 {
                let c = cov_beta[j].get(k).copied().unwrap_or(0.0);
                corr[j][k] = (c * inv_sds[j] * inv_sds[k]).clamp(-1.0, 1.0);
            }
        }
    }
    corr
}

/// Derive a symmetric `p x p` parameter correlation matrix from a [`DenseMatrix`] covariance.
#[must_use]
pub fn dense_cov_to_correlation_rows(cov: &DenseMatrix) -> Vec<Vec<f64>> {
    let p = cov.nrows();
    let rows: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| cov.get(i, j).unwrap_or(0.0)).collect()).collect();
    cov_to_correlation_rows(&rows)
}

/// Compute analytical or explicit positive observation weights for a linear GLM.
pub fn resolve_glm_weights(
    weight_spec: GlmWeightSpec,
    y_vec: &[f64],
    first_continuous_x: Option<&[f64]>,
    explicit_weights: Option<&[f64]>,
) -> Result<Option<Vec<f64>>, StatsError> {
    let n = y_vec.len();
    match weight_spec {
        GlmWeightSpec::None | GlmWeightSpec::Scheme(WeightingScheme::None) => Ok(None),
        GlmWeightSpec::Column(_) => {
            let w = explicit_weights.ok_or(StatsError::EmptyInput)?;
            if w.len() != n {
                return Err(StatsError::DimensionMismatch { expected: n, actual: w.len() });
            }
            for &wi in w {
                if !wi.is_finite() || wi <= 0.0 {
                    return Err(StatsError::InvalidParameter { name: "glm_weight", value: wi });
                }
            }
            Ok(Some(w.to_vec()))
        }
        GlmWeightSpec::Scheme(scheme) => {
            let mut out = Vec::with_capacity(n);
            match scheme {
                WeightingScheme::None => return Ok(None),
                WeightingScheme::RelativeSquare => {
                    for &yi in y_vec {
                        if !yi.is_finite() || yi == 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_y2", value: yi });
                        }
                        let w = 1.0 / (yi * yi);
                        if !w.is_finite() || w <= 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_y2", value: w });
                        }
                        out.push(w);
                    }
                }
                WeightingScheme::Relative => {
                    for &yi in y_vec {
                        if !yi.is_finite() || yi == 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_abs_y", value: yi });
                        }
                        let w = 1.0 / yi.abs();
                        if !w.is_finite() || w <= 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_abs_y", value: w });
                        }
                        out.push(w);
                    }
                }
                WeightingScheme::GeneralPowerY(k) => {
                    if !k.is_finite() {
                        return Err(StatsError::InvalidParameter { name: "weight_power_k", value: k });
                    }
                    for &yi in y_vec {
                        if !yi.is_finite() || (yi == 0.0 && k > 0.0) {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_abs_y_k", value: yi });
                        }
                        let w = if k == 0.0 { 1.0 } else { yi.abs().powf(-k) };
                        if !w.is_finite() || w <= 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_1_over_abs_y_k", value: w });
                        }
                        out.push(w);
                    }
                }
                WeightingScheme::InverseX | WeightingScheme::InverseX2 => {
                    let x0 = first_continuous_x.ok_or(StatsError::IncompatibleAnalysisSpec {
                        expected: "at least one continuous predictor column for 1/X or 1/X^2 GLM weighting",
                    })?;
                    if x0.len() != n {
                        return Err(StatsError::DimensionMismatch { expected: n, actual: x0.len() });
                    }
                    let sq = matches!(scheme, WeightingScheme::InverseX2);
                    for &xi in x0 {
                        if !xi.is_finite() || xi == 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_inverse_x", value: xi });
                        }
                        let w = if sq { 1.0 / (xi * xi) } else { 1.0 / xi.abs() };
                        if !w.is_finite() || w <= 0.0 {
                            return Err(StatsError::InvalidParameter { name: "weight_inverse_x", value: w });
                        }
                        out.push(w);
                    }
                }
                WeightingScheme::ObservedSd => {
                    return Err(StatsError::IncompatibleAnalysisSpec {
                        expected: "explicit weight column or analytical 1/Y^2, 1/|Y|, 1/|Y|^k, 1/|X|, 1/X^2 weighting for multivariable GLM",
                    });
                }
            }
            Ok(Some(out))
        }
    }
}

/// Compute Huber–White heteroscedasticity-consistent sandwich covariance `scale * B * M * B`
/// where `B = (X^T W X)^{-1}` and `M = sum_i u_i^2 x_i x_i^T` (White 1980; MacKinnon & White 1985).
pub fn compute_glm_sandwich_cov(
    x: &DenseMatrix,
    score_residuals: &[f64],
    bread: &DenseMatrix,
    finite_sample_scale: f64,
) -> Result<DenseMatrix, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if score_residuals.len() != n || bread.nrows() != p || bread.ncols() != p {
        return Err(StatsError::DimensionMismatch { expected: n, actual: score_residuals.len() });
    }
    let mut meat = vec![0.0_f64; p * p];
    for (r, &u) in score_residuals.iter().enumerate() {
        let u2 = u * u;
        if !u2.is_finite() || u2 == 0.0 {
            continue;
        }
        for j in 0..p {
            let xrj = x.get(r, j).unwrap_or(0.0);
            for k in j..p {
                meat[j * p + k] += u2 * xrj * x.get(r, k).unwrap_or(0.0);
            }
        }
    }
    for j in 0..p {
        for k in 0..j {
            meat[j * p + k] = meat[k * p + j];
        }
    }
    let meat_mat = DenseMatrix::from_row_major(p, p, meat)?;
    let bm = bread.matmul(&meat_mat)?;
    let bmb = bm.matmul(bread)?;
    let mut out = vec![0.0_f64; p * p];
    for j in 0..p {
        for k in j..p {
            let val = 0.5 * finite_sample_scale * (bmb.get(j, k).unwrap_or(0.0) + bmb.get(k, j).unwrap_or(0.0));
            out[j * p + k] = val;
            out[k * p + j] = val;
        }
    }
    Ok(DenseMatrix::from_row_major(p, p, out)?)
}

fn compute_vifs_weighted(x: &DenseMatrix, weights: &[f64], has_intercept: bool, aliased: &[bool]) -> Vec<Option<f64>> {
    let n = x.nrows();
    let p = x.ncols();
    let mut vifs = vec![None; p];
    let start = usize::from(has_intercept);
    let active_preds = (start..p).filter(|&c| !aliased.get(c).copied().unwrap_or(false)).count();
    if active_preds <= 1 {
        return vifs;
    }
    let sw: Vec<f64> = weights.iter().map(|&w| w.sqrt()).collect();
    let w_sum: f64 = weights.iter().sum();
    if !w_sum.is_finite() || w_sum <= 0.0 {
        return vifs;
    }
    for (j, slot) in vifs.iter_mut().enumerate().skip(start) {
        if aliased.get(j).copied().unwrap_or(false) {
            continue;
        }
        let retained_cols: Vec<usize> = (0..p).filter(|&c| c != j && !aliased.get(c).copied().unwrap_or(false)).collect();
        if retained_cols.len() <= start {
            continue;
        }
        let extra_intercept = usize::from(!has_intercept);
        let xm_ncols = retained_cols.len() + extra_intercept;
        let mut target_w = Vec::with_capacity(n);
        let mut mean_j = 0.0_f64;
        for (r, &wr) in weights.iter().enumerate() {
            mean_j += wr * x.get(r, j).unwrap_or(0.0);
        }
        mean_j /= w_sum;
        let mut css_w = 0.0_f64;
        let mut pred_cols = Vec::with_capacity(n * xm_ncols);
        for r in 0..n {
            let xrj = x.get(r, j).unwrap_or(0.0);
            css_w += weights[r] * (xrj - mean_j).powi(2);
            target_w.push(sw[r] * xrj);
            if !has_intercept {
                pred_cols.push(sw[r]);
            }
            for &c in &retained_cols {
                pred_cols.push(sw[r] * x.get(r, c).unwrap_or(0.0));
            }
        }
        if css_w <= 0.0 {
            continue;
        }
        let Ok(xm) = DenseMatrix::from_row_major(n, xm_ncols, pred_cols) else { continue };
        if let Ok((sol, _)) = solve_linear_least_squares(&xm, &target_w) {
            let r2 = (1.0 - sol.rss / css_w).clamp(0.0, 0.999_999_999);
            *slot = Some(1.0 / (1.0 - r2));
        }
    }
    vifs
}

fn drop_col_from_dense(x: &DenseMatrix, drop_col: usize) -> Result<DenseMatrix, StatsError> {
    let nr = x.nrows();
    let nc = x.ncols();
    let mut data = Vec::with_capacity(nr * nc.saturating_sub(1));
    for r in 0..nr {
        for c in 0..nc {
            if c != drop_col {
                data.push(x.get(r, c).unwrap_or(0.0));
            }
        }
    }
    Ok(DenseMatrix::from_row_major(nr, nc.saturating_sub(1), data)?)
}

struct LinearDiagScalars {
    rss: f64,
    mse: f64,
    rank: usize,
    df_resid_f: f64,
}

fn compute_linear_row_diagnostics(
    x: &DenseMatrix,
    residuals: &[f64],
    weights: &[f64],
    cov_unscaled: &DenseMatrix,
    aliased_flags: &[bool],
    scalars: LinearDiagScalars,
) -> LinearRowDiagnostics {
    let LinearDiagScalars { rss, mse, rank, df_resid_f } = scalars;
    let n = x.nrows();
    let p = x.ncols();
    let sigma = mse.sqrt();
    let p_eff = (rank as f64).max(1.0);
    let mut leverage = Vec::with_capacity(n);
    let mut std_res = Vec::with_capacity(n);
    let mut stud_res = Vec::with_capacity(n);
    let mut cooks = Vec::with_capacity(n);
    let mut dfbetas = Vec::with_capacity(n);
    let cov_diag_sqrt: Vec<f64> = (0..p)
        .map(|j| {
            let v = cov_unscaled.get(j, j).unwrap_or(0.0);
            if !aliased_flags[j] && v > 0.0 { v.sqrt() } else { 0.0 }
        })
        .collect();

    for (r, (&res, &wr)) in residuals.iter().zip(weights.iter()).enumerate() {
        let mut cx = vec![0.0_f64; p];
        for j in 0..p {
            let mut acc = 0.0_f64;
            for k in 0..p {
                acc += cov_unscaled.get(j, k).unwrap_or(0.0) * x.get(r, k).unwrap_or(0.0);
            }
            cx[j] = acc;
        }
        let mut quad = 0.0_f64;
        for (j, &cxj) in cx.iter().enumerate() {
            quad += x.get(r, j).unwrap_or(0.0) * cxj;
        }
        let h_raw = wr * quad;
        let hc = h_raw.clamp(0.0, 0.999_999);
        let one_minus_h = (1.0 - hc).max(1e-12);
        leverage.push(hc);

        let r_wt = wr.sqrt() * res;
        let r_std = if sigma > 0.0 { r_wt / (sigma * one_minus_h.sqrt()) } else { 0.0 };
        std_res.push(r_std);

        let sigma_del = if df_resid_f > 1.0 {
            let rss_del = (rss - (r_wt * r_wt) / one_minus_h).max(0.0);
            (rss_del / (df_resid_f - 1.0)).sqrt()
        } else {
            f64::NAN
        };
        let r_stud = if sigma_del.is_finite() && sigma_del > 0.0 { r_wt / (sigma_del * one_minus_h.sqrt()) } else { f64::NAN };
        stud_res.push(r_stud);

        let cook_denom = p_eff * mse * one_minus_h * one_minus_h;
        cooks.push(if cook_denom > 0.0 { (r_wt * r_wt * hc) / cook_denom } else { 0.0 });

        let mut dfb_row = vec![f64::NAN; p];
        if sigma_del.is_finite() && sigma_del > 0.0 {
            let factor = (wr * res) / (one_minus_h * sigma_del);
            for j in 0..p {
                if cov_diag_sqrt[j] > 0.0 {
                    dfb_row[j] = (factor * cx[j]) / cov_diag_sqrt[j];
                }
            }
        }
        dfbetas.push(dfb_row);
    }

    LinearRowDiagnostics {
        leverage,
        raw_residuals: residuals.to_vec(),
        standardized_residuals: std_res,
        studentized_residuals: stud_res,
        cooks_distance: cooks,
        dfbetas,
    }
}

struct LinearTermsContext {
    has_intercept: bool,
    rank: usize,
    rss: f64,
    mse: f64,
    df_resid_f: f64,
    t_crit: f64,
    exact_fit: bool,
}

#[allow(clippy::too_many_arguments)]
fn build_weighted_linear_terms(
    xw: &DenseMatrix,
    yw: &[f64],
    coefficients: &[f64],
    cov_beta_mat: &DenseMatrix,
    term_names: &[String],
    aliased_flags: &[bool],
    vifs: &[Option<f64>],
    ctx: LinearTermsContext,
) -> Result<Vec<LinearTermEstimate>, StatsError> {
    let p = term_names.len();
    let mut terms = Vec::with_capacity(p);
    for j in 0..p {
        if aliased_flags[j] {
            terms.push(LinearTermEstimate {
                name: term_names[j].clone(),
                estimate: f64::NAN,
                se: f64::NAN,
                t_statistic: f64::NAN,
                p_value: f64::NAN,
                ci: (f64::NAN, f64::NAN),
                vif: None,
                type_iii_ss: None,
                type_iii_f: None,
                type_iii_p: None,
            });
            continue;
        }
        let est = coefficients[j];
        let raw_se = cov_beta_mat.get(j, j).unwrap_or(0.0).max(0.0).sqrt();
        let se = if ctx.exact_fit && ctx.rss == 0.0 { 0.0 } else { raw_se };
        let (t_stat, p_val, ci) = if se > 0.0 && !ctx.exact_fit {
            let t_v = est / se;
            let p_v = student_t_two_sided_p(t_v, ctx.df_resid_f)?;
            (t_v, p_v, (est - ctx.t_crit * se, est + ctx.t_crit * se))
        } else {
            (f64::NAN, f64::NAN, (f64::NAN, f64::NAN))
        };

        let (ss_iii, f_iii, p_iii) = if p > 1 && !(ctx.has_intercept && j == 0) {
            let xw_red = drop_col_from_dense(xw, j)?;
            if let Ok((red_sol, _)) = solve_linear_least_squares(&xw_red, yw) {
                if red_sol.rank >= ctx.rank {
                    (None, None, None)
                } else {
                    let ss_drop = (red_sol.rss - ctx.rss).max(0.0);
                    if ctx.mse > 0.0 && !ctx.exact_fit {
                        let f_drop = ss_drop / ctx.mse;
                        let p_drop = f_sf(f_drop, 1.0, ctx.df_resid_f).unwrap_or(1.0);
                        (Some(ss_drop), Some(f_drop), Some(p_drop))
                    } else {
                        (Some(ss_drop), None, None)
                    }
                }
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        terms.push(LinearTermEstimate {
            name: term_names[j].clone(),
            estimate: est,
            se,
            t_statistic: t_stat,
            p_value: p_val,
            ci,
            vif: vifs[j],
            type_iii_ss: ss_iii,
            type_iii_f: f_iii,
            type_iii_p: p_iii,
        });
    }
    Ok(terms)
}

/// Fit Ordinary or Weighted Multiple Linear Regression (`WLS`) with full row diagnostics
/// (standardized/studentized residuals, Cook's D, DFBETAs), optional Huber–White HC1 sandwich
/// covariance, exact Gaussian log-likelihood AIC/AICc/BIC (including `-sum ln(w_i)`), and
/// parameter correlation matrix (Golub 1965; Cook 1977; Belsley, Kuh & Welsch 1980; White 1980).
#[allow(clippy::too_many_arguments)]
pub fn multiple_linear_regression_weighted(
    x: &DenseMatrix,
    y: &[f64],
    weights: Option<&[f64]>,
    term_names: &[String],
    has_intercept: bool,
    compute_vif: bool,
    dispersion_mode: GlmDispersionMode,
    confidence: ConfidenceLevel,
) -> Result<LinearGlmExtendedResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if dispersion_mode == GlmDispersionMode::QuasiPoisson {
        return Err(StatsError::IncompatibleAnalysisSpec { expected: "Poisson GLM family for QuasiPoisson dispersion mode" });
    }
    if y.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: y.len() });
    }
    if term_names.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: term_names.len() });
    }
    if p == 0 || n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "multiple linear regression", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;
    let w_vec: Vec<f64> = match weights {
        Some(w) => {
            if w.len() != n {
                return Err(StatsError::DimensionMismatch { expected: n, actual: w.len() });
            }
            for &wi in w {
                if !wi.is_finite() || wi <= 0.0 {
                    return Err(StatsError::InvalidParameter { name: "glm_weight", value: wi });
                }
            }
            w.to_vec()
        }
        None => vec![1.0_f64; n],
    };

    let mut xw_data = Vec::with_capacity(n * p);
    let mut yw = Vec::with_capacity(n);
    let mut sum_ln_w = NeumaierSum::new();
    let mut sum_w = NeumaierSum::new();
    for (r, (&yr, &wr)) in y.iter().zip(w_vec.iter()).enumerate() {
        let sw = wr.sqrt();
        yw.push(sw * yr);
        sum_ln_w.add(wr.ln());
        sum_w.add(wr);
        for c in 0..p {
            xw_data.push(sw * x.get(r, c).unwrap_or(0.0));
        }
    }
    let xw = DenseMatrix::from_row_major(n, p, xw_data)?;
    let (sol, cov_unscaled) = solve_linear_least_squares(&xw, &yw)?;
    let rank = sol.rank;
    let df_resid = n.saturating_sub(rank);
    if df_resid == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "linear regression residual df", required: rank + 1, actual: n });
    }
    let df_resid_f = df_resid as f64;

    // Fitted values, raw residuals, weighted RSS, and Durbin-Watson on weighted residuals.
    let mut fitted = Vec::with_capacity(n);
    let mut residuals = Vec::with_capacity(n);
    let mut wrss_acc = NeumaierSum::new();
    let mut max_term_scale = 0.0_f64;
    for (r, (&yr, &wr)) in y.iter().zip(w_vec.iter()).enumerate() {
        max_term_scale = max_term_scale.max(yr.abs());
        let mut main = NeumaierSum::new();
        let mut err = NeumaierSum::new();
        main.add(yr);
        for (c, &b) in sol.coefficients.iter().enumerate() {
            let xrc = x.get(r, c).unwrap_or(0.0);
            let neg_x = -xrc;
            let prod = neg_x * b;
            max_term_scale = max_term_scale.max(prod.abs());
            main.add(prod);
            err.add(neg_x.mul_add(b, -prod));
        }
        let res_r = main.total() + err.total();
        fitted.push(yr - res_r);
        residuals.push(res_r);
        wrss_acc.add(wr * res_r * res_r);
    }
    let rss = wrss_acc.total().max(0.0);
    let mse = rss / df_resid_f;
    let residual_sd = mse.sqrt();

    let mut dw_num_acc = NeumaierSum::new();
    let mut dw_den_acc = NeumaierSum::new();
    for i in 0..n {
        let ri_w = w_vec[i].sqrt() * residuals[i];
        dw_den_acc.add(ri_w * ri_w);
        if i > 0 {
            let d = ri_w - w_vec[i - 1].sqrt() * residuals[i - 1];
            dw_num_acc.add(d * d);
        }
    }
    let exact_fit_floor = (n as f64) * (64.0 * f64::EPSILON * max_term_scale).powi(2);
    let durbin_watson = if dw_den_acc.total() > exact_fit_floor { (dw_num_acc.total().max(0.0) / dw_den_acc.total()).clamp(0.0, 4.0) } else { 2.0 };

    let ss_total = if has_intercept {
        let mut wy_sum = NeumaierSum::new();
        for (&yr, &wr) in y.iter().zip(w_vec.iter()) {
            wy_sum.add(wr * yr);
        }
        let ybar_w = wy_sum.total() / sum_w.total();
        let mut tss = NeumaierSum::new();
        for (&yr, &wr) in y.iter().zip(w_vec.iter()) {
            tss.add(wr * (yr - ybar_w).powi(2));
        }
        tss.total()
    } else {
        let mut s2 = NeumaierSum::new();
        for (&yr, &wr) in y.iter().zip(w_vec.iter()) {
            s2.add(wr * yr * yr);
        }
        s2.total()
    };
    let ss_reg = (ss_total - rss).max(0.0);
    let df_model_f = if has_intercept { rank.saturating_sub(1) as f64 } else { rank as f64 };
    let r_squared = if ss_total > 0.0 { (1.0 - rss / ss_total).clamp(0.0, 1.0) } else { 0.0 };
    let adj_r2 = if ss_total > 0.0 && n > rank {
        let denom_df = if has_intercept { (n - 1) as f64 } else { n as f64 };
        let ms_tot = if denom_df > 0.0 { ss_total / denom_df } else { 0.0 };
        if ms_tot > 0.0 { 1.0 - (rss / df_resid_f) / ms_tot } else { r_squared }
    } else {
        r_squared
    };

    let (f_stat, f_p_val) = if df_model_f > 0.0 && mse > 0.0 {
        let f = (ss_reg / df_model_f) / mse;
        (f, f_sf(f, df_model_f, df_resid_f)?)
    } else if df_model_f > 0.0 && ss_reg > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };

    let mut aliased_flags = vec![false; p];
    for &idx in &sol.aliased_columns {
        if idx < p {
            aliased_flags[idx] = true;
        }
    }

    let cov_beta_mat: DenseMatrix = if dispersion_mode == GlmDispersionMode::RobustSandwich {
        let score_res: Vec<f64> = residuals.iter().zip(w_vec.iter()).map(|(&r, &w)| w * r).collect();
        let hc1 = (n as f64) / df_resid_f.max(1.0);
        compute_glm_sandwich_cov(x, &score_res, &cov_unscaled, hc1)?
    } else {
        let mut data = vec![0.0_f64; p * p];
        for j in 0..p {
            for k in 0..p {
                if !aliased_flags[j] && !aliased_flags[k] {
                    data[j * p + k] = mse * cov_unscaled.get(j, k).unwrap_or(0.0);
                }
            }
        }
        DenseMatrix::from_row_major(p, p, data)?
    };

    let diagnostics =
        compute_linear_row_diagnostics(x, &residuals, &w_vec, &cov_unscaled, &aliased_flags, LinearDiagScalars { rss, mse, rank, df_resid_f });
    let vifs = if compute_vif { compute_vifs_weighted(x, &w_vec, has_intercept, &aliased_flags) } else { vec![None; p] };
    let t_crit = student_t_two_sided_isf(confidence.alpha(), df_resid_f)?;
    let exact_fit = mse <= 0.0 || rss <= exact_fit_floor;

    let terms = build_weighted_linear_terms(
        &xw,
        &yw,
        &sol.coefficients,
        &cov_beta_mat,
        term_names,
        &aliased_flags,
        &vifs,
        LinearTermsContext { has_intercept, rank, rss, mse, df_resid_f, t_crit, exact_fit },
    )?;

    // Exact Gaussian log-likelihood AIC/AICc/BIC including -sum ln(w_i) matching R stats::AIC(lm).
    let nf = n as f64;
    let k_ic = (rank + 1) as f64;
    let neg2_ll = nf * (rss / nf).max(1e-300).ln() + nf + nf * (2.0 * std::f64::consts::PI).ln() - sum_ln_w.total();
    let aic = neg2_ll + 2.0 * k_ic;
    let aicc = if nf - k_ic - 1.0 > 0.0 { aic + (2.0 * k_ic * (k_ic + 1.0)) / (nf - k_ic - 1.0) } else { f64::INFINITY };
    let bic = neg2_ll + k_ic * nf.ln();

    let cov_beta: Vec<Vec<f64>> = (0..p)
        .map(|j| (0..p).map(|k| if aliased_flags[j] || aliased_flags[k] { 0.0 } else { cov_beta_mat.get(j, k).unwrap_or(0.0) }).collect())
        .collect();
    let correlation_matrix = cov_to_correlation_rows(&cov_beta);

    let core = MultipleLinearResult {
        n,
        p,
        rank,
        has_intercept,
        aliased_columns: aliased_flags,
        terms,
        r_squared,
        adjusted_r_squared: adj_r2,
        rss,
        ss_regression: ss_reg,
        ss_total,
        residual_sd,
        df_model: df_model_f,
        df_residual: df_resid_f,
        f_statistic: f_stat,
        f_p_value: f_p_val,
        aic,
        aicc,
        bic,
        durbin_watson,
        fitted,
        residuals,
        leverage: diagnostics.leverage.clone(),
        cooks_distance: diagnostics.cooks_distance.clone(),
        cov_beta,
    };
    Ok(LinearGlmExtendedResult { core, diagnostics, correlation_matrix, weights: w_vec, dispersion_mode })
}

/// Compute Pregibon (1981) / R `influence.glm` row-level diagnostics for a fitted Logistic GLM.
pub fn compute_logistic_diagnostics(x: &DenseMatrix, y: &[f64], probs: &[f64], cov_canonical: &DenseMatrix) -> Result<GlmRowDiagnostics, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: y.len() });
    }
    if probs.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: probs.len() });
    }
    if cov_canonical.nrows() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_canonical.nrows() });
    }
    if cov_canonical.ncols() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_canonical.ncols() });
    }
    let mut leverage = Vec::with_capacity(n);
    let mut pearson = Vec::with_capacity(n);
    let mut deviance = Vec::with_capacity(n);
    let mut std_p = Vec::with_capacity(n);
    let mut std_d = Vec::with_capacity(n);
    let mut cooks = Vec::with_capacity(n);
    let mut cx_rows = Vec::with_capacity(n);
    let mut w_vec = Vec::with_capacity(n);
    let mut dev_total = 0.0_f64;

    for (r, (&yi, &pi)) in y.iter().zip(probs.iter()).enumerate() {
        let pc = pi.clamp(1e-12, 1.0 - 1e-12);
        let wi = (pc * (1.0 - pc)).max(1e-12);
        w_vec.push(wi);
        let mut cx = vec![0.0_f64; p];
        for j in 0..p {
            let mut acc = 0.0_f64;
            for k in 0..p {
                acc += cov_canonical.get(j, k).unwrap_or(0.0) * x.get(r, k).unwrap_or(0.0);
            }
            cx[j] = acc;
        }
        let quad: f64 = (0..p).map(|j| x.get(r, j).unwrap_or(0.0) * cx[j]).sum();
        let hi = (wi * quad).clamp(0.0, 0.999_999);
        let omh = (1.0 - hi).max(1e-12);
        leverage.push(hi);
        cx_rows.push(cx);

        let rp = (yi - pi) / wi.sqrt();
        let dev_i = (-2.0 * (yi * pc.ln() + (1.0 - yi) * (1.0 - pc).ln())).max(0.0);
        let rd = if yi >= pi { dev_i.sqrt() } else { -dev_i.sqrt() };
        dev_total += dev_i;
        pearson.push(rp);
        deviance.push(rd);
        std_p.push(rp / omh.sqrt());
        std_d.push(rd / omh.sqrt());
        cooks.push((rp * rp / (p as f64)) * hi / (omh * omh));
    }

    let se_diag: Vec<f64> = (0..p).map(|j| cov_canonical.get(j, j).unwrap_or(0.0).max(0.0).sqrt()).collect();
    let df_m1 = (n as f64) - (p as f64) - 1.0;
    let mut dfbetas = Vec::with_capacity(n);
    for r in 0..n {
        let hi = leverage[r];
        let omh = (1.0 - hi).max(1e-12);
        let rd = deviance[r];
        let s_del = if df_m1 > 0.0 { ((dev_total - rd * rd / omh).max(0.0) / df_m1).sqrt() } else { 1.0 };
        let mut row = vec![f64::NAN; p];
        if s_del > 0.0 {
            let scale = w_vec[r].sqrt() * rd / (omh * s_del);
            for j in 0..p {
                if se_diag[j] > 0.0 {
                    row[j] = (cx_rows[r][j] * scale) / se_diag[j];
                }
            }
        }
        dfbetas.push(row);
    }

    Ok(GlmRowDiagnostics {
        leverage,
        pearson_residuals: pearson,
        deviance_residuals: deviance,
        standardized_pearson: std_p,
        standardized_deviance: std_d,
        cooks_distance: cooks,
        dfbetas,
    })
}

/// Compute Pregibon (1981) / R `influence.glm` row-level diagnostics for a fitted Poisson GLM.
pub fn compute_poisson_diagnostics(
    x: &DenseMatrix,
    y: &[f64],
    mus: &[f64],
    cov_canonical: &DenseMatrix,
    dispersion_phi: f64,
) -> Result<GlmRowDiagnostics, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: y.len() });
    }
    if mus.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: mus.len() });
    }
    if cov_canonical.nrows() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_canonical.nrows() });
    }
    if cov_canonical.ncols() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_canonical.ncols() });
    }
    let phi = if dispersion_phi.is_finite() && dispersion_phi > 0.0 { dispersion_phi } else { 1.0 };
    let mut leverage = Vec::with_capacity(n);
    let mut pearson = Vec::with_capacity(n);
    let mut deviance = Vec::with_capacity(n);
    let mut std_p = Vec::with_capacity(n);
    let mut std_d = Vec::with_capacity(n);
    let mut cooks = Vec::with_capacity(n);
    let mut cx_rows = Vec::with_capacity(n);
    let mut w_vec = Vec::with_capacity(n);
    let mut dev_total = 0.0_f64;

    for (r, (&yi, &mui)) in y.iter().zip(mus.iter()).enumerate() {
        let mu = mui.max(1e-12);
        w_vec.push(mu);
        let mut cx = vec![0.0_f64; p];
        for j in 0..p {
            let mut acc = 0.0_f64;
            for k in 0..p {
                acc += cov_canonical.get(j, k).unwrap_or(0.0) * x.get(r, k).unwrap_or(0.0);
            }
            cx[j] = acc;
        }
        let quad: f64 = (0..p).map(|j| x.get(r, j).unwrap_or(0.0) * cx[j]).sum();
        let hi = (mu * quad).clamp(0.0, 0.999_999);
        let omh = (1.0 - hi).max(1e-12);
        leverage.push(hi);
        cx_rows.push(cx);

        let rp = (yi - mu) / mu.sqrt();
        let dev_i = if yi > 0.0 { (2.0 * (yi * (yi / mu).ln() - (yi - mu))).max(0.0) } else { (2.0 * mu).max(0.0) };
        let rd = if yi >= mu { dev_i.sqrt() } else { -dev_i.sqrt() };
        dev_total += dev_i;
        pearson.push(rp);
        deviance.push(rd);
        std_p.push(rp / omh.sqrt());
        std_d.push(rd / omh.sqrt());
        cooks.push((rp * rp / (phi * (p as f64))) * hi / (omh * omh));
    }

    let se_diag: Vec<f64> = (0..p).map(|j| cov_canonical.get(j, j).unwrap_or(0.0).max(0.0).sqrt()).collect();
    let df_m1 = (n as f64) - (p as f64) - 1.0;
    let mut dfbetas = Vec::with_capacity(n);
    for r in 0..n {
        let hi = leverage[r];
        let omh = (1.0 - hi).max(1e-12);
        let rd = deviance[r];
        let s_del = if df_m1 > 0.0 { ((dev_total - rd * rd / omh).max(0.0) / df_m1).sqrt() } else { 1.0 };
        let mut row = vec![f64::NAN; p];
        if s_del > 0.0 {
            let scale = w_vec[r].sqrt() * rd / (omh * s_del);
            for j in 0..p {
                if se_diag[j] > 0.0 {
                    row[j] = (cx_rows[r][j] * scale) / se_diag[j];
                }
            }
        }
        dfbetas.push(row);
    }

    Ok(GlmRowDiagnostics {
        leverage,
        pearson_residuals: pearson,
        deviance_residuals: deviance,
        standardized_pearson: std_p,
        standardized_deviance: std_d,
        cooks_distance: cooks,
        dfbetas,
    })
}

/// Partition fitted logistic probabilities into Hosmer–Lemeshow quantile bins for observed-vs-expected
/// calibration diagnostics (Hosmer & Lemeshow 2000).
#[must_use]
pub fn compute_hosmer_lemeshow_bins(y: &[f64], probs: &[f64], g_groups: usize) -> Vec<HosmerLemeshowBin> {
    let n = y.len();
    if n < 3 || probs.len() != n {
        return Vec::new();
    }
    let mut pairs: Vec<(f64, f64)> = probs.iter().copied().zip(y.iter().copied()).collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    struct ProbBlock {
        rep_p: f64,
        sum_p: f64,
        sum_y: f64,
        count: usize,
    }
    let mut blocks: Vec<ProbBlock> = Vec::new();
    for (p, yv) in pairs {
        if let Some(last) = blocks.last_mut()
            && (p - last.rep_p).abs() <= 1.0e-12 * (1.0 + last.rep_p.abs())
        {
            last.sum_p += p;
            last.sum_y += yv;
            last.count += 1;
            continue;
        }
        blocks.push(ProbBlock { rep_p: p, sum_p: p, sum_y: yv, count: 1 });
    }
    if blocks.is_empty() {
        return Vec::new();
    }
    let g = g_groups.min(blocks.len()).max(2);
    let mut out = Vec::with_capacity(g);
    let mut block_idx = 0_usize;
    let mut cum_count = 0_usize;
    for bin_idx in 0..g {
        if block_idx >= blocks.len() {
            break;
        }
        let target = ((bin_idx + 1) * n) as f64 / (g as f64);
        let remaining_bins = g - bin_idx - 1;
        let mut obs1 = NeumaierSum::new();
        let mut exp1 = NeumaierSum::new();
        let mut nb = 0_usize;
        while block_idx < blocks.len() {
            let blk = &blocks[block_idx];
            if nb > 0 {
                let remaining_after = blocks.len() - block_idx;
                if remaining_after <= remaining_bins {
                    break;
                }
                let curr_dist = ((cum_count as f64) - target).abs();
                let next_dist = (((cum_count + blk.count) as f64) - target).abs();
                if next_dist > curr_dist {
                    break;
                }
            }
            exp1.add(blk.sum_p);
            obs1.add(blk.sum_y);
            nb += blk.count;
            cum_count += blk.count;
            block_idx += 1;
        }
        if nb > 0 {
            let o = obs1.total();
            let e = exp1.total();
            let nbf = nb as f64;
            out.push(HosmerLemeshowBin {
                label: format!("Bin {}", out.len() + 1),
                count: nb,
                observed_events: o,
                expected_events: e,
                observed_rate: o / nbf,
                expected_rate: e / nbf,
            });
        }
    }
    out
}

/// 1D Brent's root-finder on a bracket `[a, b]` with `f(a) * f(b) <= 0`.
pub(crate) fn brent_root<F: FnMut(f64) -> Option<f64>>(mut f: F, mut a: f64, mut fa: f64, mut b: f64, mut fb: f64, tol: f64) -> Option<f64> {
    if !fa.is_finite() || !fb.is_finite() || fa * fb > 0.0 {
        return None;
    }
    let (mut c, mut fc) = (a, fa);
    let mut d = b - a;
    let mut e = d;
    for _ in 0..64 {
        if (fb > 0.0 && fc > 0.0) || (fb < 0.0 && fc < 0.0) {
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb.abs() <= 1e-13 {
            return Some(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (p, q) = if (a - c).abs() <= 1e-15 {
                (2.0 * xm * s, 1.0 - s)
            } else {
                let q0 = fa / fc;
                let r0 = fb / fc;
                (s * (2.0 * xm * q0 * (q0 - r0) - (b - a) * (r0 - 1.0)), (q0 - 1.0) * (r0 - 1.0) * (s - 1.0))
            };
            let (p, q) = if p > 0.0 { (p, -q) } else { (-p, q) };
            if 2.0 * p < (3.0 * xm * q - (tol1 * q).abs()).min((e * q).abs()) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        b += if d.abs() > tol1 { d } else { tol1.copysign(xm) };
        fb = f(b)?;
    }
    Some(b)
}

fn eval_logistic_offset_ll(x_rest: &DenseMatrix, offset: &[f64], y: &[f64], b_rest: &[f64]) -> Option<(f64, Vec<f64>)> {
    let n = y.len();
    let q = b_rest.len();
    let mut ll = NeumaierSum::new();
    let mut probs = Vec::with_capacity(n);
    for (r, (&yr, &off)) in y.iter().zip(offset.iter()).enumerate() {
        let mut eta = off;
        for (c, &bc) in b_rest.iter().enumerate().take(q) {
            eta += x_rest.get(r, c).unwrap_or(0.0) * bc;
        }
        if !eta.is_finite() {
            return None;
        }
        let eta_c = eta.clamp(-35.0, 35.0);
        let p = 1.0 / (1.0 + (-eta_c).exp());
        let log1p_exp = if eta > 0.0 { eta + (-eta).exp().ln_1p() } else { eta.exp().ln_1p() };
        ll.add(yr * eta - log1p_exp);
        probs.push(p);
    }
    let val = ll.total();
    val.is_finite().then_some((val, probs))
}

fn solve_logistic_nuisance(x_rest: &DenseMatrix, offset: &[f64], y: &[f64], b_warm: &mut [f64]) -> Option<f64> {
    let n = y.len();
    let q = b_warm.len();
    let (mut ll_curr, mut probs) = eval_logistic_offset_ll(x_rest, offset, y, b_warm)?;
    if q == 0 {
        return Some(ll_curr);
    }
    for _ in 0..40 {
        let mut grad = vec![0.0_f64; q];
        let mut info = vec![0.0_f64; q * q];
        for r in 0..n {
            let diff = y[r] - probs[r];
            let wr = (probs[r] * (1.0 - probs[r])).max(1e-12);
            for j in 0..q {
                let xrj = x_rest.get(r, j).unwrap_or(0.0);
                grad[j] += xrj * diff;
                for k in j..q {
                    info[j * q + k] += wr * xrj * x_rest.get(r, k).unwrap_or(0.0);
                }
            }
        }
        for j in 0..q {
            for k in 0..j {
                info[j * q + k] = info[k * q + j];
            }
        }
        let info_mat = DenseMatrix::from_row_major(q, q, info).ok()?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad).ok()?
        } else {
            let (ls, _) = solve_linear_least_squares(&info_mat, &grad).ok()?;
            ls.coefficients
        };
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        if !max_step.is_finite() {
            return None;
        }
        if max_step < 1e-11 {
            return Some(ll_curr);
        }
        let mut scale = if max_step > 5.0 { 5.0 / max_step } else { 1.0 };
        let mut accepted = false;
        for _ in 0..24 {
            let cand: Vec<f64> = b_warm.iter().zip(step.iter()).map(|(&b, &s)| b + scale * s).collect();
            if let Some((ll_c, p_c)) = eval_logistic_offset_ll(x_rest, offset, y, &cand)
                && ll_c >= ll_curr - 1e-12
            {
                let delta = (ll_c - ll_curr).abs();
                b_warm.copy_from_slice(&cand);
                ll_curr = ll_c;
                probs = p_c;
                accepted = true;
                if delta < 1e-12 && max_step * scale < 1e-9 {
                    return Some(ll_curr);
                }
                break;
            }
            scale *= 0.5;
        }
        if !accepted {
            break;
        }
    }
    Some(ll_curr)
}

/// Compute Venzon–Moolgavkar (1988) profile-likelihood confidence intervals for every parameter
/// in a Logistic GLM via warm-started offset IRLS and bounded Brent root-finding.
pub fn profile_ci_logistic(
    x: &DenseMatrix,
    y: &[f64],
    beta_hat: &[f64],
    se_hat: &[f64],
    ll_max: f64,
    confidence: ConfidenceLevel,
) -> Result<Vec<(f64, f64)>, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: y.len() });
    }
    if beta_hat.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: beta_hat.len() });
    }
    if se_hat.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: se_hat.len() });
    }
    let z = norm_two_sided_isf(confidence.alpha())?;
    let thresh = ll_max - 0.5 * z * z;
    let mut out = Vec::with_capacity(p);

    for j in 0..p {
        let b0 = beta_hat[j];
        let se = se_hat[j];
        if !b0.is_finite() || !se.is_finite() || se <= 0.0 {
            out.push((f64::NAN, f64::NAN));
            continue;
        }
        let col_j: Vec<f64> = (0..n).map(|r| x.get(r, j).unwrap_or(0.0)).collect();
        let x_rest = if p > 1 { drop_col_from_dense(x, j)? } else { DenseMatrix::zeros(n, 0)? };
        let b_rest_mle: Vec<f64> = beta_hat.iter().enumerate().filter(|&(k, _)| k != j).map(|(_, &v)| v).collect();

        let solve_side = |dir: f64| -> f64 {
            let mut b_warm = b_rest_mle.clone();
            let mut eval_diff = |bj: f64| -> Option<f64> {
                let offset: Vec<f64> = col_j.iter().map(|&xj| bj * xj).collect();
                let ll = solve_logistic_nuisance(&x_rest, &offset, y, &mut b_warm)?;
                Some(ll - thresh)
            };
            let mut prev_x = b0;
            let mut prev_f = ll_max - thresh;
            let base_step = (z * se).max(1e-4);
            let mut cur_step = base_step;
            let max_dist = (40.0 * se).max(50.0);
            for _ in 0..24 {
                let cand = prev_x + dir * cur_step;
                if (cand - b0).abs() > max_dist {
                    return if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY };
                }
                let Some(fc) = eval_diff(cand) else {
                    cur_step *= 0.5;
                    continue;
                };
                if fc <= 0.0 {
                    let (a, fa, b, fb) = if dir < 0.0 { (cand, fc, prev_x, prev_f) } else { (prev_x, prev_f, cand, fc) };
                    return brent_root(eval_diff, a, fa, b, fb, 1e-10 * (1.0 + b0.abs())).unwrap_or(cand);
                }
                prev_x = cand;
                prev_f = fc;
                cur_step *= 1.6;
            }
            if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY }
        };

        out.push((solve_side(-1.0), solve_side(1.0)));
    }
    Ok(out)
}

fn eval_poisson_offset_ll(x_rest: &DenseMatrix, offset: &[f64], y: &[f64], ln_fact: &[f64], b_rest: &[f64]) -> Option<(f64, Vec<f64>)> {
    let n = y.len();
    let q = b_rest.len();
    let mut ll = NeumaierSum::new();
    let mut mus = Vec::with_capacity(n);
    for (r, ((&yr, &off), &lnf)) in y.iter().zip(offset.iter()).zip(ln_fact.iter()).enumerate() {
        let mut eta = off;
        for (c, &bc) in b_rest.iter().enumerate().take(q) {
            eta += x_rest.get(r, c).unwrap_or(0.0) * bc;
        }
        if !eta.is_finite() || eta > 500.0 {
            return None;
        }
        let mu = eta.exp();
        if !mu.is_finite() {
            return None;
        }
        ll.add(yr * eta - mu - lnf);
        mus.push(mu);
    }
    let val = ll.total();
    val.is_finite().then_some((val, mus))
}

fn solve_poisson_nuisance(x_rest: &DenseMatrix, offset: &[f64], y: &[f64], ln_fact: &[f64], b_warm: &mut [f64]) -> Option<f64> {
    let n = y.len();
    let q = b_warm.len();
    let (mut ll_curr, mut mus) = eval_poisson_offset_ll(x_rest, offset, y, ln_fact, b_warm)?;
    if q == 0 {
        return Some(ll_curr);
    }
    for _ in 0..40 {
        let mut grad = vec![0.0_f64; q];
        let mut info = vec![0.0_f64; q * q];
        for r in 0..n {
            let diff = y[r] - mus[r];
            let wr = mus[r].max(1e-12);
            for j in 0..q {
                let xrj = x_rest.get(r, j).unwrap_or(0.0);
                grad[j] += xrj * diff;
                for k in j..q {
                    info[j * q + k] += wr * xrj * x_rest.get(r, k).unwrap_or(0.0);
                }
            }
        }
        for j in 0..q {
            for k in 0..j {
                info[j * q + k] = info[k * q + j];
            }
        }
        let info_mat = DenseMatrix::from_row_major(q, q, info).ok()?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad).ok()?
        } else {
            let (ls, _) = solve_linear_least_squares(&info_mat, &grad).ok()?;
            ls.coefficients
        };
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        if !max_step.is_finite() {
            return None;
        }
        if max_step < 1e-11 {
            return Some(ll_curr);
        }
        let max_eta = (0..n).map(|r| (0..q).map(|j| x_rest.get(r, j).unwrap_or(0.0) * step[j]).sum::<f64>().abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_eta > 8.0 { 8.0 / max_eta } else { 1.0 };
        let mut accepted = false;
        for _ in 0..24 {
            let cand: Vec<f64> = b_warm.iter().zip(step.iter()).map(|(&b, &s)| b + scale * s).collect();
            if let Some((ll_c, m_c)) = eval_poisson_offset_ll(x_rest, offset, y, ln_fact, &cand)
                && ll_c >= ll_curr - 1e-12
            {
                let delta = (ll_c - ll_curr).abs();
                b_warm.copy_from_slice(&cand);
                ll_curr = ll_c;
                mus = m_c;
                accepted = true;
                if delta < 1e-12 && max_step * scale < 1e-9 {
                    return Some(ll_curr);
                }
                break;
            }
            scale *= 0.5;
        }
        if !accepted {
            break;
        }
    }
    Some(ll_curr)
}

/// Compute Venzon–Moolgavkar (1988) profile-likelihood confidence intervals for every parameter
/// in a Poisson (or quasi-Poisson) GLM via warm-started offset IRLS and bounded Brent root-finding.
#[allow(clippy::too_many_arguments)]
pub fn profile_ci_poisson(
    x: &DenseMatrix,
    y: &[f64],
    beta_hat: &[f64],
    se_hat: &[f64],
    ll_max: f64,
    quasi_dispersion: Option<(f64, f64)>,
    confidence: ConfidenceLevel,
) -> Result<Vec<(f64, f64)>, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: y.len() });
    }
    if beta_hat.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: beta_hat.len() });
    }
    if se_hat.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: se_hat.len() });
    }
    let (drop_ll, step_mult) = match quasi_dispersion {
        Some((phi, df_resid)) if phi.is_finite() && phi > 0.0 && df_resid >= 1.0 => {
            let f_crit = f_isf(confidence.alpha(), 1.0, df_resid)?;
            (0.5 * phi * f_crit, f_crit.sqrt().max(1.0))
        }
        _ => {
            let z = norm_two_sided_isf(confidence.alpha())?;
            (0.5 * z * z, z.max(1.0))
        }
    };
    let thresh = ll_max - drop_ll;
    let ln_fact: Vec<f64> = y.iter().map(|&yr| curvebook_math::special::ln_gamma(yr + 1.0).unwrap_or(0.0)).collect();
    let mut out = Vec::with_capacity(p);

    for j in 0..p {
        let b0 = beta_hat[j];
        let se = se_hat[j];
        if !b0.is_finite() || !se.is_finite() || se <= 0.0 {
            out.push((f64::NAN, f64::NAN));
            continue;
        }
        let col_j: Vec<f64> = (0..n).map(|r| x.get(r, j).unwrap_or(0.0)).collect();
        let x_rest = if p > 1 { drop_col_from_dense(x, j)? } else { DenseMatrix::zeros(n, 0)? };
        let b_rest_mle: Vec<f64> = beta_hat.iter().enumerate().filter(|&(k, _)| k != j).map(|(_, &v)| v).collect();

        let solve_side = |dir: f64| -> f64 {
            let mut b_warm = b_rest_mle.clone();
            let mut eval_diff = |bj: f64| -> Option<f64> {
                let offset: Vec<f64> = col_j.iter().map(|&xj| bj * xj).collect();
                let ll = solve_poisson_nuisance(&x_rest, &offset, y, &ln_fact, &mut b_warm)?;
                Some(ll - thresh)
            };
            let mut prev_x = b0;
            let mut prev_f = ll_max - thresh;
            let base_step = (step_mult * se).max(1e-4);
            let mut cur_step = base_step;
            let max_dist = (40.0 * se).max(20.0 * base_step).max(50.0);
            for _ in 0..24 {
                let cand = prev_x + dir * cur_step;
                if (cand - b0).abs() > max_dist {
                    return if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY };
                }
                let Some(fc) = eval_diff(cand) else {
                    cur_step *= 0.5;
                    continue;
                };
                if fc <= 0.0 {
                    let (a, fa, b, fb) = if dir < 0.0 { (cand, fc, prev_x, prev_f) } else { (prev_x, prev_f, cand, fc) };
                    return brent_root(eval_diff, a, fa, b, fb, 1e-10 * (1.0 + b0.abs())).unwrap_or(cand);
                }
                prev_x = cand;
                prev_f = fc;
                cur_step *= 1.6;
            }
            if dir < 0.0 { f64::NEG_INFINITY } else { f64::INFINITY }
        };

        out.push((solve_side(-1.0), solve_side(1.0)));
    }
    Ok(out)
}

/// Compute out-of-sample Linear GLM predictions, confidence intervals (for mean response), and
/// prediction intervals (for a new observation at unit weight).
pub fn predict_linear_oos(
    x_new: &DenseMatrix,
    beta: &[f64],
    cov_beta: &[Vec<f64>],
    mse: f64,
    df_resid: f64,
    confidence: ConfidenceLevel,
) -> Result<Vec<LinearOosPrediction>, StatsError> {
    let p = x_new.ncols();
    if beta.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: beta.len() });
    }
    if cov_beta.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_beta.len() });
    }
    if let Some(bad_row) = cov_beta.iter().find(|row| row.len() != p) {
        return Err(StatsError::DimensionMismatch { expected: p, actual: bad_row.len() });
    }
    let t_crit = student_t_two_sided_isf(confidence.alpha(), df_resid.max(1.0))?;
    let mut out = Vec::with_capacity(x_new.nrows());
    for r in 0..x_new.nrows() {
        let mut fit = 0.0_f64;
        let mut var_fit = 0.0_f64;
        for j in 0..p {
            let xj = x_new.get(r, j).unwrap_or(0.0);
            if beta[j].is_finite() {
                fit += xj * beta[j];
            }
            for k in 0..p {
                let xk = x_new.get(r, k).unwrap_or(0.0);
                var_fit += xj * cov_beta[j].get(k).copied().unwrap_or(0.0) * xk;
            }
        }
        let vf = var_fit.max(0.0);
        let se_fit = vf.sqrt();
        let se_pred = (mse.max(0.0) + vf).sqrt();
        out.push(LinearOosPrediction {
            fit,
            se_fit,
            ci: (fit - t_crit * se_fit, fit + t_crit * se_fit),
            pi: (fit - t_crit * se_pred, fit + t_crit * se_pred),
        });
    }
    Ok(out)
}

/// Compute out-of-sample Logistic GLM predicted probabilities and link-scale back-transformed CIs.
pub fn predict_logistic_oos(
    x_new: &DenseMatrix,
    beta: &[f64],
    cov_beta: &DenseMatrix,
    cutoff: f64,
    confidence: ConfidenceLevel,
) -> Result<Vec<LogisticOosPrediction>, StatsError> {
    let p = x_new.ncols();
    if beta.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: beta.len() });
    }
    if cov_beta.nrows() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_beta.nrows() });
    }
    if cov_beta.ncols() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_beta.ncols() });
    }
    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let logistic = |eta: f64| -> f64 { 1.0 / (1.0 + (-eta.clamp(-50.0, 50.0)).exp()) };
    let mut out = Vec::with_capacity(x_new.nrows());
    for r in 0..x_new.nrows() {
        let mut eta = 0.0_f64;
        let mut var_eta = 0.0_f64;
        for j in 0..p {
            let xj = x_new.get(r, j).unwrap_or(0.0);
            eta += xj * beta[j];
            for k in 0..p {
                var_eta += xj * cov_beta.get(j, k).unwrap_or(0.0) * x_new.get(r, k).unwrap_or(0.0);
            }
        }
        let se_eta = var_eta.max(0.0).sqrt();
        let prob = logistic(eta);
        let lo = logistic(eta - z_crit * se_eta);
        let hi = logistic(eta + z_crit * se_eta);
        out.push(LogisticOosPrediction {
            eta,
            se_eta,
            probability: prob,
            prob_ci: (lo, hi),
            predicted_class: if prob >= cutoff { 1.0 } else { 0.0 },
        });
    }
    Ok(out)
}

/// Compute out-of-sample Poisson GLM expected counts / rates and log-link back-transformed CIs.
pub fn predict_poisson_oos(
    x_new: &DenseMatrix,
    beta: &[f64],
    cov_beta: &DenseMatrix,
    crit_val: f64,
) -> Result<Vec<PoissonOosPrediction>, StatsError> {
    let p = x_new.ncols();
    if beta.len() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: beta.len() });
    }
    if cov_beta.nrows() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_beta.nrows() });
    }
    if cov_beta.ncols() != p {
        return Err(StatsError::DimensionMismatch { expected: p, actual: cov_beta.ncols() });
    }
    let mut out = Vec::with_capacity(x_new.nrows());
    for r in 0..x_new.nrows() {
        let mut eta = 0.0_f64;
        let mut var_eta = 0.0_f64;
        for j in 0..p {
            let xj = x_new.get(r, j).unwrap_or(0.0);
            eta += xj * beta[j];
            for k in 0..p {
                var_eta += xj * cov_beta.get(j, k).unwrap_or(0.0) * x_new.get(r, k).unwrap_or(0.0);
            }
        }
        let se_eta = var_eta.max(0.0).sqrt();
        let rate = eta.clamp(-500.0, 500.0).exp();
        let lo = (eta - crit_val * se_eta).clamp(-500.0, 500.0).exp();
        let hi = (eta + crit_val * se_eta).clamp(-500.0, 500.0).exp();
        out.push(PoissonOosPrediction { eta, se_eta, rate, rate_ci: (lo, hi) });
    }
    Ok(out)
}
