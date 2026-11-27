//! Generalized linear and multiple regression models: Multiple Linear Regression (OLS with
//! Type I/III partial *F*, VIF multicollinearity, Cook's distance, leverage, Durbin–Watson),
//! Multiple Logistic Regression (IRLS with separation safeguard, Hosmer–Lemeshow, pseudo-*R*^2,
//! ROC AUC), and Multiple Poisson Regression (log-link IRLS, deviance & Pearson dispersion).
//!
//! References (D1 citation pass 2026-11-18; diagnostics cite their sources inline):
//! - Golub, G. (1965). Numerical methods for solving linear least squares problems. *Numerische
//!   Mathematik*, 7(3), 206-216. `doi:10.1007/bf01436075`.
//! - Björck, Å. (1967). Iterative refinement of linear least squares solutions I. *BIT*, 7(4),
//!   257-278. `doi:10.1007/bf01939321`.
//! - McCullagh, P., & Nelder, J. A. (1989). *Generalized Linear Models* (2nd ed.). London: Chapman &
//!   Hall.
//! - Albert, A., & Anderson, J. A. (1984). On the existence of maximum likelihood estimates in
//!   logistic regression models. *Biometrika*, 71(1), 1-10. `doi:10.1093/biomet/71.1.1`.
//! - Hosmer, D. W., & Lemeshow, S. (1980). Goodness of fit tests for the multiple logistic
//!   regression model. *Communications in Statistics – Theory and Methods*, 9(10), 1043-1069.
//!   `doi:10.1080/03610928008827941`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, GlmFamily, GlmSpec, PValueFormat,
    PassingBablokSpec, ResultCell, ResultSheet, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableColumn, VariableRole, XColumnMode,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, f_sf, norm_two_sided_isf, norm_two_sided_p, student_t_two_sided_isf, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, svd};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, checklist_item, ci_cell,
    enforce_summary_mode_matrix, finite_ci_cell, init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_num_cell, p_cell_at_alpha,
    push_scalar, sanitize_result_sheet_floats, text_cell, validate_finite_slice,
};
use crate::error::StatsError;
use crate::roc::{RocCiMethod, roc_curve_analysis};

// ============================================================================
// Typed Outputs
// ============================================================================

/// Parameter row for Multiple Linear Regression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearTermEstimate {
    /// Parameter or encoded factor-level term name (e.g. `"Intercept"` or `"Arm [B]"`).
    pub name: String,
    /// Ordinary least-squares regression coefficient $\hat{\beta}_j$ (`f64::NAN` on aliased/collinear terms).
    pub estimate: f64,
    /// Standard error $\sqrt{\hat{\sigma}^2 (X^\top X)^+_{jj}}$ (`f64::NAN` on aliased/collinear terms, `0.0` on exact non-aliased fits).
    pub se: f64,
    /// Two-sided Student *t* statistic $\hat{\beta}_j / \mathrm{SE}(\hat{\beta}_j)$ on `df_residual` df (`f64::NAN` when SE is zero or aliased).
    pub t_statistic: f64,
    /// Two-sided *P* value $P(|T_{\text{df}}| \ge |t|)$ testing $H_0: \beta_j = 0$ (`f64::NAN` when SE is zero or aliased).
    pub p_value: f64,
    /// Two-sided `t(df_residual)` confidence interval `(lower, upper)` at configured `ConfidenceLevel` (`(f64::NAN, f64::NAN)` when SE is zero or aliased).
    pub ci: (f64, f64),
    /// Variance Inflation Factor $1 / (1 - R_j^2)$ measuring predictor collinearity (`None` for the intercept, when `compute_vif == false`, or single-predictor models).
    pub vif: Option<f64>,
    /// Type III partial sum of squares measuring the effect of dropping this term while retaining all others (`None` when intercept or unidentifiable).
    pub type_iii_ss: Option<f64>,
    /// Type III partial *F*-statistic on `(1, df_residual)` df (`None` when `type_iii_ss` is unavailable).
    pub type_iii_f: Option<f64>,
    /// Upper-tail Type III partial *F* *P* value (`None` when `type_iii_f` is unavailable).
    pub type_iii_p: Option<f64>,
}

/// Complete Multiple Linear Regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleLinearResult {
    /// Number of complete rows (observations) included in the regression fit.
    pub n: usize,
    /// Total number of columns in the design matrix `X` (including intercept when `has_intercept == true`).
    pub p: usize,
    /// Numerical column rank of `X` from SVD (`rank < p` when predictors are linearly dependent).
    pub rank: usize,
    /// True when column `0` represents the model intercept.
    pub has_intercept: bool,
    /// Per-column boolean mask (`len == p`) marking aliased/rank-deficient predictors dropped from estimation.
    pub aliased_columns: Vec<bool>,
    /// Ordered coefficient, SE, *t*, *P*, CI, VIF, and Type III SS/F/P records of length `p`.
    pub terms: Vec<LinearTermEstimate>,
    /// Coefficient of determination $R^2 = 1 - \text{rss} / \text{ss\_total}$ in `[0, 1]`.
    pub r_squared: f64,
    /// Adjusted coefficient of determination $1 - (1 - R^2) (n - \mathbb{I}[\text{intercept}]) / \text{df\_residual}$ (`R^2` adjusted for degrees of freedom).
    pub adjusted_r_squared: f64,
    /// Residual sum of squares $\sum (y_i - \hat{y}_i)^2$ in squared response units.
    pub rss: f64,
    /// Model regression sum of squares (`ss_total - rss`).
    pub ss_regression: f64,
    /// Total sum of squares ($\sum (y_i - \bar{y})^2$ when `has_intercept == true`, else uncentered $\sum y_i^2$).
    pub ss_total: f64,
    /// Residual standard deviation $\hat{\sigma} = \sqrt{\text{rss} / \text{df\_residual}}$ in response units (`Sy.x`).
    pub residual_sd: f64,
    /// Regression model numerator degrees of freedom (`rank - 1` with intercept, else `rank`).
    pub df_model: f64,
    /// Residual denominator degrees of freedom `n - rank`.
    pub df_residual: f64,
    /// Omnibus ANOVA *F*-statistic $(\text{ss\_regression} / \text{df\_model}) / (\text{rss} / \text{df\_residual})$.
    pub f_statistic: f64,
    /// Upper-tail $F(\text{df\_model}, \text{df\_residual})$ *P* value testing zero slope for all non-intercept terms.
    pub f_p_value: f64,
    /// Gaussian maximum-likelihood Akaike Information Criterion `n * ln(rss / n) + 2 * (rank + 1)` (includes constant/noise variance parameter; relative comparisons unaffected).
    pub aic: f64,
    /// Small-sample corrected $\text{AIC}_c$ (`f64::INFINITY` when `n - (rank + 1) - 1 <= 0`).
    pub aicc: f64,
    /// Schwarz Bayesian Information Criterion `BIC` under the Gaussian likelihood.
    pub bic: f64,
    /// Durbin–Watson first-order residual autocorrelation statistic in `[0, 4]` (`~2.0` indicates uncorrelated residuals).
    pub durbin_watson: f64,
    /// Fitted mean responses $\hat{y}_i = X_i \hat{\beta}$ in complete-case input order (`len == n`).
    pub fitted: Vec<f64>,
    /// Unweighted OLS residuals $y_i - \hat{y}_i$ (`len == n`).
    pub residuals: Vec<f64>,
    /// Diagonal hat-matrix leverage values $h_{ii} = x_i^\top (X^\top X)^+ x_i \in [0, 1]$ (`len == n`).
    pub leverage: Vec<f64>,
    /// Cook's distance $D_i = \frac{r_i^2}{k \hat{\sigma}^2} \frac{h_{ii}}{(1 - h_{ii})^2}$ measuring case influence (`0.0` or large at $h_{ii} = 1$; `len == n`).
    pub cooks_distance: Vec<f64>,
    /// Parameter covariance matrix `sigma^2 * (X'X)^-1` (`p x p`).
    #[serde(default)]
    pub cov_beta: Vec<Vec<f64>>,
}

/// Parameter row for Logistic or Poisson GLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlmTermEstimate {
    /// Coefficient term name (`"Intercept"` or predictor / encoded dummy level).
    pub name: String,
    /// Log-link or logit-link regression coefficient $\hat{\beta}_j$.
    pub beta: f64,
    /// Asymptotic standard error $\sqrt{I(\hat{\beta})^{-1}_{jj}}$ from Fisher information (`McCullagh & Nelder 1989`).
    pub se: f64,
    /// Asymptotic Wald standard normal *z* statistic $\hat{\beta}_j / \mathrm{SE}(\hat{\beta}_j)$.
    pub z_statistic: f64,
    /// Two-sided Wald normal *P* value $2 \Phi(-|z|)$ testing $H_0: \beta_j = 0$.
    pub p_value: f64,
    /// Exponentiated effect ratio $\exp(\hat{\beta}_j)$ (Odds Ratio for logistic regression; Incidence Rate Ratio for Poisson regression).
    pub exp_beta: f64,
    /// Two-sided Wald confidence interval $\exp(\hat{\beta}_j \pm z_{\alpha/2}\mathrm{SE})$ for `exp_beta` at configured `ConfidenceLevel`.
    pub exp_beta_ci: (f64, f64),
    /// Variance Inflation Factor $1 / (1 - R_j^2)$ (`None` for the intercept, when `compute_vif == false`, or single-predictor fits).
    pub vif: Option<f64>,
}

/// Complete Multiple Logistic Regression output (Bernoulli/binomial logit GLM via IRLS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleLogisticResult {
    /// Number of complete binary observations included (`n`).
    pub n: usize,
    /// Number of observed positive events (`y == 1`) across the `n` cases (`0 < n_events < n`).
    pub n_events: usize,
    /// True when IRLS log-likelihood and coefficient steps satisfied convergence tolerance.
    pub converged: bool,
    /// True when Albert–Anderson (1984) quasi-complete or complete linear separation was detected during IRLS.
    pub separation_detected: bool,
    /// Ordered coefficient, OR, Wald *z*/*P*/CI, and VIF records (`len == p`).
    pub terms: Vec<GlmTermEstimate>,
    /// Null intercept-only Bernoulli log-likelihood $\ell_0$.
    pub log_lik_null: f64,
    /// Fitted model unpenalized Bernoulli log-likelihood $\ell(\hat{\beta})$.
    pub log_lik_model: f64,
    /// Null model deviance `-2 * log_lik_null`.
    pub deviance_null: f64,
    /// Residual deviance `-2 * log_lik_model >= 0`.
    pub deviance_residual: f64,
    /// Omnibus Likelihood Ratio $\chi^2 = \text{deviance\_null} - \text{deviance\_residual} \ge 0$.
    pub lr_chi2: f64,
    /// Degrees of freedom for the omnibus Likelihood Ratio test (`max(p - 1, 1)`).
    pub lr_df: f64,
    /// Upper-tail $\chi^2(\text{lr\_df})$ *P* value testing all non-intercept coefficients equal to zero.
    pub lr_p: f64,
    /// Akaike Information Criterion `deviance_residual + 2 * p`.
    pub aic: f64,
    /// Small-sample corrected $\text{AIC}_c$ (`f64::INFINITY` when `n - p - 1 <= 0`).
    pub aicc: f64,
    /// Schwarz Bayesian Information Criterion `deviance_residual + p * ln(n)`.
    pub bic: f64,
    /// McFadden's pseudo-$R^2 = 1 - \text{deviance\_residual} / \text{deviance\_null} \in [0, 1]$.
    pub mcfadden_r2: f64,
    /// Nagelkerke (1991) Cragg–Uhler rescaled pseudo-$R^2 \in [0, 1]$.
    pub nagelkerke_r2: f64,
    /// Tjur (2009) coefficient of discrimination $\bar{\hat{p}}_{y=1} - \bar{\hat{p}}_{y=0} \in [0, 1]$.
    pub tjur_r2: f64,
    /// Hosmer–Lemeshow decile calibration chi-square goodness-of-fit statistic (`Hosmer & Lemeshow 2000`).
    pub hosmer_lemeshow_chi2: f64,
    /// Hosmer–Lemeshow degrees of freedom (`groups - 2`, typically `8.0`).
    pub hosmer_lemeshow_df: f64,
    /// Upper-tail $\chi^2(\text{hosmer\_lemeshow\_df})$ calibration *P* value (`< 0.05` signals miscalibration).
    pub hosmer_lemeshow_p: f64,
    /// Area under the fitted ROC curve discriminating `y == 1` vs `y == 0` (`None` if single-class after filtering).
    pub roc_auc: Option<f64>,
    /// Fitted event probabilities $\hat{p}_i = \mathrm{logistic}(X_i \hat{\beta}) \in (0, 1)$ aligned with complete-case rows (`len == n`).
    pub predicted_probs: Vec<f64>,
}

/// Complete Multiple Poisson Regression output (log-link count GLM via IRLS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiplePoissonResult {
    /// Number of complete non-negative count/rate observations included (`n`).
    pub n: usize,
    /// True when IRLS deviance steps satisfied convergence tolerance.
    pub converged: bool,
    /// Ordered coefficient, IRR ($\exp(\beta)$), Wald *z*/*P*/CI, and VIF records (`len == p`).
    pub terms: Vec<GlmTermEstimate>,
    /// Null intercept-only Poisson log-likelihood $\ell_0$.
    pub log_lik_null: f64,
    /// Fitted model Poisson log-likelihood $\ell(\hat{\beta})$.
    pub log_lik_model: f64,
    /// Null Poisson deviance $2 \sum (y \ln(y / \bar{y}) - (y - \bar{y}))$.
    pub deviance_null: f64,
    /// Residual Poisson deviance $2 \sum (y \ln(y / \hat{\mu}) - (y - \hat{\mu})) \ge 0$.
    pub deviance_residual: f64,
    /// Pearson chi-square goodness-of-fit statistic $\sum (y_i - \hat{\mu}_i)^2 / \hat{\mu}_i$.
    pub pearson_chi2: f64,
    /// Residual denominator degrees of freedom `n - p`.
    pub df_residual: f64,
    /// Pearson dispersion parameter $\hat{\phi} = \text{pearson\_chi2} / \text{df\_residual}$ (`> 1` indicates overdispersion relative to Poisson variance).
    pub dispersion_pearson: f64,
    /// Omnibus Likelihood Ratio $\chi^2 = 2(\ell(\hat{\beta}) - \ell_0) \ge 0$.
    pub lr_chi2: f64,
    /// Degrees of freedom for the omnibus Likelihood Ratio test (`max(p - 1, 1)`).
    pub lr_df: f64,
    /// Upper-tail $\chi^2(\text{lr\_df})$ *P* value testing all non-intercept coefficients equal to zero.
    pub lr_p: f64,
    /// Akaike Information Criterion `-2 * log_lik_model + 2 * p`.
    pub aic: f64,
    /// Small-sample corrected $\text{AIC}_c$ (`f64::INFINITY` when `n - p - 1 <= 0`).
    pub aicc: f64,
    /// Schwarz Bayesian Information Criterion `-2 * log_lik_model + p * ln(n)`.
    pub bic: f64,
    /// Fitted Poisson conditional means $\hat{\mu}_i = \exp(X_i \hat{\beta}) > 0$ aligned with complete-case rows (`len == n`).
    pub predicted_means: Vec<f64>,
}

// ============================================================================
// Multiple Linear Regression (OLS via QR/SVD)
// ============================================================================

fn compute_ols_residuals_and_dw(x: &DenseMatrix, y: &[f64], coefficients: &[f64]) -> (Vec<f64>, Vec<f64>, f64, f64) {
    let n = y.len();
    let mut fitted = Vec::with_capacity(n);
    let mut residuals = Vec::with_capacity(n);
    let mut dw_den_acc = NeumaierSum::new();
    let mut max_term_scale = 0.0_f64;
    for (r, &yr) in y.iter().enumerate() {
        max_term_scale = max_term_scale.max(yr.abs());
        let mut main = NeumaierSum::new();
        let mut err = NeumaierSum::new();
        main.add(yr);
        for (c, &b) in coefficients.iter().enumerate() {
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
        dw_den_acc.add(res_r * res_r);
    }

    // Durbin & Watson (1950, Biometrika 37:409-428) autocorrelation statistic evaluated on the
    // self-consistent Dot2 residual vector, returning 2.0 on exact fits at the machine noise floor.
    let mut dw_num_acc = NeumaierSum::new();
    for i in 1..n {
        let d = residuals[i] - residuals[i - 1];
        dw_num_acc.add(d * d);
    }
    let dw_num = dw_num_acc.total().max(0.0);
    let dw_den = dw_den_acc.total().max(0.0);
    let exact_fit_floor = (n as f64) * (64.0 * f64::EPSILON * max_term_scale).powi(2);
    let durbin_watson = if dw_den > exact_fit_floor { (dw_num / dw_den).clamp(0.0, 4.0) } else { 2.0 };
    (fitted, residuals, durbin_watson, exact_fit_floor)
}

/// Fit Ordinary Least Squares multiple linear regression on design matrix `x` (`n x p`,
/// where column `0` is `1.0` if `has_intercept == true`) and response vector `y`.
///
/// Uses `curvebook_math::linalg::solve_linear_least_squares` (column-equilibrated pivoted QR with
/// Bjorck mixed-precision refinement and SVD fallback) for full precision on stiff/collinear
/// benchmarks (`Wampler1..5`, `Filip`, `Longley`, `NoInt1`, `NoInt2`). Householder QR least squares
/// follows Golub (1965); the refinement is Björck (1967).
pub fn multiple_linear_regression(
    x: &DenseMatrix,
    y: &[f64],
    term_names: &[String],
    has_intercept: bool,
    compute_vif: bool,
    confidence: ConfidenceLevel,
) -> Result<MultipleLinearResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n || term_names.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if p == 0 || n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "multiple linear regression", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;

    let (sol, cov_unscaled) = solve_linear_least_squares(x, y)?;
    let rank = sol.rank;
    let df_resid = n.saturating_sub(rank);
    if df_resid == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "linear regression residual df", required: rank + 1, actual: n });
    }
    let df_resid_f = df_resid as f64;
    let rss = sol.rss.max(0.0);
    let mse = rss / df_resid_f;
    let residual_sd = mse.sqrt();

    let ym = SummaryMoments::from_slice(y)?;
    let ss_total = if has_intercept {
        ym.css
    } else {
        let mut s2 = NeumaierSum::new();
        for &yi in y {
            s2.add(yi * yi);
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

    // Fitted values, residuals (FMA-compensated Dot2; Ogita, Rump & Oishi 2005), and Durbin–Watson (1950).
    let (fitted, residuals, durbin_watson, exact_fit_floor) = compute_ols_residuals_and_dw(x, y, &sol.coefficients);

    // Leverage h_i = x_i^T (X^T X)^+ x_i and Cook's distance D_i.
    let mut leverage = Vec::with_capacity(n);
    let mut cooks_distance = Vec::with_capacity(n);
    let p_eff = (rank as f64).max(1.0);
    for (r, &res) in residuals.iter().enumerate() {
        let mut h = 0.0_f64;
        for j in 0..p {
            let xrj = x.get(r, j).unwrap_or(0.0);
            for k in 0..p {
                h += xrj * cov_unscaled.get(j, k).unwrap_or(0.0) * x.get(r, k).unwrap_or(0.0);
            }
        }
        let hc = h.clamp(0.0, 0.999_999);
        leverage.push(hc);
        let denom = p_eff * mse * (1.0 - hc).powi(2);
        let cook = if denom > 0.0 { (res * res * hc) / denom } else { 0.0 };
        cooks_distance.push(cook);
    }

    let mut aliased_flags = vec![false; p];
    for &idx in &sol.aliased_columns {
        if idx < p {
            aliased_flags[idx] = true;
        }
    }

    let vifs = if compute_vif { compute_vifs_from_matrix(x, has_intercept, &aliased_flags) } else { vec![None; p] };
    let t_crit = student_t_two_sided_isf(confidence.alpha(), df_resid_f)?;
    let exact_fit = mse <= 0.0 || rss <= exact_fit_floor;

    let mut terms = Vec::with_capacity(p);
    for j in 0..p {
        let is_aliased = aliased_flags[j];
        if is_aliased {
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

        let est = sol.coefficients[j];
        let raw_se = (mse * cov_unscaled.get(j, j).unwrap_or(0.0).max(0.0)).sqrt();
        let se = if exact_fit && rss == 0.0 { 0.0 } else { raw_se };
        let (t_stat, p_val, ci) = if se > 0.0 && !exact_fit {
            let t_v = est / se;
            let p_v = student_t_two_sided_p(t_v, df_resid_f)?;
            (t_v, p_v, (est - t_crit * se, est + t_crit * se))
        } else {
            (f64::NAN, f64::NAN, (f64::NAN, f64::NAN))
        };

        let (ss_iii, f_iii, p_iii) = if p > 1 && !(has_intercept && j == 0) {
            let x_red = drop_matrix_column(x, j)?;
            if let Ok((red_sol, _)) = solve_linear_least_squares(&x_red, y) {
                if red_sol.rank >= rank {
                    (None, None, None)
                } else {
                    let ss_drop = (red_sol.rss - rss).max(0.0);
                    if mse > 0.0 && !exact_fit {
                        let f_drop = ss_drop / mse;
                        let p_drop = f_sf(f_drop, 1.0, df_resid_f).unwrap_or(1.0);
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

    // Information criteria (AIC / AICc / BIC) with k_params = rank + 1 (including sigma^2).
    let nf = n as f64;
    let k_ic = (rank + 1) as f64;
    let n_ln_rss = nf * (rss / nf).max(1e-300).ln() + nf + nf * (2.0 * std::f64::consts::PI).ln();
    let aic = n_ln_rss + 2.0 * k_ic;
    let aicc = if nf - k_ic - 1.0 > 0.0 { aic + (2.0 * k_ic * (k_ic + 1.0)) / (nf - k_ic - 1.0) } else { f64::INFINITY };
    let bic = n_ln_rss + k_ic * nf.ln();

    let cov_beta: Vec<Vec<f64>> = (0..p)
        .map(|j| (0..p).map(|k| if aliased_flags[j] || aliased_flags[k] { 0.0 } else { mse * cov_unscaled.get(j, k).unwrap_or(0.0) }).collect())
        .collect();

    Ok(MultipleLinearResult {
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
        leverage,
        cooks_distance,
        cov_beta,
    })
}

fn drop_matrix_columns(x: &DenseMatrix, drop_start: usize, drop_end: usize) -> Result<DenseMatrix, StatsError> {
    let nr = x.nrows();
    let nc = x.ncols();
    let drop_end = drop_end.min(nc);
    let drop_start = drop_start.min(drop_end);
    let kept = nc - (drop_end - drop_start);
    let mut data = Vec::with_capacity(nr * kept);
    for r in 0..nr {
        for c in 0..nc {
            if c < drop_start || c >= drop_end {
                data.push(x.get(r, c).unwrap_or(0.0));
            }
        }
    }
    Ok(DenseMatrix::from_row_major(nr, kept, data)?)
}

fn drop_matrix_column(x: &DenseMatrix, drop_col: usize) -> Result<DenseMatrix, StatsError> {
    drop_matrix_columns(x, drop_col, drop_col.saturating_add(1))
}

fn compute_vifs_from_matrix(x: &DenseMatrix, has_intercept: bool, aliased: &[bool]) -> Vec<Option<f64>> {
    let n = x.nrows();
    let p = x.ncols();
    let mut vifs = vec![None; p];
    let start = usize::from(has_intercept);
    let active_preds = (start..p).filter(|&c| !aliased.get(c).copied().unwrap_or(false)).count();
    if active_preds <= 1 {
        return vifs;
    }
    for (j, slot) in vifs.iter_mut().enumerate().skip(start) {
        if aliased.get(j).copied().unwrap_or(false) {
            continue;
        }
        let mut target = Vec::with_capacity(n);
        let retained_cols: Vec<usize> = (0..p).filter(|&c| c != j && !aliased.get(c).copied().unwrap_or(false)).collect();
        if retained_cols.len() <= start {
            continue;
        }
        let extra_intercept = usize::from(!has_intercept);
        let xm_ncols = retained_cols.len() + extra_intercept;
        let mut pred_cols = Vec::with_capacity(n * xm_ncols);
        for r in 0..n {
            target.push(x.get(r, j).unwrap_or(0.0));
            if !has_intercept {
                pred_cols.push(1.0);
            }
            for &c in &retained_cols {
                pred_cols.push(x.get(r, c).unwrap_or(0.0));
            }
        }
        let Ok(xm) = DenseMatrix::from_row_major(n, xm_ncols, pred_cols) else { continue };
        let Ok(tm) = SummaryMoments::from_slice(&target) else { continue };
        if tm.css <= 0.0 {
            continue;
        }
        if let Ok((sol, _)) = solve_linear_least_squares(&xm, &target) {
            let r2 = (1.0 - sol.rss / tm.css).clamp(0.0, 0.999_999_999);
            *slot = Some(1.0 / (1.0 - r2));
        }
    }
    vifs
}

// ============================================================================
// Multiple Logistic Regression (IRLS + Separation Safeguard)
// ============================================================================

fn eval_logistic_ll(x: &DenseMatrix, y_binary: &[f64], b: &[f64], ridge: f64) -> (f64, Vec<f64>, Vec<f64>) {
    let n = y_binary.len();
    let mut ll = 0.0_f64;
    let mut probs = Vec::with_capacity(n);
    let mut weights = Vec::with_capacity(n);
    for (r, &yr) in y_binary.iter().enumerate() {
        let mut eta = 0.0_f64;
        for (c, &bc) in b.iter().enumerate() {
            eta += x.get(r, c).unwrap_or(0.0) * bc;
        }
        eta = eta.clamp(-30.0, 30.0);
        let mu = 1.0 / (1.0 + (-eta).exp());
        let muc = mu.clamp(1e-12, 1.0 - 1e-12);
        ll += yr * muc.ln() + (1.0 - yr) * (1.0 - muc).ln();
        probs.push(mu);
        weights.push((muc * (1.0 - muc)).max(1e-12));
    }
    if ridge > 0.0 {
        let pen: f64 = b.iter().skip(1).map(|&v| v * v).sum();
        ll -= 0.5 * ridge * pen;
    }
    (ll, probs, weights)
}

fn has_logistic_separation(x: &DenseMatrix, y_binary: &[f64], dir: &[f64]) -> bool {
    let p = dir.len();
    let mut eta_min = f64::INFINITY;
    let mut eta_max = f64::NEG_INFINITY;
    let mut min_1 = f64::INFINITY;
    let mut max_1 = f64::NEG_INFINITY;
    let mut min_0 = f64::INFINITY;
    let mut max_0 = f64::NEG_INFINITY;
    for (r, &yr) in y_binary.iter().enumerate() {
        let mut eta = 0.0_f64;
        for (c, &dc) in dir.iter().enumerate().take(p) {
            eta += x.get(r, c).unwrap_or(0.0) * dc;
        }
        if !eta.is_finite() {
            return false;
        }
        eta_min = eta_min.min(eta);
        eta_max = eta_max.max(eta);
        if yr == 1.0 {
            min_1 = min_1.min(eta);
            max_1 = max_1.max(eta);
        } else {
            min_0 = min_0.min(eta);
            max_0 = max_0.max(eta);
        }
    }
    let span = eta_max - eta_min;
    if !span.is_finite() || span <= 1.0e-12 {
        return false;
    }
    let tol = 1.0e-12 * span;
    (min_1 >= max_0 - tol && max_1 > min_0 + tol) || (min_0 >= max_1 - tol && max_0 > min_1 + tol)
}

fn standardize_glm_design(x: &DenseMatrix) -> Result<(DenseMatrix, Vec<f64>, Vec<f64>, bool), StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    let nf = n as f64;
    let has_const_intercept = p > 0 && (0..n).all(|r| (x.get(r, 0).unwrap_or(0.0) - 1.0).abs() <= 1.0e-14);
    let mut means = vec![0.0_f64; p];
    let mut scales = vec![1.0_f64; p];
    let start = usize::from(has_const_intercept);
    for j in start..p {
        let col: Vec<f64> = (0..n).map(|r| x.get(r, j).unwrap_or(0.0)).collect();
        if has_const_intercept {
            let mom = SummaryMoments::from_slice(&col)?;
            let max_dev = col.iter().map(|&v| (v - mom.mean).abs()).fold(0.0_f64, f64::max);
            let sd = if max_dev > 0.0 {
                let ss_scaled: f64 = col
                    .iter()
                    .map(|&v| {
                        let d = (v - mom.mean) / max_dev;
                        d * d
                    })
                    .sum();
                max_dev * (ss_scaled / nf).sqrt()
            } else {
                0.0
            };
            means[j] = mom.mean;
            scales[j] = if sd.is_finite() && sd > 0.0 { sd } else { 1.0 };
        } else {
            let max_abs = col.iter().map(|&v| v.abs()).fold(0.0_f64, f64::max);
            let rms = if max_abs > 0.0 {
                let ss_scaled: f64 = col
                    .iter()
                    .map(|&v| {
                        let d = v / max_abs;
                        d * d
                    })
                    .sum();
                max_abs * (ss_scaled / nf).sqrt()
            } else {
                0.0
            };
            scales[j] = if rms.is_finite() && rms > 0.0 { rms } else { 1.0 };
        }
    }
    let mut z_flat = Vec::with_capacity(n * p);
    for r in 0..n {
        for j in 0..p {
            let raw = x.get(r, j).unwrap_or(0.0);
            z_flat.push((raw - means[j]) / scales[j]);
        }
    }
    let z_mat = DenseMatrix::from_row_major(n, p, z_flat)?;
    Ok((z_mat, means, scales, has_const_intercept))
}

fn transform_cov_from_standardized(cov_z: &DenseMatrix, means: &[f64], scales: &[f64], has_const_intercept: bool) -> Result<DenseMatrix, StatsError> {
    let p = cov_z.nrows();
    // Jacobian J mapping standardized beta_z to raw beta = J * beta_z:
    // J[j, j] = 1 / scales[j]; if has_const_intercept, J[0, j] = -means[j] / scales[j] (j >= 1).
    let mut tmp = vec![0.0_f64; p * p];
    for col in 0..p {
        if has_const_intercept && p > 0 {
            let mut acc = cov_z.get(0, col).unwrap_or(0.0);
            for j in 1..p {
                acc -= (means[j] / scales[j]) * cov_z.get(j, col).unwrap_or(0.0);
            }
            tmp[col] = acc;
        } else if p > 0 {
            tmp[col] = cov_z.get(0, col).unwrap_or(0.0) / scales[0];
        }
        for row in 1..p {
            tmp[row * p + col] = cov_z.get(row, col).unwrap_or(0.0) / scales[row];
        }
    }
    let mut out = vec![0.0_f64; p * p];
    for row in 0..p {
        if has_const_intercept && p > 0 {
            let mut acc = tmp[row * p];
            for k in 1..p {
                acc -= tmp[row * p + k] * (means[k] / scales[k]);
            }
            out[row * p] = acc;
        } else if p > 0 {
            out[row * p] = tmp[row * p] / scales[0];
        }
        for col in 1..p {
            out[row * p + col] = tmp[row * p + col] / scales[col];
        }
    }
    for i in 0..p {
        for j in (i + 1)..p {
            let sym = 0.5 * (out[i * p + j] + out[j * p + i]);
            out[i * p + j] = sym;
            out[j * p + i] = sym;
        }
    }
    Ok(DenseMatrix::from_row_major(p, p, out)?)
}

fn center_logistic_design(x: &DenseMatrix, means: &[f64], has_const_intercept: bool) -> Result<DenseMatrix, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    let mut c_flat = Vec::with_capacity(n * p);
    for r in 0..n {
        for j in 0..p {
            let raw = x.get(r, j).unwrap_or(0.0);
            let shift = if has_const_intercept && j > 0 { means[j] } else { 0.0 };
            c_flat.push(raw - shift);
        }
    }
    Ok(DenseMatrix::from_row_major(n, p, c_flat)?)
}

fn uncenter_beta(beta_c: &[f64], means: &[f64], scales: &[f64], has_const_intercept: bool) -> Vec<f64> {
    let p = beta_c.len();
    let mut beta = vec![0.0_f64; p];
    let start = usize::from(has_const_intercept);
    for j in start..p {
        beta[j] = beta_c[j] / scales[j];
    }
    if has_const_intercept && p > 0 {
        let mut intercept = beta_c[0];
        for j in 1..p {
            intercept -= beta[j] * means[j];
        }
        beta[0] = intercept;
    }
    beta
}

fn run_logistic_irls_solver(x: &DenseMatrix, y_binary: &[f64], beta: &mut [f64], ridge: f64, max_iters: usize) -> Result<(bool, bool), StatsError> {
    let p = beta.len();
    let (mut ll_curr, mut probs, mut weights) = eval_logistic_ll(x, y_binary, beta, ridge);
    let mut converged = false;
    let mut cholesky_failed = false;

    for _iter in 0..max_iters {
        let mut grad = vec![0.0_f64; p];
        let mut info = vec![0.0_f64; p * p];
        for (r, (&yr, &pr)) in y_binary.iter().zip(probs.iter()).enumerate() {
            let diff = yr - pr;
            let wr = weights[r];
            for j in 0..p {
                let xrj = x.get(r, j).unwrap_or(0.0);
                grad[j] += xrj * diff;
                for k in j..p {
                    info[j * p + k] += wr * xrj * x.get(r, k).unwrap_or(0.0);
                }
            }
        }
        if ridge > 0.0 {
            for j in 1..p {
                grad[j] -= ridge * beta[j];
                info[j * p + j] += ridge;
            }
        }
        for j in 0..p {
            for k in 0..j {
                info[j * p + k] = info[k * p + j];
            }
        }

        let info_mat = DenseMatrix::from_row_major(p, p, info)?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad)?
        } else {
            cholesky_failed = true;
            if ridge <= 0.0 {
                break;
            }
            let (ls, _) = solve_linear_least_squares(&info_mat, &grad)?;
            ls.coefficients
        };

        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_step > 3.0 { 3.0 / max_step } else { 1.0 };
        let mut accepted = false;
        for _ in 0..16 {
            let cand: Vec<f64> = (0..p).map(|j| beta[j] + scale * step[j]).collect();
            let (ll_cand, p_cand, w_cand) = eval_logistic_ll(x, y_binary, &cand, ridge);
            if ll_cand.is_finite() && ll_cand >= ll_curr - 1.0e-10 {
                let delta = (ll_cand - ll_curr).abs();
                beta.copy_from_slice(&cand);
                ll_curr = ll_cand;
                probs = p_cand;
                weights = w_cand;
                accepted = true;
                if delta < 1.0e-9 && max_step * scale < 1.0e-7 {
                    converged = true;
                }
                break;
            }
            scale *= 0.5;
        }
        if ridge <= 0.0
            && (has_logistic_separation(x, y_binary, beta)
                || has_logistic_separation(x, y_binary, &step)
                || beta.iter().skip(1).any(|b| b.abs() > 25.0))
        {
            break;
        }
        if converged || !accepted {
            break;
        }
    }
    Ok((converged, cholesky_failed))
}

fn logistic_irls_loop(
    x: &DenseMatrix,
    z_mat: &DenseMatrix,
    means: &[f64],
    scales: &[f64],
    has_const_intercept: bool,
    y_binary: &[f64],
    beta_z: &mut [f64],
) -> Result<(Vec<f64>, bool, bool), StatsError> {
    let p = beta_z.len();
    let init_b0 = if has_const_intercept && p > 0 { beta_z[0] } else { 0.0 };

    // Phase 1: Pure unpenalized IRLS on affine-standardized coordinates so predictor unit
    // rescaling cannot trigger spurious separation or ridge bias on well-specified fits.
    let (p1_converged, p1_chol_failed) = run_logistic_irls_solver(z_mat, y_binary, beta_z, 0.0, 60)?;
    let (_, p1_probs, _) = eval_logistic_ll(z_mat, y_binary, beta_z, 0.0);
    // Only treat fitted probabilities as saturated separation when every event is near 1 AND
    // every non-event is near 0 (true perfect classification, not low/high base prevalence).
    let probs_saturated = p1_probs.iter().zip(y_binary.iter()).all(|(&pr, &yv)| if yv == 1.0 { pr > 1.0 - 1.0e-4 } else { pr < 1.0e-4 });
    let start = usize::from(has_const_intercept);
    let std_slope_diverged = beta_z.iter().skip(start).any(|b| b.abs() > 25.0);
    let separated = p1_chol_failed || std_slope_diverged || probs_saturated || has_logistic_separation(z_mat, y_binary, beta_z);

    if !separated {
        let beta_p1 = uncenter_beta(beta_z, means, scales, has_const_intercept);
        return Ok((beta_p1, p1_converged, false));
    }

    // Phase 2: Separation confirmed geometrically or by Fisher collapse — solve penalized IRLS
    // on centered predictors and report unpenalized non-convergence alongside the safeguard flag.
    let c_mat = center_logistic_design(x, means, has_const_intercept)?;
    let mut beta_c = vec![0.0_f64; p];
    if has_const_intercept && p > 0 {
        beta_c[0] = init_b0;
    }
    let _ = run_logistic_irls_solver(&c_mat, y_binary, &mut beta_c, 1.0e-2, 60)?;
    let unit_scales = vec![1.0_f64; p];
    let beta_p2 = uncenter_beta(&beta_c, means, &unit_scales, has_const_intercept);
    if has_const_intercept && p > 0 {
        beta_z[0] = beta_c[0];
    }
    for j in start..p {
        beta_z[j] = beta_c[j] * scales[j];
    }
    Ok((beta_p2, false, true))
}

/// Fit binary Multiple Logistic Regression (`y in {0, 1}`) via IRLS Newton–Raphson with
/// backtracking line search and mild ridge/Firth-like safeguard when complete or quasi-complete
/// separation occurs.
///
/// IRLS for the binomial GLM (McCullagh & Nelder 1989); separation per Albert & Anderson
/// (1984); Hosmer & Lemeshow (1980) decile goodness of fit.
pub fn multiple_logistic_regression(
    x: &DenseMatrix,
    y_binary: &[f64],
    term_names: &[String],
    compute_vif: bool,
    confidence: ConfidenceLevel,
) -> Result<MultipleLogisticResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y_binary.len() != n || term_names.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y_binary.len() }));
    }
    if n <= p || p == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "GLM regression", required: p + 1, actual: n });
    }
    let mut n_events = 0_usize;
    for &yv in y_binary {
        if yv != 0.0 && yv != 1.0 {
            return Err(StatsError::InvalidParameter { name: "logistic_y", value: yv });
        }
        if yv == 1.0 {
            n_events += 1;
        }
    }
    if n_events == 0 || n_events == n {
        return Err(StatsError::InvalidParameter { name: "logistic_events", value: n_events as f64 });
    }
    let (z_mat, means, scales, has_const_intercept) = standardize_glm_design(x)?;
    let z_rank = svd(&z_mat)?.rank();
    if z_rank < p {
        return Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank: z_rank, expected_rank: p }));
    }

    let nf = n as f64;
    let p_bar = (n_events as f64) / nf;
    let ll_null = (n_events as f64) * p_bar.ln() + ((n - n_events) as f64) * (1.0 - p_bar).ln();
    let dev_null = -2.0 * ll_null;
    let w_null = p_bar * (1.0 - p_bar);
    let diag_null_z = fisher_null_diagonal(&z_mat, w_null);

    let mut beta_z = vec![0.0_f64; p];
    if has_const_intercept {
        beta_z[0] = (p_bar / (1.0 - p_bar)).ln();
    }
    let (beta, converged, separation_detected) = logistic_irls_loop(x, &z_mat, &means, &scales, has_const_intercept, y_binary, &mut beta_z)?;

    // Recompute unpenalized log-likelihood & Fisher information covariance matrix I_z(theta)^{-1}
    // in standardized coordinates (McCullagh & Nelder 1989, §2.5 & §4.3; Albert & Anderson 1984)
    // and back-transform Cov(beta) = J I_z^{-1} J^T so shifted predictors never fail conditioning.
    let (ll_unpen, probs_final, _weights_final) = eval_logistic_ll(&z_mat, y_binary, &beta_z, 0.0);
    let mut info_z_final = vec![0.0_f64; p * p];
    for (r, &pr) in probs_final.iter().enumerate() {
        let wr = (pr * (1.0 - pr)).max(0.0);
        for j in 0..p {
            let zrj = z_mat.get(r, j).unwrap_or(0.0);
            for k in j..p {
                info_z_final[j * p + k] += wr * zrj * z_mat.get(r, k).unwrap_or(0.0);
            }
        }
    }
    for j in 0..p {
        for k in 0..j {
            info_z_final[j * p + k] = info_z_final[k * p + j];
        }
    }
    let info_z_mat = DenseMatrix::from_row_major(p, p, info_z_final)?;
    let cov_z = invert_glm_fisher_information(&info_z_mat, &diag_null_z, 1e-8)?;
    let cov_beta = transform_cov_from_standardized(&cov_z, &means, &scales, has_const_intercept)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let vifs = if compute_vif { compute_vifs_from_matrix(x, true, &vec![false; p]) } else { vec![None; p] };
    let mut terms = Vec::with_capacity(p);
    for j in 0..p {
        let se = cov_beta.get(j, j).unwrap_or(0.0).max(0.0).sqrt();
        let z_stat = if se > 0.0 { beta[j] / se } else { 0.0 };
        let p_val = norm_two_sided_p(z_stat);
        let or = beta[j].exp();
        let or_ci = ((beta[j] - z_crit * se).exp(), (beta[j] + z_crit * se).exp());
        terms.push(GlmTermEstimate {
            name: term_names[j].clone(),
            beta: beta[j],
            se,
            z_statistic: z_stat,
            p_value: p_val,
            exp_beta: or,
            exp_beta_ci: or_ci,
            vif: vifs[j],
        });
    }

    let dev_resid = (-2.0 * ll_unpen).max(0.0);
    let lr_chi2 = (dev_null - dev_resid).max(0.0);
    let lr_df = (p - 1).max(1) as f64;
    let lr_p = chi2_sf(lr_chi2, lr_df)?;

    let pf = p as f64;
    let aic = dev_resid + 2.0 * pf;
    let aicc = if nf - pf - 1.0 > 0.0 { aic + (2.0 * pf * (pf + 1.0)) / (nf - pf - 1.0) } else { f64::INFINITY };
    let bic = dev_resid + pf * nf.ln();

    let mcfadden_r2 = if dev_null > 0.0 { (1.0 - dev_resid / dev_null).clamp(0.0, 1.0) } else { 0.0 };
    let cox_snell = 1.0 - (2.0 * (ll_null - ll_unpen) / nf).exp();
    let max_r2 = 1.0 - (2.0 * ll_null / nf).exp();
    let nagelkerke_r2 = if max_r2 > 0.0 { (cox_snell / max_r2).clamp(0.0, 1.0) } else { 0.0 };

    let mean_p1: f64 = probs_final.iter().zip(y_binary.iter()).filter(|(_, y)| **y == 1.0).map(|(pr, _)| *pr).sum::<f64>() / (n_events as f64);
    let mean_p0: f64 = probs_final.iter().zip(y_binary.iter()).filter(|(_, y)| **y == 0.0).map(|(pr, _)| *pr).sum::<f64>() / ((n - n_events) as f64);
    let tjur_r2 = (mean_p1 - mean_p0).clamp(0.0, 1.0);

    let (hl_chi2, hl_df, hl_p) = hosmer_lemeshow_test(y_binary, &probs_final, 10)?;

    let ctrls: Vec<f64> = probs_final.iter().zip(y_binary.iter()).filter(|(_, y)| **y == 0.0).map(|(p, _)| *p).collect();
    let cases: Vec<f64> = probs_final.iter().zip(y_binary.iter()).filter(|(_, y)| **y == 1.0).map(|(p, _)| *p).collect();
    let roc_auc = roc_curve_analysis(&ctrls, &cases, RocCiMethod::DeLong, confidence).ok().map(|r| r.auc);

    Ok(MultipleLogisticResult {
        n,
        n_events,
        converged,
        separation_detected,
        terms,
        log_lik_null: ll_null,
        log_lik_model: ll_unpen,
        deviance_null: dev_null,
        deviance_residual: dev_resid,
        lr_chi2,
        lr_df,
        lr_p,
        aic,
        aicc,
        bic,
        mcfadden_r2,
        nagelkerke_r2,
        tjur_r2,
        hosmer_lemeshow_chi2: hl_chi2,
        hosmer_lemeshow_df: hl_df,
        hosmer_lemeshow_p: hl_p,
        roc_auc,
        predicted_probs: probs_final,
    })
}

fn hosmer_lemeshow_test(y: &[f64], probs: &[f64], g_groups: usize) -> Result<(f64, f64, f64), StatsError> {
    let n = y.len();
    let mut pairs: Vec<(f64, f64)> = probs.iter().copied().zip(y.iter().copied()).collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Group tied fitted probabilities before quantile binning so row input ordering can never
    // split identical probabilities across neighboring deciles.
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

    if blocks.len() < 3 {
        return Ok((f64::NAN, f64::NAN, f64::NAN));
    }

    let g = g_groups.min(blocks.len()).max(2);
    let mut chi2 = 0.0_f64;
    let mut actual_bins = 0_usize;
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
        let mut nb_usize = 0_usize;

        while block_idx < blocks.len() {
            let blk = &blocks[block_idx];
            if nb_usize > 0 {
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
            nb_usize += blk.count;
            cum_count += blk.count;
            block_idx += 1;
        }

        let nb = nb_usize as f64;
        let e1 = exp1.total();
        let o1 = obs1.total();
        let pi_bar = (e1 / nb).clamp(1e-12, 1.0 - 1e-12);
        let denom = nb * pi_bar * (1.0 - pi_bar);
        if denom > 0.0 {
            chi2 += (o1 - e1).powi(2) / denom;
            actual_bins += 1;
        }
    }
    if actual_bins < 3 {
        return Ok((f64::NAN, f64::NAN, f64::NAN));
    }
    let df = (actual_bins - 2) as f64;
    let p_val = chi2_sf(chi2, df)?;
    Ok((chi2, df, p_val))
}

fn fisher_null_diagonal(x: &DenseMatrix, w_null: f64) -> Vec<f64> {
    let n = x.nrows();
    let p = x.ncols();
    let mut diag = vec![0.0_f64; p];
    for (j, slot) in diag.iter_mut().enumerate() {
        let mut ss = NeumaierSum::new();
        for r in 0..n {
            let xrj = x.get(r, j).unwrap_or(0.0);
            ss.add(xrj * xrj);
        }
        *slot = w_null * ss.total();
    }
    diag
}

/// Invert the observed/expected GLM Fisher information matrix `I(beta) = X^T W X` (McCullagh &
/// Nelder 1989, §2.5), returning [`curvebook_math::MathError::SingularMatrix`] whenever `I(beta)`
/// is rank-deficient or suffers Fisher-weight collapse relative to the intercept-only baseline.
fn invert_glm_fisher_information(info_mat: &DenseMatrix, diag_null: &[f64], diag_floor_ratio: f64) -> Result<DenseMatrix, StatsError> {
    let p = info_mat.nrows();
    if let Ok(chol) = cholesky(info_mat) {
        let well_conditioned = (0..p).all(|j| {
            let dn = diag_null.get(j).copied().unwrap_or(0.0);
            let dc = info_mat.get(j, j).unwrap_or(0.0);
            let piv = chol.l().get(j, j).unwrap_or(0.0);
            let piv_sq = piv * piv;
            dn.is_finite() && dn > 0.0 && dc.is_finite() && dc > diag_floor_ratio * dn && piv.is_finite() && piv_sq > 1e-10 * dc
        });
        if well_conditioned && let Ok(inv) = chol.inverse() {
            return Ok(inv);
        }
    }
    let eff_rank = svd(info_mat).map(|s| s.rank().min(p.saturating_sub(1))).unwrap_or(0);
    Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank: eff_rank, expected_rank: p }))
}

// ============================================================================
// Multiple Poisson Regression (Log-Link IRLS)
// ============================================================================

/// Fit count-data Multiple Poisson Regression (`y >= 0`, log link) via IRLS with backtracking
/// line search, Rate Ratios (`exp(beta)`), deviance, and Pearson dispersion check.
///
/// IRLS for the Poisson log-linear GLM (McCullagh & Nelder 1989).
pub fn multiple_poisson_regression(
    x: &DenseMatrix,
    y_counts: &[f64],
    term_names: &[String],
    compute_vif: bool,
    confidence: ConfidenceLevel,
) -> Result<MultiplePoissonResult, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y_counts.len() != n || term_names.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y_counts.len() }));
    }
    if n <= p || p == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "GLM regression", required: p + 1, actual: n });
    }
    for &yc in y_counts {
        if !yc.is_finite() || yc < 0.0 || yc.fract() != 0.0 {
            return Err(StatsError::InvalidParameter { name: "poisson_y", value: yc });
        }
    }
    let (z_mat, means, scales, has_const_intercept) = standardize_glm_design(x)?;
    let z_rank = svd(&z_mat)?.rank();
    if z_rank < p {
        return Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank: z_rank, expected_rank: p }));
    }

    let nf = n as f64;
    let mut y_sum = NeumaierSum::new();
    for &yc in y_counts {
        y_sum.add(yc);
    }
    let y_mean_raw = y_sum.total() / nf;
    if y_mean_raw <= 0.0 {
        return Err(StatsError::InvalidParameter { name: "poisson_events", value: 0.0 });
    }
    let y_mean = y_mean_raw.max(1e-6);
    let diag_null_z = fisher_null_diagonal(&z_mat, y_mean);
    let mut beta_z = vec![0.0_f64; p];
    if has_const_intercept {
        beta_z[0] = y_mean.ln();
    }

    let eval_poisson_half_dev = |b: &[f64]| -> Option<(f64, Vec<f64>, Vec<f64>)> {
        let mut half_dev = NeumaierSum::new();
        let mut mu_vec = Vec::with_capacity(n);
        let mut eta_vec = Vec::with_capacity(n);
        for (r, &yr) in y_counts.iter().enumerate() {
            let mut eta = 0.0_f64;
            for (c, &bc) in b.iter().enumerate() {
                eta += z_mat.get(r, c).unwrap_or(0.0) * bc;
            }
            if !eta.is_finite() || eta > 700.0 {
                return None;
            }
            let mu = eta.exp();
            if !mu.is_finite() {
                return None;
            }
            let contrib = if yr > 0.0 { yr * (yr.ln() - eta) - (yr - mu) } else { mu };
            if !contrib.is_finite() {
                return None;
            }
            half_dev.add(contrib);
            mu_vec.push(mu);
            eta_vec.push(eta);
        }
        Some((half_dev.total().max(0.0), mu_vec, eta_vec))
    };

    let (mut hd_curr, mut mus, mut etas) =
        eval_poisson_half_dev(&beta_z).ok_or(StatsError::InvalidParameter { name: "poisson_eta", value: beta_z[0] })?;
    let mut converged = false;

    for _iter in 0..80 {
        let mut grad = vec![0.0_f64; p];
        let mut info = vec![0.0_f64; p * p];
        for r in 0..n {
            let diff = y_counts[r] - mus[r];
            let wr = mus[r].max(1e-12);
            for j in 0..p {
                let zrj = z_mat.get(r, j).unwrap_or(0.0);
                grad[j] += zrj * diff;
                for k in j..p {
                    info[j * p + k] += wr * zrj * z_mat.get(r, k).unwrap_or(0.0);
                }
            }
        }
        for j in 0..p {
            for k in 0..j {
                info[j * p + k] = info[k * p + j];
            }
        }
        let info_mat = DenseMatrix::from_row_major(p, p, info)?;
        let step = if let Ok(chol) = cholesky(&info_mat) {
            chol.solve(&grad)?
        } else {
            let (ls, _) = solve_linear_least_squares(&info_mat, &grad)?;
            ls.coefficients
        };
        let max_eta_jump = (0..n).map(|r| (0..p).map(|j| z_mat.get(r, j).unwrap_or(0.0) * step[j]).sum::<f64>().abs()).fold(0.0_f64, f64::max);
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_eta_jump > 10.0 { 10.0 / max_eta_jump } else { 1.0 };
        let mut accepted = false;
        let hd_tol = 1.0e-10 * hd_curr.abs().max(1.0);
        for _ in 0..24 {
            let cand: Vec<f64> = (0..p).map(|j| beta_z[j] + scale * step[j]).collect();
            if let Some((hd_cand, mu_cand, eta_cand)) = eval_poisson_half_dev(&cand)
                && hd_cand <= hd_curr + hd_tol
            {
                let delta = (hd_cand - hd_curr).abs();
                beta_z = cand;
                hd_curr = hd_cand;
                mus = mu_cand;
                etas = eta_cand;
                accepted = true;
                if delta <= hd_tol && max_step * scale < 1.0e-7 && max_eta_jump * scale < 1.0e-7 {
                    converged = true;
                }
                break;
            }
            scale *= 0.5;
        }
        if converged || !accepted {
            break;
        }
    }

    let beta = uncenter_beta(&beta_z, &means, &scales, has_const_intercept);

    let ln_fact = |y: f64| -> f64 { curvebook_math::special::ln_gamma(y + 1.0).unwrap_or(0.0) };
    let mut ll_null_acc = NeumaierSum::new();
    let mut ll_model_acc = NeumaierSum::new();
    let mut half_dev_null_acc = NeumaierSum::new();
    let ln_y_mean = y_mean.ln();
    for (r, &yr) in y_counts.iter().enumerate() {
        let lnf = ln_fact(yr);
        ll_null_acc.add(yr * ln_y_mean - y_mean - lnf);
        ll_model_acc.add(yr * etas[r] - mus[r] - lnf);
        if yr > 0.0 {
            half_dev_null_acc.add(yr * (yr.ln() - ln_y_mean) - (yr - y_mean));
        } else {
            half_dev_null_acc.add(y_mean);
        }
    }
    let ll_null = ll_null_acc.total();
    let ll_curr = ll_model_acc.total();
    let dev_null = (2.0 * half_dev_null_acc.total()).max(0.0);
    let dev_resid = (2.0 * hd_curr).max(0.0);

    let mut info_z_final = vec![0.0_f64; p * p];
    let mut pearson_acc = NeumaierSum::new();
    for (r, (&yr, &mur)) in y_counts.iter().zip(mus.iter()).enumerate() {
        let wr_info = mur.max(0.0);
        let wr_dev = mur.max(1e-12);
        for j in 0..p {
            let zrj = z_mat.get(r, j).unwrap_or(0.0);
            for k in j..p {
                info_z_final[j * p + k] += wr_info * zrj * z_mat.get(r, k).unwrap_or(0.0);
            }
        }
        pearson_acc.add((yr - wr_dev).powi(2) / wr_dev);
    }
    let pearson_chi2 = pearson_acc.total();
    for j in 0..p {
        for k in 0..j {
            info_z_final[j * p + k] = info_z_final[k * p + j];
        }
    }
    let info_z_mat = DenseMatrix::from_row_major(p, p, info_z_final)?;
    let cov_z = invert_glm_fisher_information(&info_z_mat, &diag_null_z, 1e-8)?;
    let cov_beta = transform_cov_from_standardized(&cov_z, &means, &scales, has_const_intercept)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let vifs = if compute_vif { compute_vifs_from_matrix(x, true, &vec![false; p]) } else { vec![None; p] };
    let mut terms = Vec::with_capacity(p);
    for j in 0..p {
        let se = cov_beta.get(j, j).unwrap_or(0.0).max(0.0).sqrt();
        let z_stat = if se > 0.0 { beta[j] / se } else { 0.0 };
        let p_val = norm_two_sided_p(z_stat);
        let rr = beta[j].exp();
        let rr_ci = ((beta[j] - z_crit * se).exp(), (beta[j] + z_crit * se).exp());
        terms.push(GlmTermEstimate {
            name: term_names[j].clone(),
            beta: beta[j],
            se,
            z_statistic: z_stat,
            p_value: p_val,
            exp_beta: rr,
            exp_beta_ci: rr_ci,
            vif: vifs[j],
        });
    }

    let df_resid = (n - p) as f64;
    let dispersion = pearson_chi2 / df_resid.max(1.0);
    let lr_chi2 = (dev_null - dev_resid).max(0.0);
    let lr_df = (p - 1).max(1) as f64;
    let lr_p = chi2_sf(lr_chi2, lr_df)?;

    let pf = p as f64;
    let aic = -2.0 * ll_curr + 2.0 * pf;
    let aicc = if nf - pf - 1.0 > 0.0 { aic + (2.0 * pf * (pf + 1.0)) / (nf - pf - 1.0) } else { f64::INFINITY };
    let bic = -2.0 * ll_curr + pf * nf.ln();

    Ok(MultiplePoissonResult {
        n,
        converged,
        terms,
        log_lik_null: ll_null,
        log_lik_model: ll_curr,
        deviance_null: dev_null,
        deviance_residual: dev_resid,
        pearson_chi2,
        df_residual: df_resid,
        dispersion_pearson: dispersion,
        lr_chi2,
        lr_df,
        lr_p,
        aic,
        aicc,
        bic,
        predicted_means: mus,
    })
}

// ============================================================================
// Design Matrix Builder & ResultSheet Builder
// ============================================================================

fn validate_selected_data_sets(selected: &[usize], total: usize) -> Result<(), StatsError> {
    let mut seen = Vec::with_capacity(selected.len());
    for &idx in selected {
        if idx >= total {
            return Err(StatsError::MissingColumn { index: idx });
        }
        if seen.contains(&idx) {
            return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: idx as f64 });
        }
        seen.push(idx);
    }
    Ok(())
}

struct GlmFactorSpan {
    label: String,
    col_start: usize,
    col_end: usize,
}

enum RetainedGlmPred {
    Categorical(String),
    Numeric(f64),
}

struct GlmDesignMatrix {
    x_mat: DenseMatrix,
    y_vec: Vec<f64>,
    row_labels: Vec<String>,
    term_names: Vec<String>,
    factor_spans: Vec<GlmFactorSpan>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn cell_has_entry(cell: Option<&Cell>) -> bool {
    cell.is_some_and(|c| !c.is_empty() || c.excluded)
}

struct MvGlmRows {
    retained_rows: Vec<(usize, f64, Vec<RetainedGlmPred>)>,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn extract_mv_glm_pred(var: &VariableColumn, row: usize, n_rows: usize, is_cat: bool) -> Result<Option<RetainedGlmPred>, StatsError> {
    let cell = var.cells.get(row).ok_or(StatsError::DimensionMismatch { expected: n_rows, actual: var.cells.len() })?;
    if cell.excluded {
        return Ok(None);
    }
    if !is_cat {
        return Ok(cell.active_number().map(RetainedGlmPred::Numeric));
    }
    let tok = match &cell.value {
        curvebook_doc::CellValue::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed.to_owned()
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
        _ => return Ok(None),
    };
    Ok(Some(RetainedGlmPred::Categorical(tok)))
}

fn collect_mv_glm_rows(mv: &curvebook_doc::MultipleVariablesTable, glm_spec: &GlmSpec, is_cat: &[bool]) -> Result<MvGlmRows, StatsError> {
    let resp_var = &mv.variables[glm_spec.response_column];
    let Some(end_row) = (0..mv.row_count()).rfind(|&r| {
        cell_has_entry(resp_var.cells.get(r)) || glm_spec.predictor_columns.iter().any(|&pc| cell_has_entry(mv.variables[pc].cells.get(r)))
    }) else {
        return Ok(MvGlmRows { retained_rows: Vec::new(), skipped_rows: 0, ignored_non_numeric: Vec::new() });
    };

    let mut retained_rows = Vec::new();
    let mut skipped_rows = 0_usize;
    let mut resp_non_num = 0_usize;
    let mut pred_non_num = vec![0_usize; glm_spec.predictor_columns.len()];

    for r in 0..=end_row {
        let rc = resp_var.cells.get(r);
        let has_entry = cell_has_entry(rc) || glm_spec.predictor_columns.iter().any(|&pc| cell_has_entry(mv.variables[pc].cells.get(r)));
        if !has_entry {
            continue;
        }
        if rc.is_some_and(is_ignored_non_numeric_cell) {
            resp_non_num += 1;
        }
        for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
            if !is_cat[pos] && mv.variables[pc].cells.get(r).is_some_and(is_ignored_non_numeric_cell) {
                pred_non_num[pos] += 1;
            }
        }
        let Some(yv) = rc.and_then(|c| c.active_number()) else {
            skipped_rows += 1;
            continue;
        };
        let mut row_preds = Vec::with_capacity(glm_spec.predictor_columns.len());
        let mut ok = true;
        for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
            let pred_var = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
            let Some(pred) = extract_mv_glm_pred(pred_var, r, mv.row_count(), is_cat[pos])? else {
                ok = false;
                break;
            };
            row_preds.push(pred);
        }
        if ok {
            retained_rows.push((r + 1, yv, row_preds));
        } else {
            skipped_rows += 1;
        }
    }

    let mut ignored_non_numeric = Vec::new();
    if resp_non_num > 0 {
        ignored_non_numeric.push((resp_var.title.clone(), resp_non_num));
    }
    for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
        if pred_non_num[pos] > 0 {
            ignored_non_numeric.push((mv.variables[pc].title.clone(), pred_non_num[pos]));
        }
    }
    Ok(MvGlmRows { retained_rows, skipped_rows, ignored_non_numeric })
}

type GlmCatLevel = Option<(String, Vec<String>)>;

fn resolve_glm_categorical_levels(
    mv: &curvebook_doc::MultipleVariablesTable,
    glm_spec: &GlmSpec,
    is_cat: &[bool],
    retained_rows: &[(usize, f64, Vec<RetainedGlmPred>)],
) -> Result<Vec<GlmCatLevel>, StatsError> {
    let mut cat_levels = Vec::with_capacity(glm_spec.predictor_columns.len());
    for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
        if !is_cat[pos] {
            cat_levels.push(None);
            continue;
        }
        let var = &mv.variables[pc];
        let cells: Vec<Cell> = retained_rows
            .iter()
            .filter_map(|(_, _, preds)| match &preds[pos] {
                RetainedGlmPred::Categorical(tok) => Some(Cell::text(tok.as_str())),
                RetainedGlmPred::Numeric(_) => None,
            })
            .collect();
        let retained_col = VariableColumn {
            title: var.title.clone(),
            role: var.role,
            role_inferred: var.role_inferred,
            source: var.source.clone(),
            level_order: var.level_order.clone(),
            cells,
            display_decimals: var.display_decimals,
        };
        let mut lvls: Vec<String> = retained_col.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
        let explicit_ref = glm_spec.reference_levels.iter().rev().find(|(idx, _)| *idx == pc).map(|(_, l)| l.clone());
        let ref_lvl = match explicit_ref {
            Some(req) => {
                if !lvls.iter().any(|existing| existing == &req) {
                    return Err(StatsError::InvalidParameter { name: "reference_levels", value: pc as f64 });
                }
                Some(req)
            }
            None => lvls.first().cloned(),
        };
        if let Some(rl) = ref_lvl {
            lvls.retain(|l| l != &rl);
            cat_levels.push(Some((rl, lvls)));
        } else {
            cat_levels.push(None);
        }
    }
    Ok(cat_levels)
}

fn build_mv_glm_design(mv: &curvebook_doc::MultipleVariablesTable, selected: &[usize], glm_spec: &GlmSpec) -> Result<GlmDesignMatrix, StatsError> {
    validate_selected_data_sets(selected, mv.variables.len())?;
    if glm_spec.response_column >= mv.variables.len() {
        return Err(StatsError::MissingColumn { index: glm_spec.response_column });
    }
    if glm_spec.predictor_columns.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "GLM predictors", required: 1, actual: 0 });
    }
    for &(ref_col, _) in &glm_spec.reference_levels {
        if ref_col >= mv.variables.len() {
            return Err(StatsError::MissingColumn { index: ref_col });
        }
    }
    let mut is_cat = Vec::with_capacity(glm_spec.predictor_columns.len());
    for &pc in &glm_spec.predictor_columns {
        let var = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
        is_cat.push(var.role == VariableRole::Categorical);
    }

    let MvGlmRows { retained_rows, skipped_rows, ignored_non_numeric } = collect_mv_glm_rows(mv, glm_spec, &is_cat)?;
    let cat_levels = resolve_glm_categorical_levels(mv, glm_spec, &is_cat, &retained_rows)?;

    let mut term_names = vec![String::from("Intercept")];
    let mut factor_spans = Vec::with_capacity(glm_spec.predictor_columns.len());
    for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
        let title = &mv.variables[pc].title;
        let col_start = term_names.len();
        if let Some((ref_lvl, non_refs)) = &cat_levels[pos] {
            for lv in non_refs {
                term_names.push(format!("{title}[{lv} vs {ref_lvl}]"));
            }
        } else {
            term_names.push(title.clone());
        }
        let col_end = term_names.len();
        let col_count = col_end - col_start;
        if col_count == 1 {
            factor_spans.push(GlmFactorSpan { label: term_names[col_start].clone(), col_start, col_end });
        } else if col_count >= 2 {
            factor_spans.push(GlmFactorSpan { label: title.clone(), col_start, col_end });
        }
    }

    let n_cols = term_names.len();
    if !retained_rows.is_empty() && n_cols <= 1 {
        return Err(StatsError::ZeroVariance { context: "GLM categorical predictor has fewer than 2 levels after complete-case filtering" });
    }
    let n_obs = retained_rows.len();
    if n_obs <= n_cols {
        return Err(StatsError::InsufficientSampleSize { context: "GLM observations", required: n_cols + 1, actual: n_obs });
    }
    let mut x_rows = Vec::with_capacity(n_obs * n_cols);
    let mut y_vec = Vec::with_capacity(n_obs);
    let mut row_labels = Vec::with_capacity(n_obs);
    for (row_num, yv, preds) in retained_rows {
        row_labels.push(format!("Row {row_num}"));
        y_vec.push(yv);
        x_rows.push(1.0);
        for (pos, pred) in preds.into_iter().enumerate() {
            match (pred, &cat_levels[pos]) {
                (RetainedGlmPred::Categorical(tok), Some((_, non_refs))) => {
                    for lv in non_refs {
                        x_rows.push(if &tok == lv { 1.0 } else { 0.0 });
                    }
                }
                (RetainedGlmPred::Numeric(xv), None) => x_rows.push(xv),
                _ => {}
            }
        }
    }
    let x_mat = DenseMatrix::from_row_major(n_obs, n_cols, x_rows)?;
    Ok(GlmDesignMatrix { x_mat, y_vec, row_labels, term_names, factor_spans, skipped_rows, ignored_non_numeric })
}

fn build_xy_glm_design(xy: &curvebook_doc::XyTable, selected: &[usize], glm_spec: &GlmSpec) -> Result<GlmDesignMatrix, StatsError> {
    if xy.y_subcolumn_mode.is_pre_summarized() || matches!(&xy.x_mode, XColumnMode::Numbers { subcolumn_mode } if subcolumn_mode.is_pre_summarized())
    {
        return Err(StatsError::RequiresRawReplicates);
    }
    if xy.data_sets.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    validate_selected_data_sets(selected, xy.data_sets.len())?;
    if glm_spec.response_column >= xy.data_sets.len() {
        return Err(StatsError::MissingColumn { index: glm_spec.response_column });
    }
    let y_idx = if selected.is_empty() {
        glm_spec.response_column
    } else if selected.len() == 1 {
        let sel = selected[0];
        if glm_spec.response_column == 0 || glm_spec.response_column == sel {
            sel
        } else {
            return Err(StatsError::InvalidParameter { name: "response_column", value: glm_spec.response_column as f64 });
        }
    } else if selected.contains(&glm_spec.response_column) {
        glm_spec.response_column
    } else {
        return Err(StatsError::InvalidParameter { name: "response_column", value: glm_spec.response_column as f64 });
    };

    if glm_spec.predictor_columns.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "GLM predictors", required: 1, actual: 0 });
    }
    for &pc in &glm_spec.predictor_columns {
        if pc >= xy.data_sets.len().max(2) {
            return Err(StatsError::MissingColumn { index: pc });
        }
    }
    if glm_spec.predictor_columns.len() != 1 {
        return Err(StatsError::InvalidParameter { name: "predictor_columns", value: glm_spec.predictor_columns.len() as f64 });
    }
    let pred_col = glm_spec.predictor_columns[0];
    if pred_col >= 2 {
        return Err(StatsError::InvalidParameter { name: "predictor_columns", value: pred_col as f64 });
    }

    let ds = xy.data_sets.get(y_idx).ok_or(StatsError::MissingColumn { index: y_idx })?;
    let n_subcols = xy.y_subcolumn_mode.subcolumn_count().max(1);
    let is_gen_seq = matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. });
    let active_end = (0..xy.row_count()).rfind(|&r| {
        (!is_gen_seq && xy.x_cells.get(r).is_some_and(|s| s.iter().any(|c| cell_has_entry(Some(c)))))
            || ds.cells.get(r).is_some_and(|subs| subs.iter().any(|c| !c.is_empty() || c.excluded))
    });
    let mut x_rows = Vec::new();
    let mut y_vec = Vec::new();
    let mut row_labels = Vec::new();
    let mut skipped_rows = 0_usize;
    let mut x_non_num = 0_usize;
    let mut y_non_num = 0_usize;

    if let Some(end_row) = active_end {
        for r in 0..=end_row {
            let x_subs = if is_gen_seq { &[][..] } else { xy.x_cells.get(r).map(Vec::as_slice).unwrap_or(&[]) };
            for xc in x_subs {
                if is_ignored_non_numeric_cell(xc) {
                    x_non_num += 1;
                }
            }
            let x_explicit = !is_gen_seq && x_subs.iter().any(|c| cell_has_entry(Some(c)));
            let xv_opt = xy.x_summary_at(r)?;
            let row_len = ds.cells.get(r).map_or(0, Vec::len);
            for sub in 0..row_len {
                let yc = ds.cell(r, sub);
                if yc.is_some_and(is_ignored_non_numeric_cell) {
                    y_non_num += 1;
                }
                let y_explicit = cell_has_entry(yc);
                if let (Some(xv), Some(yv)) = (xv_opt, yc.and_then(Cell::active_number)) {
                    let lbl = if n_subcols > 1 { format!("Row {} (rep {})", r + 1, sub + 1) } else { format!("Row {}", r + 1) };
                    row_labels.push(lbl);
                    x_rows.push(1.0);
                    x_rows.push(xv);
                    y_vec.push(yv);
                } else if y_explicit || (sub == 0 && x_explicit) {
                    skipped_rows += 1;
                }
            }
        }
    }
    let n_obs = y_vec.len();
    if n_obs < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "GLM XY observations", required: 3, actual: n_obs });
    }
    let mut ignored_non_numeric = Vec::new();
    if x_non_num > 0 {
        ignored_non_numeric.push((xy.x_title.clone(), x_non_num));
    }
    if y_non_num > 0 {
        ignored_non_numeric.push((ds.title.clone(), y_non_num));
    }
    let x_mat = DenseMatrix::from_row_major(n_obs, 2, x_rows)?;
    let term_names = vec![String::from("Intercept"), xy.x_title.clone()];
    let factor_spans = vec![GlmFactorSpan { label: xy.x_title.clone(), col_start: 1, col_end: 2 }];
    Ok(GlmDesignMatrix { x_mat, y_vec, row_labels, term_names, factor_spans, skipped_rows, ignored_non_numeric })
}

/// Assemble `(X, y, row_labels, term_names, factor_spans)` from a `MultipleVariablesTable`
/// (with categorical reference-level dummy encoding derived strictly after complete-case filtering)
/// or an `XyTable`.
fn build_glm_design_with_factors(sheet: &DataSheet, spec: &AnalysisSpec, glm_spec: &GlmSpec) -> Result<GlmDesignMatrix, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => build_mv_glm_design(mv, &spec.selected_data_sets, glm_spec),
        TableArchetype::Xy(xy) => build_xy_glm_design(xy, &spec.selected_data_sets, glm_spec),
        other => Err(StatsError::IncompatibleTable { analysis: "generalized linear model", actual: other.kind() }),
    }
}

/// Complete Passing–Bablok (1983) nonparametric method-comparison regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassingBablokResult {
    /// Number of finite paired `(x_i, y_i)` method-comparison measurements (`n >= 3`).
    pub n: usize,
    /// Count of valid pairwise slopes `S_{ij}` retained after excluding exact ties (`dx == dy == 0`) and `S_{ij} == -1`.
    pub valid_slopes: usize,
    /// Passing–Bablok (1983) rank-offset $K$ counting valid pairwise slopes strictly less than `-1` (ensuring symmetry in `x`/`y` inversion).
    pub k_offset: usize,
    /// Shifted median slope $\hat{b}$ across ordered pairwise slopes at rank index offset by `k_offset`.
    pub slope: f64,
    /// Two-sided Passing–Bablok normal-quantile rank confidence interval `(lower, upper)` for `slope` at configured `ConfidenceLevel`.
    pub slope_ci: (f64, f64),
    /// Median intercept $\hat{a} = \mathrm{median}_i(y_i - \hat{b} x_i)$.
    pub intercept: f64,
    /// Two-sided quantile confidence interval `median_i(y_i - slope_ci.1 x_i)..median_i(y_i - slope_ci.0 x_i)` for `intercept` (Passing & Bablok 1983).
    pub intercept_ci: (f64, f64),
    /// Raw maximum absolute cumulative sum `H = max_k |S_k|` across ordered residuals.
    pub cusum_h: f64,
    /// Standardized CUSUM statistic `H / sqrt(L + l)` (Passing & Bablok 1983, §3.3) distributed as Kolmogorov supremum.
    pub cusum_statistic: f64,
    /// Two-sided asymptotic Kolmogorov CUSUM linearity *p*-value.
    pub cusum_p_value: f64,
    /// True when `cusum_p_value >= confidence.alpha()` (no significant departure from linearity detected by CUSUM test at configured confidence level).
    pub linearity_passed: bool,
    /// True if asymptotic CI index offset `C_gamma` exceeded the bounds of ordered slopes and endpoints were clamped to extreme observed slopes.
    pub slope_ci_clamped: bool,
    /// Vertical Passing–Bablok regression residuals `y_i - (intercept + slope * x_i)` in input order.
    pub residuals: Vec<f64>,
}

const PASSING_BABLOK_MAX_N: usize = 10_000;

fn median_unsorted(vals: &mut [f64]) -> f64 {
    vals.sort_by(f64::total_cmp);
    let m = vals.len();
    if m % 2 == 1 { vals[m / 2] } else { 0.5 * (vals[m / 2 - 1] + vals[m / 2]) }
}

fn kolmogorov_cusum_p(lambda: f64) -> f64 {
    if !lambda.is_finite() || lambda <= 0.0 {
        return 1.0;
    }
    if lambda > 8.0 {
        return 0.0;
    }
    let l2 = lambda * lambda;
    let mut sum = 0.0_f64;
    for k in 1..=200 {
        let kf = k as f64;
        let term = (-2.0 * kf * kf * l2).exp();
        if k % 2 == 1 {
            sum += term;
        } else {
            sum -= term;
        }
        if term <= 1e-16 * sum.abs().max(1e-300) {
            break;
        }
    }
    (2.0 * sum).clamp(0.0, 1.0)
}

fn passing_bablok_cusum(x: &[f64], y: &[f64], slope: f64, intercept: f64, residuals: &[f64]) -> (f64, f64, f64) {
    let scale_y = y.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
    let scale_x = x.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
    let tol = 1e-12 * (1.0 + scale_y + (slope * scale_x).abs());

    let pos_count = residuals.iter().filter(|&&r| r > tol).count();
    let neg_count = residuals.iter().filter(|&&r| r < -tol).count();
    if pos_count == 0 || neg_count == 0 {
        return (0.0, 0.0, 1.0);
    }

    let pos_score = ((neg_count as f64) / (pos_count as f64)).sqrt();
    let neg_score = -((pos_count as f64) / (neg_count as f64)).sqrt();
    let denom = (1.0 + slope * slope).sqrt();

    let mut proj_scores: Vec<(f64, f64)> = Vec::with_capacity(x.len());
    let mut d_max_abs = 0.0_f64;
    for i in 0..x.len() {
        let d_i = (x[i] + slope * (y[i] - intercept)) / denom;
        d_max_abs = d_max_abs.max(d_i.abs());
        let score = if residuals[i] > tol {
            pos_score
        } else if residuals[i] < -tol {
            neg_score
        } else {
            0.0
        };
        proj_scores.push((d_i, score));
    }
    proj_scores.sort_by(|a, b| a.0.total_cmp(&b.0));

    let d_tol = 1e-12 * (1.0 + d_max_abs);
    let mut cum = 0.0_f64;
    let mut h_raw = 0.0_f64;
    let mut idx = 0_usize;
    while idx < proj_scores.len() {
        let d0 = proj_scores[idx].0;
        let mut grp = proj_scores[idx].1;
        idx += 1;
        while idx < proj_scores.len() && (proj_scores[idx].0 - d0).abs() <= d_tol {
            grp += proj_scores[idx].1;
            idx += 1;
        }
        cum += grp;
        h_raw = h_raw.max(cum.abs());
    }

    // Passing & Bablok (1983, J. Clin. Chem. Clin. Biochem. 21:709-720, §3.3):
    // H_std = H / sqrt(L + l), where L = pos_count and l = neg_count.
    let h_std = h_raw / ((pos_count + neg_count) as f64).sqrt();
    let p_val = kolmogorov_cusum_p(h_std);
    (h_raw, h_std, p_val)
}

/// Passing–Bablok (1983) nonparametric method-comparison regression (`Y = a + b X`) with
/// exact `K`-offset slope estimator, rank-based slope/intercept confidence intervals, and
/// Kolmogorov-type CUSUM linearity test.
pub fn passing_bablok_regression(x: &[f64], y: &[f64], conf: ConfidenceLevel) -> Result<PassingBablokResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::DimensionMismatch { expected: x.len(), actual: y.len() });
    }
    validate_finite_slice(x)?;
    validate_finite_slice(y)?;
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Passing-Bablok regression", required: 3, actual: n });
    }
    if n > PASSING_BABLOK_MAX_N {
        return Err(StatsError::InvalidParameter { name: "passing_bablok_max_n", value: n as f64 });
    }

    // Passing & Bablok (1983, J. Clin. Chem. Clin. Biochem. 21:709-720, eq. 2):
    // Identical pairs (dx == 0 && dy == 0) and exact -1 slopes are omitted, whereas vertical
    // pairs (dx == 0 && dy != 0) carry canonical undirected slope +inf (angle +pi/2, order-invariant,
    // not counted in K as < -1).
    let mut slopes = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[j] - x[i];
            let dy = y[j] - y[i];
            if dx == 0.0 {
                if dy == 0.0 {
                    continue;
                }
                slopes.push(f64::INFINITY);
                continue;
            }
            let s = dy / dx;
            if (s + 1.0).abs() <= 1e-12 {
                continue;
            }
            slopes.push(s);
        }
    }
    if slopes.is_empty() {
        return Err(StatsError::ZeroVariance { context: "Passing-Bablok pairwise slopes" });
    }

    slopes.sort_by(f64::total_cmp);
    let k_offset = slopes.iter().take_while(|&&s| s < -1.0).count();
    let n_slopes = slopes.len();
    let mut angular = Vec::with_capacity(n_slopes);
    angular.extend_from_slice(&slopes[k_offset..]);
    angular.extend_from_slice(&slopes[..k_offset]);

    let slope = if n_slopes % 2 == 1 {
        angular[n_slopes / 2]
    } else {
        let left = angular[n_slopes / 2 - 1];
        let right = angular[n_slopes / 2];
        if !left.is_finite() || !right.is_finite() {
            return Err(StatsError::ZeroVariance { context: "Passing-Bablok vertical median slope" });
        }
        0.5 * (left + right)
    };
    if !slope.is_finite() {
        return Err(StatsError::ZeroVariance { context: "Passing-Bablok vertical median slope" });
    }

    let z_crit = norm_two_sided_isf(conf.alpha())?;
    let nf = n as f64;
    let c_alpha = z_crit * (nf * (nf - 1.0) * (2.0 * nf + 5.0) / 18.0).sqrt();
    let m1_raw = (((n_slopes as f64) - c_alpha) / 2.0).round() as isize;
    let slope_ci_clamped = m1_raw < 1;
    let m1 = m1_raw.max(1).min(n_slopes as isize) as usize;
    let m2 = (n_slopes + 1 - m1).max(m1).min(n_slopes);
    let raw_lo = angular[m1 - 1];
    let raw_hi = angular[m2 - 1];

    let mut tmp = Vec::with_capacity(n);
    for (&xi, &yi) in x.iter().zip(y.iter()) {
        tmp.push(yi - slope * xi);
    }
    let intercept = median_unsorted(&mut tmp);

    let (slope_ci, intercept_ci) = if raw_lo.is_finite() && raw_hi.is_finite() && raw_lo <= slope && slope <= raw_hi {
        tmp.clear();
        for (&xi, &yi) in x.iter().zip(y.iter()) {
            tmp.push(yi - raw_hi * xi);
        }
        let a1 = median_unsorted(&mut tmp);
        tmp.clear();
        for (&xi, &yi) in x.iter().zip(y.iter()) {
            tmp.push(yi - raw_lo * xi);
        }
        let a2 = median_unsorted(&mut tmp);
        ((raw_lo, raw_hi), (a1.min(a2), a1.max(a2)))
    } else {
        ((-f64::INFINITY, f64::INFINITY), (-f64::INFINITY, f64::INFINITY))
    };

    let residuals: Vec<f64> = x.iter().zip(y.iter()).map(|(&xi, &yi)| yi - (intercept + slope * xi)).collect();
    let (cusum_h, cusum_statistic, cusum_p_value) = passing_bablok_cusum(x, y, slope, intercept, &residuals);
    let linearity_passed = cusum_p_value >= conf.alpha();

    Ok(PassingBablokResult {
        n,
        valid_slopes: n_slopes,
        k_offset,
        slope,
        slope_ci,
        intercept,
        intercept_ci,
        cusum_h,
        cusum_statistic,
        cusum_p_value,
        linearity_passed,
        slope_ci_clamped,
        residuals,
    })
}

struct PassingBablokInput {
    xs: Vec<f64>,
    ys: Vec<f64>,
    row_labels: Vec<String>,
    x_label: String,
    y_label: String,
    skipped_rows: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

#[derive(Default)]
struct PbRawAccumulator {
    xs: Vec<f64>,
    ys: Vec<f64>,
    row_labels: Vec<String>,
    skipped_rows: usize,
    x_non_num: usize,
    y_non_num: usize,
}

impl PbRawAccumulator {
    fn into_input(self, x_label: String, y_label: String, same_column: bool) -> Result<PassingBablokInput, StatsError> {
        if self.xs.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let mut ignored_non_numeric = Vec::new();
        if self.x_non_num > 0 {
            ignored_non_numeric.push((x_label.clone(), self.x_non_num));
        }
        if self.y_non_num > 0 && !same_column {
            ignored_non_numeric.push((y_label.clone(), self.y_non_num));
        }
        Ok(PassingBablokInput {
            xs: self.xs,
            ys: self.ys,
            row_labels: self.row_labels,
            x_label,
            y_label,
            skipped_rows: self.skipped_rows,
            ignored_non_numeric,
        })
    }
}

fn extract_pb_single_col_pairs<'a>(
    n_rows: usize,
    x_at: impl Fn(usize) -> Option<&'a Cell>,
    y_at: impl Fn(usize) -> Option<&'a Cell>,
    x_label: String,
    y_label: String,
    same_column: bool,
) -> Result<PassingBablokInput, StatsError> {
    let mut acc = PbRawAccumulator::default();
    if let Some(end_row) = (0..n_rows).rfind(|&r| cell_has_entry(x_at(r)) || cell_has_entry(y_at(r))) {
        for r in 0..=end_row {
            let ca = x_at(r);
            let cb = y_at(r);
            if !cell_has_entry(ca) && !cell_has_entry(cb) {
                continue;
            }
            if ca.is_some_and(is_ignored_non_numeric_cell) {
                acc.x_non_num += 1;
            }
            if cb.is_some_and(is_ignored_non_numeric_cell) {
                acc.y_non_num += 1;
            }
            if let (Some(xv), Some(yv)) = (ca.and_then(Cell::active_number), cb.and_then(Cell::active_number)) {
                acc.xs.push(xv);
                acc.ys.push(yv);
                acc.row_labels.push(format!("Row {}", r + 1));
            } else {
                acc.skipped_rows += 1;
            }
        }
    }
    acc.into_input(x_label, y_label, same_column)
}

fn extract_pb_xy_pairs(t: &curvebook_doc::XyTable, selected: &[usize], x_col: usize, y_col: usize) -> Result<PassingBablokInput, StatsError> {
    if t.y_subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    validate_selected_data_sets(selected, t.data_sets.len())?;
    let total_cols = 1 + t.data_sets.len();
    if x_col >= total_cols {
        return Err(StatsError::MissingColumn { index: x_col });
    }
    if y_col >= total_cols {
        return Err(StatsError::MissingColumn { index: y_col });
    }
    let x_is_presum = matches!(&t.x_mode, XColumnMode::Numbers { subcolumn_mode } if subcolumn_mode.is_pre_summarized());
    if (x_col == 0 || y_col == 0) && x_is_presum {
        return Err(StatsError::RequiresRawReplicates);
    }
    let x_raw_subs = match &t.x_mode {
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(k) } if *k > 1 => *k,
        _ => 1,
    };
    let y_subs = t.y_subcolumn_mode.subcolumn_count().max(1);
    let n_subs = if x_col > 0 && y_col > 0 { y_subs } else { y_subs.max(x_raw_subs) };
    let is_gen_seq = matches!(t.x_mode, XColumnMode::GeneratedSequence { .. });

    let cell_for = |row: usize, sub: usize, col: usize| -> Option<&Cell> {
        if col == 0 { if x_raw_subs > 1 { t.x_cells.get(row).and_then(|s| s.get(sub)) } else { None } } else { t.data_sets[col - 1].cell(row, sub) }
    };
    let col_has_entry_row = |row: usize, col: usize| -> bool {
        if col == 0 {
            !is_gen_seq && t.x_cells.get(row).is_some_and(|subs| subs.iter().any(|c| !c.is_empty() || c.excluded))
        } else {
            t.data_sets[col - 1].cells.get(row).is_some_and(|subs| subs.iter().any(|c| !c.is_empty() || c.excluded))
        }
    };

    let mut acc = PbRawAccumulator::default();
    if let Some(end_row) = (0..t.row_count()).rfind(|&r| col_has_entry_row(r, x_col) || col_has_entry_row(r, y_col)) {
        for r in 0..=end_row {
            let shared_x_non_num =
                x_raw_subs == 1 && !is_gen_seq && t.x_cells.get(r).and_then(|s| s.first()).is_some_and(is_ignored_non_numeric_cell);
            acc.x_non_num += usize::from(x_col == 0 && shared_x_non_num);
            acc.y_non_num += usize::from(y_col == 0 && x_col != 0 && shared_x_non_num);
            for sub in 0..n_subs {
                let cx = cell_for(r, sub, x_col);
                let cy = cell_for(r, sub, y_col);
                acc.x_non_num += usize::from(cx.is_some_and(is_ignored_non_numeric_cell));
                acc.y_non_num += usize::from(cy.is_some_and(is_ignored_non_numeric_cell) && !(x_col == y_col && x_col > 0));
                let xv = if x_col == 0 && x_raw_subs == 1 { t.x_summary_at(r)? } else { cx.and_then(Cell::active_number) };
                let yv = if y_col == 0 && x_raw_subs == 1 { t.x_summary_at(r)? } else { cy.and_then(Cell::active_number) };
                if let (Some(x_val), Some(y_val)) = (xv, yv) {
                    acc.xs.push(x_val);
                    acc.ys.push(y_val);
                    acc.row_labels.push(if n_subs > 1 { format!("Row {} (rep {})", r + 1, sub + 1) } else { format!("Row {}", r + 1) });
                } else {
                    let shared_x_entry = !is_gen_seq && sub == 0 && cell_has_entry(t.x_cells.get(r).and_then(|s| s.first()));
                    let x_entry = if x_col == 0 && x_raw_subs == 1 { shared_x_entry } else { cell_has_entry(cx) };
                    let y_entry = if y_col == 0 && x_raw_subs == 1 { shared_x_entry } else { cell_has_entry(cy) };
                    acc.skipped_rows += usize::from(x_entry || y_entry);
                }
            }
        }
    }
    let label_for = |col: usize| -> String { if col == 0 { t.x_title.clone() } else { t.data_sets[col - 1].title.clone() } };
    acc.into_input(label_for(x_col), label_for(y_col), x_col == y_col)
}

fn extract_passing_bablok_pairs(sheet: &DataSheet, selected: &[usize], x_col: usize, y_col: usize) -> Result<PassingBablokInput, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(t) => {
            if t.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            validate_selected_data_sets(selected, t.data_sets.len())?;
            let cx = t.data_sets.get(x_col).ok_or(StatsError::MissingColumn { index: x_col })?;
            let cy = t.data_sets.get(y_col).ok_or(StatsError::MissingColumn { index: y_col })?;
            extract_pb_single_col_pairs(t.row_count(), |r| cx.cell(r, 0), |r| cy.cell(r, 0), cx.title.clone(), cy.title.clone(), x_col == y_col)
        }
        TableArchetype::MultipleVariables(t) => {
            validate_selected_data_sets(selected, t.variables.len())?;
            let vx = t.variables.get(x_col).ok_or(StatsError::MissingColumn { index: x_col })?;
            let vy = t.variables.get(y_col).ok_or(StatsError::MissingColumn { index: y_col })?;
            extract_pb_single_col_pairs(t.row_count(), |r| vx.cells.get(r), |r| vy.cells.get(r), vx.title.clone(), vy.title.clone(), x_col == y_col)
        }
        TableArchetype::Xy(t) => extract_pb_xy_pairs(t, selected, x_col, y_col),
        other => Err(StatsError::IncompatibleTable { analysis: "Passing-Bablok regression", actual: other.kind() }),
    }
}

/// Execute a [`PassingBablokSpec`] against a `Column`, `Xy`, or `MultipleVariables` [`DataSheet`]
/// using the caller's [`AnalysisSpec`] metadata.
pub fn run_passing_bablok_analysis(sheet: &DataSheet, spec: &AnalysisSpec, pb_spec: &PassingBablokSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::PassingBablok(*pb_spec))?;
    let mut out = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let PassingBablokInput { xs, ys, row_labels: resid_labels, x_label, y_label, skipped_rows, ignored_non_numeric } =
        extract_passing_bablok_pairs(sheet, &spec.selected_data_sets, pb_spec.x_column, pb_spec.y_column)?;
    let res = passing_bablok_regression(&xs, &ys, conf)?;

    push_scalar(&mut out, "Slope", None, res.slope);
    push_scalar(&mut out, "Slope CI lower", None, res.slope_ci.0);
    push_scalar(&mut out, "Slope CI upper", None, res.slope_ci.1);
    push_scalar(&mut out, "Intercept", None, res.intercept);
    push_scalar(&mut out, "Intercept CI lower", None, res.intercept_ci.0);
    push_scalar(&mut out, "Intercept CI upper", None, res.intercept_ci.1);
    push_scalar(&mut out, "CUSUM H", None, res.cusum_h);
    push_scalar(&mut out, "CUSUM statistic", None, res.cusum_statistic);
    push_scalar(&mut out, "CUSUM P value", None, res.cusum_p_value);
    push_scalar(&mut out, "N", None, res.n as f64);
    push_scalar(&mut out, "Valid slopes", None, res.valid_slopes as f64);
    push_scalar(&mut out, "K offset", None, res.k_offset as f64);

    let prop_ok = res.slope_ci.0.is_finite() && res.slope_ci.1.is_finite() && (res.slope_ci.0..=res.slope_ci.1).contains(&1.0);
    let sys_ok = res.intercept_ci.0.is_finite() && res.intercept_ci.1.is_finite() && (res.intercept_ci.0..=res.intercept_ci.1).contains(&0.0);

    out.tabs.push(
        TabBuilder::new(
            "pb-parameters",
            format!("Passing-Bablok regression ({y_label} vs {x_label})"),
            ResultTabKind::ParametersAndConfidenceIntervals,
        )
        .headers([String::from("Estimate"), format!("{}% CI", conf.percent()), String::from("Identity check")])
        .row(
            "Slope (b)",
            [
                num_cell(res.slope),
                ci_cell(res.slope_ci.0, res.slope_ci.1),
                text_cell(if prop_ok { "CI includes 1 (no proportional bias)" } else { "CI excludes 1 (proportional bias)" }),
            ],
        )
        .row(
            "Intercept (a)",
            [
                num_cell(res.intercept),
                ci_cell(res.intercept_ci.0, res.intercept_ci.1),
                text_cell(if sys_ok { "CI includes 0 (no systematic bias)" } else { "CI excludes 0 (systematic bias)" }),
            ],
        )
        .build(),
    );

    out.tabs.push(
        KvTabBuilder::new("pb-summary", "Passing-Bablok linearity & slope counts", ResultTabKind::MainSummary, "Value")
            .num("Paired observations (n)", res.n as f64)
            .num("Valid pairwise slopes (N)", res.valid_slopes as f64)
            .num("Negative-slope offset (K)", res.k_offset as f64)
            .num("CUSUM linearity H", res.cusum_h)
            .num("CUSUM standardized H / sqrt(L + l)", res.cusum_statistic)
            .p("CUSUM linearity P value", res.cusum_p_value, p_fmt, conf.alpha())
            .build(),
    );

    let mut resid_tab = TabBuilder::new("pb-residuals", "Passing-Bablok residuals", ResultTabKind::Residuals).headers([
        x_label,
        y_label,
        String::from("Fitted Y"),
        String::from("Residual (Y - a - bX)"),
    ]);
    for (lbl, ((&xv, &yv), &rv)) in resid_labels.into_iter().zip(xs.iter().zip(ys.iter()).zip(res.residuals.iter())) {
        resid_tab.push_row(lbl, [num_cell(xv), num_cell(yv), num_cell(res.intercept + res.slope * xv), num_cell(rv)]);
    }
    out.tabs.push(resid_tab.build());

    let x_min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let x_max = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if x_min.is_finite() && x_max.is_finite() {
        let n_pts = 65_usize;
        let mut pts = Vec::with_capacity(n_pts);
        for i in 0..n_pts {
            let t = (i as f64) / ((n_pts - 1) as f64);
            let xv = x_min + t * (x_max - x_min);
            let yv = res.intercept + res.slope * xv;
            let (ci_lo, ci_hi) =
                if res.slope_ci.0.is_finite() && res.slope_ci.1.is_finite() && res.intercept_ci.0.is_finite() && res.intercept_ci.1.is_finite() {
                    let y1 = res.intercept_ci.0 + res.slope_ci.0 * xv;
                    let y2 = res.intercept_ci.0 + res.slope_ci.1 * xv;
                    let y3 = res.intercept_ci.1 + res.slope_ci.0 * xv;
                    let y4 = res.intercept_ci.1 + res.slope_ci.1 * xv;
                    (Some(y1.min(y2).min(y3).min(y4)), Some(y1.max(y2).max(y3).max(y4)))
                } else {
                    (None, None)
                };
            pts.push(CurveSamplePoint { x: xv, y: yv, ci_lower: ci_lo, ci_upper: ci_hi, pred_lower: None, pred_upper: None });
        }
        out.fitted_curves.push(FittedCurveOverlay {
            data_set_index: 0,
            equation_id: String::from("passing_bablok"),
            converged: true,
            ambiguous: false,
            points: pts,
        });
    }

    out.checklist.push(checklist_item(
        "Linearity assumption (Passing-Bablok CUSUM)",
        if res.linearity_passed { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("H = {:.3}, z_H = {:.3}, P = {:.4}", res.cusum_h, res.cusum_statistic, res.cusum_p_value)),
        if res.linearity_passed {
            "CUSUM test shows no significant deviation from linearity between the two methods."
        } else {
            "Significant deviation from linearity detected by CUSUM; Passing-Bablok linear parameters may misrepresent nonlinear bias."
        },
    ));
    out.checklist.push(checklist_item(
        "Method equivalence against Y = X",
        if prop_ok && sys_ok { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("slope = {:.4}, intercept = {:.4}", res.slope, res.intercept)),
        if prop_ok && sys_ok {
            "Slope CI includes 1 and intercept CI includes 0, indicating no evidence of proportional or constant systematic bias."
        } else {
            "At least one confidence interval excludes the identity line (slope != 1 or intercept != 0), indicating method bias."
        },
    ));
    if res.slope_ci_clamped {
        out.checklist.push(checklist_item(
            "Sample size for rank confidence interval",
            ChecklistStatus::Warn,
            Some(format!("n = {}, N = {} valid slopes", res.n, res.valid_slopes)),
            "Sample size is too small for the requested exact rank-based confidence width; CI endpoints were clamped to observed extreme slopes.",
        ));
    }

    append_listwise_filtered_checklist_and_footnote(&mut out, "Passing-Bablok regression", skipped_rows, xs.len());
    append_ignored_text_checklist_and_footnote(&mut out, "Passing-Bablok regression", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

/// Execute Passing–Bablok nonparametric regression against a [`DataSheet`] at confidence `conf`.
pub fn execute_passing_bablok(sheet: &DataSheet, spec: &PassingBablokSpec, conf: ConfidenceLevel) -> Result<ResultSheet, StatsError> {
    let mut analysis = AnalysisSpec::new(SheetId::new("passing-bablok"), sheet.id.clone(), AnalysisKind::PassingBablok(*spec))?;
    analysis.confidence_level = conf;
    run_passing_bablok_analysis(sheet, &analysis, spec)
}

fn append_linear_type_iii_tab(
    out: &mut ResultSheet,
    res: &MultipleLinearResult,
    x_mat: &DenseMatrix,
    y_vec: &[f64],
    factor_spans: &[GlmFactorSpan],
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<(), StatsError> {
    let mut type_iii_tab = TabBuilder::new("glm-linear-type-iii", "Type III partial sums of squares", ResultTabKind::AnovaTable)
        .headers(["SS", "df", "MS", "F", "P value"]);
    let mut row_count = 0_usize;
    let mse = res.residual_sd * res.residual_sd;
    for span in factor_spans {
        let col_count = span.col_end.saturating_sub(span.col_start);
        if col_count == 0 {
            continue;
        }
        if col_count == 1 {
            if let Some(t) = res.terms.get(span.col_start)
                && let (Some(ss), Some(f_val), Some(p_val)) = (t.type_iii_ss, t.type_iii_f, t.type_iii_p)
            {
                type_iii_tab
                    .push_row(span.label.clone(), [num_cell(ss), num_cell(1.0), num_cell(ss), num_cell(f_val), p_cell_at_alpha(p_val, p_fmt, alpha)]);
                row_count += 1;
            }
            continue;
        }
        let x_red = drop_matrix_columns(x_mat, span.col_start, span.col_end)?;
        if let Ok((red_sol, _)) = solve_linear_least_squares(&x_red, y_vec) {
            let df_factor = res.rank.saturating_sub(red_sol.rank) as f64;
            if df_factor <= 0.0 {
                continue;
            }
            let ss_drop = (red_sol.rss - res.rss).max(0.0);
            let ms_drop = ss_drop / df_factor;
            let f_drop = if mse > 0.0 {
                ms_drop / mse
            } else if ms_drop > 0.0 {
                f64::INFINITY
            } else {
                0.0
            };
            let p_drop = f_sf(f_drop, df_factor, res.df_residual).unwrap_or(1.0);
            type_iii_tab.push_row(
                span.label.clone(),
                [num_cell(ss_drop), num_cell(df_factor), num_cell(ms_drop), num_cell(f_drop), p_cell_at_alpha(p_drop, p_fmt, alpha)],
            );
            row_count += 1;
        }
    }
    if row_count > 0 {
        out.tabs.push(type_iii_tab.build());
    }
    Ok(())
}

fn populate_linear_glm_sheet(
    out: &mut ResultSheet,
    design: GlmDesignMatrix,
    glm_spec: &GlmSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<(), StatsError> {
    let res = multiple_linear_regression(&design.x_mat, &design.y_vec, &design.term_names, true, glm_spec.compute_vif, confidence)?;
    push_scalar(out, "R squared", None, res.r_squared);
    push_scalar(out, "Adjusted R squared", None, res.adjusted_r_squared);
    push_scalar(out, "Residual SD (Sy.x)", None, res.residual_sd);
    push_scalar(out, "F statistic", None, res.f_statistic);
    push_scalar(out, "P value", None, res.f_p_value);
    push_scalar(out, "AIC", None, res.aic);
    push_scalar(out, "AICc", None, res.aicc);
    push_scalar(out, "Durbin-Watson", None, res.durbin_watson);

    let mut headers =
        vec![String::from("Estimate"), String::from("SE"), String::from("t"), String::from("P value"), format!("{}% CI", confidence.percent())];
    if glm_spec.compute_vif {
        headers.push(String::from("VIF"));
    }
    let mut params_tab =
        TabBuilder::new("glm-linear-params", "Multiple linear regression parameters", ResultTabKind::ParametersAndConfidenceIntervals)
            .headers(headers);
    let mut labels = Vec::with_capacity(res.terms.len());
    let mut aliased_names = Vec::new();
    let mut exact_fit_terms = false;
    for (idx, t) in res.terms.iter().enumerate() {
        labels.push(t.name.clone());
        let mut r = if res.aliased_columns.get(idx).copied().unwrap_or(false) {
            aliased_names.push(t.name.clone());
            vec![text_cell("Aliased"), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty, ResultCell::Empty]
        } else {
            if !t.t_statistic.is_finite() {
                exact_fit_terms = true;
            }
            vec![
                num_cell(t.estimate),
                num_cell(t.se),
                num_cell(t.t_statistic),
                p_cell_at_alpha(t.p_value, p_fmt, confidence.alpha()),
                finite_ci_cell(t.ci.0, t.ci.1),
            ]
        };
        if glm_spec.compute_vif {
            r.push(opt_num_cell(t.vif));
        }
        params_tab.push_row(t.name.clone(), r);
    }
    if !aliased_names.is_empty() {
        params_tab
            .push_footnote(format!("Aliased predictor(s) omitted from estimation due to exact linear dependence: {}.", aliased_names.join(", ")));
    }
    if exact_fit_terms {
        params_tab.push_footnote(
            "Exact linear fit (zero residual sum of squares): coefficient t statistics, P values, and confidence intervals are unavailable.",
        );
    }
    out.tabs.push(params_tab.build());

    out.tabs.push(
        TabBuilder::new("glm-linear-anova", "ANOVA of regression", ResultTabKind::AnovaTable)
            .headers(["SS", "df", "MS", "F", "P value"])
            .row(
                "Regression",
                [
                    num_cell(res.ss_regression),
                    num_cell(res.df_model),
                    num_cell(res.ss_regression / res.df_model.max(1.0)),
                    num_cell(res.f_statistic),
                    p_cell_at_alpha(res.f_p_value, p_fmt, confidence.alpha()),
                ],
            )
            .row(
                "Residual",
                [num_cell(res.rss), num_cell(res.df_residual), num_cell(res.residual_sd * res.residual_sd), ResultCell::Empty, ResultCell::Empty],
            )
            .row("Total", [num_cell(res.ss_total), num_cell((res.n - 1) as f64), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty])
            .build(),
    );

    append_linear_type_iii_tab(out, &res, &design.x_mat, &design.y_vec, &design.factor_spans, p_fmt, confidence.alpha())?;

    let mut cov_tab =
        TabBuilder::new("glm-linear-covariance", "Parameter covariance matrix", ResultTabKind::CovarianceMatrix).headers(labels.clone());
    for (lbl, row) in labels.into_iter().zip(res.cov_beta.iter()) {
        cov_tab.push_row(lbl, row.iter().map(|&v| num_cell(v)));
    }
    out.tabs.push(cov_tab.build());

    let mut resid_tab = TabBuilder::new("glm-linear-residuals", "Linear regression residuals and influence", ResultTabKind::Residuals).headers([
        "Observed Y",
        "Predicted Y",
        "Residual",
        "Leverage",
        "Cook's D",
    ]);
    for (i, (lbl, &yi)) in design.row_labels.into_iter().zip(design.y_vec.iter()).take(res.n).enumerate() {
        resid_tab.push_row(
            lbl,
            [num_cell(yi), num_cell(res.fitted[i]), num_cell(res.residuals[i]), num_cell(res.leverage[i]), num_cell(res.cooks_distance[i])],
        );
    }
    out.tabs.push(resid_tab.build());

    if !aliased_names.is_empty() {
        out.checklist.push(checklist_item(
            "Predictor collinearity (aliased terms)",
            ChecklistStatus::Warn,
            Some(format!("{} aliased term(s): {}", aliased_names.len(), aliased_names.join(", "))),
            "Design matrix is rank-deficient; aliased columns are exact linear combinations of retained predictors and marked \"Aliased\".",
        ));
    }
    let max_vif = res.terms.iter().filter_map(|t| t.vif).fold(0.0_f64, f64::max);
    if glm_spec.compute_vif && max_vif > 0.0 {
        let status = if max_vif >= 10.0 {
            ChecklistStatus::Violated
        } else if max_vif >= 5.0 {
            ChecklistStatus::Warn
        } else {
            ChecklistStatus::Pass
        };
        out.checklist.push(checklist_item(
            "Multicollinearity (VIF)",
            status,
            Some(format!("max VIF = {max_vif:.2}")),
            if max_vif >= 10.0 {
                "Severe multicollinearity detected (VIF >= 10); parameter estimates have inflated standard errors."
            } else if max_vif >= 5.0 {
                "Moderate multicollinearity detected (VIF >= 5)."
            } else {
                "Predictor collinearity is low (all VIF < 5)."
            },
        ));
    }
    let max_cook = res.cooks_distance.iter().copied().fold(0.0_f64, f64::max);
    out.checklist.push(checklist_item(
        "Influential observations (Cook's D)",
        if max_cook > 1.0 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
        Some(format!("max Cook's D = {max_cook:.3}, DW = {:.3}", res.durbin_watson)),
        if max_cook > 1.0 {
            "One or more points have Cook's D > 1.0 and strongly influence the fitted hyperplane."
        } else {
            "No extreme high-leverage influential points (all Cook's D <= 1.0)."
        },
    ));
    out.checklist.push(residual_normality_checklist_item(&res.residuals, confidence.alpha()));
    Ok(())
}

fn populate_logistic_glm_sheet(
    out: &mut ResultSheet,
    design: GlmDesignMatrix,
    glm_spec: &GlmSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<(), StatsError> {
    let res = multiple_logistic_regression(&design.x_mat, &design.y_vec, &design.term_names, glm_spec.compute_vif, confidence)?;
    push_scalar(out, "Converged", None, if res.converged { 1.0 } else { 0.0 });
    push_scalar(out, "Log-likelihood (model)", None, res.log_lik_model);
    push_scalar(out, "LR chi-square", None, res.lr_chi2);
    push_scalar(out, "LR P value", None, res.lr_p);
    push_scalar(out, "McFadden R squared", None, res.mcfadden_r2);
    push_scalar(out, "Nagelkerke R squared", None, res.nagelkerke_r2);
    push_scalar(out, "Tjur R squared", None, res.tjur_r2);
    push_scalar(out, "AIC", None, res.aic);
    push_scalar(out, "AICc", None, res.aicc);
    push_scalar(out, "Hosmer-Lemeshow chi2", None, res.hosmer_lemeshow_chi2);
    push_scalar(out, "Hosmer-Lemeshow P", None, res.hosmer_lemeshow_p);
    if let Some(auc) = res.roc_auc {
        push_scalar(out, "ROC AUC", None, auc);
    }
    let roc_ctrls: Vec<f64> = res.predicted_probs.iter().zip(design.y_vec.iter()).filter(|(_, y)| **y == 0.0).map(|(p, _)| *p).collect();
    let roc_cases: Vec<f64> = res.predicted_probs.iter().zip(design.y_vec.iter()).filter(|(_, y)| **y == 1.0).map(|(p, _)| *p).collect();
    if let Ok(roc) = roc_curve_analysis(&roc_ctrls, &roc_cases, RocCiMethod::DeLong, confidence) {
        let curve_pts: Vec<CurveSamplePoint> = roc
            .points
            .iter()
            .map(|pt| CurveSamplePoint {
                x: pt.one_minus_specificity,
                y: pt.sensitivity,
                ci_lower: None,
                ci_upper: None,
                pred_lower: None,
                pred_upper: None,
            })
            .collect();
        out.fitted_curves.push(FittedCurveOverlay {
            data_set_index: 0,
            equation_id: String::from("ROC"),
            converged: res.converged,
            ambiguous: res.separation_detected,
            points: curve_pts,
        });
    }

    let mut params_tab = build_glm_ratio_params_tab(
        "glm-logistic-params",
        "Logistic regression odds ratios",
        "Odds Ratio",
        "OR",
        &res.terms,
        glm_spec.compute_vif,
        confidence,
        p_fmt,
    );
    if res.separation_detected {
        params_tab.push_footnote("Complete or quasi-complete separation detected: unpenalized MLE did not converge; ridge safeguard applied.");
    } else if !res.converged {
        params_tab.push_footnote("IRLS did not converge within maximum iterations; parameter estimates and Wald statistics may be unreliable.");
    }
    if !res.hosmer_lemeshow_p.is_finite() {
        params_tab
            .push_footnote("Hosmer-Lemeshow goodness-of-fit test is unavailable because fewer than 3 distinct fitted-probability groups remain.");
    }
    out.tabs.push(params_tab.build());

    out.tabs.push(build_logistic_classification_tab(&design.y_vec, &res.predicted_probs));

    let mut pred_tab = TabBuilder::new("glm-logistic-predictions", "Logistic regression predicted probabilities", ResultTabKind::Residuals)
        .headers(["Observed Y", "Predicted probability", "Predicted class"]);
    for (lbl, (&yi, &pi)) in design.row_labels.into_iter().zip(design.y_vec.iter().zip(res.predicted_probs.iter())) {
        pred_tab.push_row(lbl, [num_cell(yi), num_cell(pi), num_cell(if pi >= 0.5 { 1.0 } else { 0.0 })]);
    }
    out.tabs.push(pred_tab.build());

    let hl_str = if res.hosmer_lemeshow_p.is_finite() { format!("{:.4}", res.hosmer_lemeshow_p) } else { String::from("unavailable") };
    let status = if res.separation_detected || !res.converged { ChecklistStatus::Warn } else { ChecklistStatus::Pass };
    let explanation = match (res.converged, res.separation_detected) {
        (true, false) => "IRLS converged without separation; Hosmer-Lemeshow P and ROC AUC summarize calibration and discrimination.",
        (false, true) => "Complete or quasi-complete separation detected; unpenalized IRLS did not converge and a penalized safeguard was applied.",
        (true, true) => "Separation detected; penalized safeguard applied to stabilize odds ratios.",
        (false, false) => "IRLS failed to converge within the iteration limit; parameter estimates and Wald tests may be unreliable.",
    };
    out.checklist.push(checklist_item(
        "Logistic separation & calibration",
        status,
        Some(format!("converged = {}, Hosmer-Lemeshow P = {}, AUC = {:.3}", res.converged, hl_str, res.roc_auc.unwrap_or(0.5))),
        explanation,
    ));
    Ok(())
}

fn populate_poisson_glm_sheet(
    out: &mut ResultSheet,
    design: GlmDesignMatrix,
    glm_spec: &GlmSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<(), StatsError> {
    let res = multiple_poisson_regression(&design.x_mat, &design.y_vec, &design.term_names, glm_spec.compute_vif, confidence)?;
    let mcfadden_r2 = if res.log_lik_null.abs() > 1e-12 && res.log_lik_null.is_finite() && res.log_lik_model.is_finite() {
        (1.0 - res.log_lik_model / res.log_lik_null).clamp(0.0, 1.0)
    } else {
        0.0
    };
    push_scalar(out, "Converged", None, if res.converged { 1.0 } else { 0.0 });
    push_scalar(out, "Residual deviance", None, res.deviance_residual);
    push_scalar(out, "Pearson dispersion", None, res.dispersion_pearson);
    push_scalar(out, "LR chi-square", None, res.lr_chi2);
    push_scalar(out, "LR P value", None, res.lr_p);
    push_scalar(out, "AIC", None, res.aic);
    push_scalar(out, "AICc", None, res.aicc);
    push_scalar(out, "McFadden R squared", None, mcfadden_r2);

    let mut params_tab = build_glm_ratio_params_tab(
        "glm-poisson-params",
        "Poisson regression rate ratios",
        "Rate Ratio",
        "RR",
        &res.terms,
        glm_spec.compute_vif,
        confidence,
        p_fmt,
    );
    if !res.converged {
        params_tab
            .push_footnote("Poisson IRLS did not converge within maximum iterations; parameter estimates and Wald statistics may be unreliable.");
    }
    out.tabs.push(params_tab.build());

    let mut resid_tab = TabBuilder::new("glm-poisson-residuals", "Poisson regression Pearson and deviance residuals", ResultTabKind::Residuals)
        .headers(["Observed Y", "Predicted count", "Pearson residual", "Deviance residual"]);
    for (lbl, (&yi, &mui)) in design.row_labels.into_iter().zip(design.y_vec.iter().zip(res.predicted_means.iter())) {
        let mu = mui.max(1e-12);
        let pearson = (yi - mui) / mu.sqrt();
        let dev_sq = if yi > 0.0 { 2.0 * (yi * (yi / mu).ln() - (yi - mu)) } else { 2.0 * mu };
        let dev_res = if yi >= mui { dev_sq.max(0.0).sqrt() } else { -dev_sq.max(0.0).sqrt() };
        resid_tab.push_row(lbl, [num_cell(yi), num_cell(mui), num_cell(pearson), num_cell(dev_res)]);
    }
    out.tabs.push(resid_tab.build());

    out.checklist.push(checklist_item(
        "Poisson IRLS convergence",
        if res.converged { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(if res.converged { String::from("converged") } else { String::from("did not converge") }),
        if res.converged {
            "Poisson IRLS deviance and step criteria converged within tolerance."
        } else {
            "Poisson IRLS did not converge within the iteration limit (possible zero-count boundary separation); estimates may be unreliable."
        },
    ));

    let overdispersed = res.dispersion_pearson > 1.5;
    out.checklist.push(checklist_item(
        "Equidispersion assumption (Poisson)",
        if overdispersed { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
        Some(format!("Pearson chi2 / df = {:.3}", res.dispersion_pearson)),
        if overdispersed {
            "Overdispersion detected (Pearson chi2 / df > 1.5); Poisson variance assumptions understate dispersion and standard errors."
        } else {
            "Count variance is consistent with Poisson equidispersion (dispersion <= 1.5)."
        },
    ));
    Ok(())
}

/// Execute a [`GlmSpec`] (`Linear`, `Logistic`, or `Poisson`) against a `DataSheet`.
pub fn run_glm_analysis(sheet: &DataSheet, spec: &AnalysisSpec, glm_spec: &GlmSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))?;
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let mut design = build_glm_design_with_factors(sheet, spec, glm_spec)?;
    let skipped_rows = design.skipped_rows;
    let kept_rows = design.y_vec.len();
    let ignored_non_numeric = std::mem::take(&mut design.ignored_non_numeric);

    match glm_spec.family {
        GlmFamily::Linear => populate_linear_glm_sheet(&mut out, design, glm_spec, confidence, p_fmt)?,
        GlmFamily::Logistic => populate_logistic_glm_sheet(&mut out, design, glm_spec, confidence, p_fmt)?,
        GlmFamily::Poisson => populate_poisson_glm_sheet(&mut out, design, glm_spec, confidence, p_fmt)?,
    }

    append_listwise_filtered_checklist_and_footnote(&mut out, "GLM regression", skipped_rows, kept_rows);
    append_ignored_text_checklist_and_footnote(&mut out, "GLM regression", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

/// Evaluate residual Gaussian normality for linear GLM without remediation suggestions (`QOL-7`).
fn residual_normality_checklist_item(residuals: &[f64], alpha: f64) -> curvebook_doc::ChecklistItem {
    let outcome = if residuals.len() < 3 {
        return checklist_item(
            "Residual normality",
            ChecklistStatus::Info,
            Some(format!("n = {}", residuals.len())),
            "Fewer than 3 residuals are available; normality testing requires at least 3 residuals.",
        );
    } else if residuals.len() <= 5000 {
        crate::column::shapiro_wilk(residuals, alpha)
    } else {
        crate::column::dagostino_pearson_k2(residuals, alpha)
    };
    match outcome {
        Ok(res) => checklist_item(
            "Residual normality",
            if res.passed { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("{} = {:.4}, P = {:.4}", res.test_name, res.statistic, res.p_value)),
            if res.passed {
                "Residual distribution shows no significant departure from Gaussian normality; small-sample t and F approximations hold under normal errors."
            } else {
                "Residual distribution departs from Gaussian normality (P < alpha); small-sample t-intervals and F-tests rely on asymptotic normality."
            },
        ),
        Err(StatsError::ZeroVariance { .. }) => checklist_item(
            "Residual normality",
            ChecklistStatus::Info,
            Some(String::from("exact fit (zero residual variance)")),
            "Regression residuals have zero variance (exact model fit); normality test statistics are degenerate.",
        ),
        Err(err) => checklist_item("Residual normality", ChecklistStatus::Info, None, format!("Residual normality diagnostic unavailable: {err}.")),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_glm_ratio_params_tab(
    id: &str,
    title: &str,
    ratio_col: &str,
    ratio_abbr: &str,
    terms: &[GlmTermEstimate],
    compute_vif: bool,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> TabBuilder {
    let mut headers = vec![
        String::from("beta"),
        String::from("SE"),
        String::from("Wald z"),
        String::from("P value"),
        String::from(ratio_col),
        format!("{}% CI ({ratio_abbr})", confidence.percent()),
    ];
    if compute_vif {
        headers.push(String::from("VIF"));
    }
    let mut tab = TabBuilder::new(id, title, ResultTabKind::ParametersAndConfidenceIntervals).headers(headers);
    for t in terms {
        let mut r = vec![
            num_cell(t.beta),
            num_cell(t.se),
            num_cell(t.z_statistic),
            p_cell_at_alpha(t.p_value, p_fmt, confidence.alpha()),
            num_cell(t.exp_beta),
            finite_ci_cell(t.exp_beta_ci.0, t.exp_beta_ci.1),
        ];
        if compute_vif {
            r.push(opt_num_cell(t.vif));
        }
        tab.push_row(t.name.clone(), r);
    }
    tab
}

/// Build logistic classification summary (cutoff 0.50) and cutoff-accuracy sweep (`0.1..=0.9`).
fn build_logistic_classification_tab(y: &[f64], probs: &[f64]) -> curvebook_doc::ResultTab {
    fn metrics_row(y: &[f64], probs: &[f64], cutoff: f64) -> [ResultCell; 10] {
        let mut tp = 0_usize;
        let mut fp = 0_usize;
        let mut fn_cnt = 0_usize;
        let mut tn = 0_usize;
        for (&yi, &pi) in y.iter().zip(probs.iter()) {
            let obs_pos = yi > 0.5;
            let pred_pos = pi >= cutoff;
            match (obs_pos, pred_pos) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (true, false) => fn_cnt += 1,
                (false, false) => tn += 1,
            }
        }
        let sens = (tp + fn_cnt > 0).then(|| (tp as f64) / ((tp + fn_cnt) as f64));
        let spec = (tn + fp > 0).then(|| (tn as f64) / ((tn + fp) as f64));
        let ppv = (tp + fp > 0).then(|| (tp as f64) / ((tp + fp) as f64));
        let npv = (tn + fn_cnt > 0).then(|| (tn as f64) / ((tn + fn_cnt) as f64));
        let acc = ((tp + tn) as f64) / (y.len().max(1) as f64);
        [
            num_cell(cutoff),
            num_cell(tp as f64),
            num_cell(fp as f64),
            num_cell(fn_cnt as f64),
            num_cell(tn as f64),
            opt_num_cell(sens),
            opt_num_cell(spec),
            opt_num_cell(ppv),
            opt_num_cell(npv),
            num_cell(acc),
        ]
    }

    let mut tab = TabBuilder::new("glm-logistic-classification", "Logistic classification table & cutoff sweep", ResultTabKind::Diagnostics)
        .headers(["Cutoff", "TP", "FP", "FN", "TN", "Sensitivity", "Specificity", "PPV", "NPV", "Accuracy"])
        .row("Cutoff 0.50 (2x2)", metrics_row(y, probs, 0.5));
    for k in 1..=9 {
        let cutoff = (k as f64) * 0.1;
        tab.push_row(format!("Sweep cutoff {cutoff:.1}"), metrics_row(y, probs, cutoff));
    }
    tab.build()
}
