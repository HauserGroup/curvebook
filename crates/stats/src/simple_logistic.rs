//! Standalone simple logistic regression on single-subcolumn `XY` tables with binary outcomes
//! $Y_i \in \{0, 1\}$.
//!
//! Fits the single-predictor binomial logit model
//! $\ln\!\left(\frac{P(Y=1 \mid X)}{1 - P(Y=1 \mid X)}\right) = \beta_0 + \beta_1 X$
//! via Newton–Raphson / Fisher scoring IRLS, computes asymptotic standard errors, Wald $Z$-tests,
//! likelihood-ratio $G^2$ test, exact profile-likelihood confidence intervals for $\beta_0$,
//! $\beta_1$, odds ratios $e^{\beta_0}, e^{\beta_1}$, and $X_{50} = -\beta_0 / \beta_1$,
//! goodness-of-fit metrics (deviance, $\text{AIC}_c$, Tjur's $R^2$, Cox–Snell $R^2$), empirical
//! ROC/AUC with DeLong SE/CI, row-level classification and out-of-sample `(X_new, blank Y)`
//! predictions, sigmoid probability curve overlays, and explicit collinearity / separation
//! diagnostics.
//!
//! References:
//! - Hosmer, D. W., Lemeshow, S., & Sturdivant, R. X. (2013). *Applied Logistic Regression*
//!   (3rd ed.). Hoboken, NJ: Wiley. `doi:10.1002/9781118548387`.
//! - McCullagh, P., & Nelder, J. A. (1989). *Generalized Linear Models* (2nd ed.). London:
//!   Chapman & Hall.
//! - Tjur, T. (2009). Coefficients of determination in logistic regression models—A new proposal:
//!   The coefficient of discrimination. *The American Statistician*, 63(4), 366-372.
//!   `doi:10.1198/tast.2009.08210`.
//! - Albert, A., & Anderson, J. A. (1984). On the existence of maximum likelihood estimates in
//!   logistic regression models. *Biometrika*, 71(1), 1-10. `doi:10.1093/biomet/71.1.1`.
//! - Heinze, G., & Schemper, M. (2002). A solution to the problem of separation in logistic
//!   regression. *Statistics in Medicine*, 21(16), 2409-2419. `doi:10.1002/sim.1047`.
//! - Venzon, D. J., & Moolgavkar, S. H. (1988). A method for computing profile-likelihood-based
//!   confidence intervals. *Journal of the Royal Statistical Society: Series C*, 37(1), 87-94.
//!   `doi:10.2307/2347496`.
//! - DeLong, E. R., DeLong, D. M., & Clarke-Pearson, D. L. (1988). Comparing the areas under two
//!   or more correlated receiver operating characteristic curves: A nonparametric approach.
//!   *Biometrics*, 44(3), 837-845. `doi:10.2307/2531595`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellValue, ChecklistStatus, CiBound, CurveSamplePoint, DataSheet, FittedCurveOverlay, ResultCell, ResultSheet,
    ResultTabKind, SimpleLogisticSpec, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, norm_two_sided_isf, norm_two_sided_p};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, ci_cell, enforce_summary_mode_matrix, init_result_sheet,
    num_cell, opt_num_cell, p_cell_at_alpha, push_scalar, sanitize_result_sheet_floats, text_cell, try_resolve_selected_indices,
};
use crate::error::StatsError;
use crate::roc::{RocCiMethod, RocResult, roc_curve_analysis};

// ============================================================================
// Typed Outputs
// ============================================================================

/// Exact geometric separation and collinearity classification for 1D binary logistic regression
/// (Albert & Anderson 1984; Heinze & Schemper 2002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SimpleLogisticSeparation {
    /// Predictor ranges for $Y=0$ and $Y=1$ strictly overlap; a unique finite MLE exists.
    None,
    /// Predictor $X$ is constant across all observations (collinear with the intercept).
    Collinear,
    /// All $Y=1$ observations lie strictly on one side of all $Y=0$ observations ($\hat{\beta}_1 \to \pm\infty$).
    CompleteSeparation,
    /// $Y=0$ and $Y=1$ ranges meet only at a single boundary $X$ value without strictly overlapping ($\hat{\beta}_1 \to \pm\infty$).
    QuasiCompleteSeparation,
}

impl SimpleLogisticSeparation {
    /// Short human-readable status label for tables and summaries.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None (overlap; finite MLE exists)",
            Self::Collinear => "Collinearity (constant X)",
            Self::CompleteSeparation => "Perfect (complete) separation",
            Self::QuasiCompleteSeparation => "Quasi-perfect separation",
        }
    }

    /// Plain-language diagnostic explanation citing Albert & Anderson (1984) and Heinze & Schemper (2002).
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::None => "Predictor ranges for Y=0 and Y=1 overlap; Newton-Raphson IRLS converged to a unique finite MLE.",
            Self::Collinear => {
                "All observed X values are identical, making X collinear with the intercept; slope beta_1 is unidentifiable (Heinze & Schemper 2002)."
            }
            Self::CompleteSeparation => {
                "Perfect (complete) separation detected: a single X threshold separates Y=0 and Y=1 with zero overlap, so the unpenalized MLE diverges to infinity (Albert & Anderson 1984; Heinze & Schemper 2002). Parameter estimates are suppressed rather than reporting diverging values."
            }
            Self::QuasiCompleteSeparation => {
                "Quasi-perfect separation detected: Y=0 and Y=1 meet only at a single boundary X value without strictly overlapping, so the unpenalized MLE diverges to infinity (Albert & Anderson 1984; Heinze & Schemper 2002). Parameter estimates are suppressed rather than reporting diverging values."
            }
        }
    }
}

/// Asymmetrical profile-likelihood confidence interval whose lower or upper bound may be open
/// (unbounded) when the profile log-likelihood does not cross the critical threshold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfileCi {
    /// Finite lower confidence limit, or `None` when unbounded below.
    pub lower: Option<f64>,
    /// Finite upper confidence limit, or `None` when unbounded above.
    pub upper: Option<f64>,
}

impl ProfileCi {
    /// Unbounded interval `(-Inf, +Inf)`.
    pub const OPEN: Self = Self { lower: None, upper: None };

    /// Convert to a [`ResultCell::ConfidenceInterval`] preserving open bounds cleanly.
    #[must_use]
    pub fn to_result_cell(self) -> ResultCell {
        let lower = match self.lower {
            Some(v) if v.is_finite() => CiBound::Finite(v),
            _ => CiBound::Open,
        };
        let upper = match self.upper {
            Some(v) if v.is_finite() => CiBound::Finite(v),
            _ => CiBound::Open,
        };
        ResultCell::ConfidenceInterval { lower, upper }
    }
}

/// Row-level classification and predicted-probability record for observed `(X_i, Y_i)` and
/// appended out-of-sample `(X_new, blank Y)` rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimpleLogisticRowPrediction {
    /// Zero-based row index in the source `DataSheet`.
    pub row_index: usize,
    /// Predictor value $X_i$ (or $X_{\text{new}}$).
    pub x: f64,
    /// Observed binary outcome $Y_i \in \{0, 1\}$, or `None` for appended `(X_new, blank Y)` rows.
    pub observed_y: Option<f64>,
    /// Predicted event probability $\hat{P}(Y=1 \mid X) = \sigma(\hat{\beta}_0 + \hat{\beta}_1 X)$.
    pub predicted_prob: f64,
    /// Inverse-logit two-sided confidence interval `(lower, upper)` in `(0, 1)` for $P(Y=1 \mid X)$.
    pub prob_ci: (f64, f64),
    /// Predicted binary class (`1` if `predicted_prob >= cutoff`, else `0`).
    pub predicted_class: u8,
    /// `true` when this row had a blank `Y` cell and was automatically predicted out-of-sample.
    pub is_unknown_row: bool,
}

/// Complete standalone simple logistic regression output for one `XY` dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimpleLogisticResult {
    /// Number of complete `(X, Y)` observations entering the fit.
    pub n: usize,
    /// Number of positive events (`Y == 1`).
    pub n_events: usize,
    /// Number of non-events (`Y == 0`).
    pub n_non_events: usize,
    /// `true` when IRLS converged to a unique finite MLE without separation or collinearity.
    pub converged: bool,
    /// Exact collinearity / separation diagnostic.
    pub separation: SimpleLogisticSeparation,
    /// Estimated intercept $\hat{\beta}_0$ (`NaN` under separation/collinearity).
    pub beta0: f64,
    /// Estimated slope $\hat{\beta}_1$ (`NaN` under separation/collinearity).
    pub beta1: f64,
    /// Asymptotic Fisher-information standard error of $\hat{\beta}_0$.
    pub se_beta0: f64,
    /// Asymptotic Fisher-information standard error of $\hat{\beta}_1$.
    pub se_beta1: f64,
    /// Asymptotic covariance $\widehat{\mathrm{Cov}}(\hat{\beta}_0, \hat{\beta}_1)$.
    pub cov_01: f64,
    /// Wald $Z$-statistic $\hat{\beta}_0 / \mathrm{SE}(\hat{\beta}_0)$.
    pub wald_z_beta0: f64,
    /// Two-sided Wald $P$-value for $H_0: \beta_0 = 0$.
    pub wald_p_beta0: f64,
    /// Wald $Z$-statistic $\hat{\beta}_1 / \mathrm{SE}(\hat{\beta}_1)$.
    pub wald_z_beta1: f64,
    /// Two-sided Wald $P$-value for $H_0: \beta_1 = 0$.
    pub wald_p_beta1: f64,
    /// Asymmetrical profile-likelihood CI for $\beta_0$.
    pub profile_ci_beta0: ProfileCi,
    /// Asymmetrical profile-likelihood CI for $\beta_1$.
    pub profile_ci_beta1: ProfileCi,
    /// Baseline odds $e^{\hat{\beta}_0}$ at $X = 0$.
    pub odds_ratio_beta0: f64,
    /// Per-unit odds ratio $e^{\hat{\beta}_1}$.
    pub odds_ratio_beta1: f64,
    /// Propagated profile-likelihood CI for $e^{\beta_0}$.
    pub odds_ratio_ci_beta0: ProfileCi,
    /// Propagated profile-likelihood CI for $e^{\beta_1}$.
    pub odds_ratio_ci_beta1: ProfileCi,
    /// Predictor value $X_{50} = -\hat{\beta}_0 / \hat{\beta}_1$ where $\hat{P}(Y=1 \mid X_{50}) = 0.5$ (`None` when $\hat{\beta}_1 = 0$ or separated).
    pub x50: Option<f64>,
    /// Asymmetrical profile-likelihood CI for $X_{50}$ (open/unbounded when $G^2 \le \chi^2_{1, 1-\alpha}$).
    pub x50_profile_ci: ProfileCi,
    /// Null (intercept-only) log-likelihood $\ell_0$.
    pub log_lik_null: f64,
    /// Fitted model log-likelihood $\ell(\hat{\beta}_0, \hat{\beta}_1)$.
    pub log_lik_model: f64,
    /// Null deviance $D_0 = -2 \ell_0$.
    pub deviance_null: f64,
    /// Model residual deviance $D_1 = -2 \ell(\hat{\beta})$.
    pub deviance_model: f64,
    /// Likelihood-ratio test statistic $G^2 = D_0 - D_1 \sim \chi^2_1$.
    pub lr_chi2: f64,
    /// Likelihood-ratio degrees of freedom (`1.0`).
    pub lr_df: f64,
    /// Upper-tail $\chi^2_1$ likelihood-ratio $P$-value for $H_0: \beta_1 = 0$.
    pub lr_p: f64,
    /// Akaike Information Criterion $D_1 + 2k$ ($k = 2$).
    pub aic: f64,
    /// Small-sample corrected $\text{AIC}_c = \text{AIC} + \frac{2k(k+1)}{N - k - 1}$ (`+Inf` when $N \le 3$).
    pub aicc: f64,
    /// Bayesian Information Criterion $D_1 + k \ln N$.
    pub bic: f64,
    /// Tjur's (2009) coefficient of discrimination $\bar{\hat{p}}_{Y=1} - \bar{\hat{p}}_{Y=0}$.
    pub tjur_r2: f64,
    /// Cox–Snell generalized $R^2 = 1 - \exp(-G^2 / N)$.
    pub cox_snell_r2: f64,
    /// Nagelkerke normalized $R^2$.
    pub nagelkerke_r2: f64,
    /// McFadden pseudo-$R^2 = 1 - D_1 / D_0$.
    pub mcfadden_r2: f64,
    /// Empirical ROC curve and DeLong (1988) AUC with SE/CI over fitted probabilities.
    pub roc: Option<RocResult>,
    /// Classification cutoff probability in `(0, 1)`.
    pub classification_cutoff: f64,
    /// True positives at `classification_cutoff`.
    pub tp: usize,
    /// True negatives at `classification_cutoff`.
    pub tn: usize,
    /// False positives at `classification_cutoff`.
    pub fp: usize,
    /// False negatives at `classification_cutoff`.
    pub fn_count: usize,
    /// Overall classification accuracy `(TP + TN) / N`.
    pub accuracy: f64,
    /// Sensitivity `TP / (TP + FN)`.
    pub sensitivity: f64,
    /// Specificity `TN / (TN + FP)`.
    pub specificity: f64,
    /// Per-row predicted probabilities and CIs for observed rows plus appended `(X_new, blank Y)` rows.
    pub row_predictions: Vec<SimpleLogisticRowPrediction>,
}

// ============================================================================
// Core Numerics
// ============================================================================

#[inline]
fn logistic_sigmoid(eta: f64) -> f64 {
    if eta >= 0.0 {
        let z = (-eta).exp();
        1.0 / (1.0 + z)
    } else {
        let z = eta.exp();
        z / (1.0 + z)
    }
}

/// Numerically stable Bernoulli log-likelihood $\sum [y_i \eta_i - \ln(1 + e^{\eta_i})]$.
fn log_likelihood(x: &[f64], y: &[f64], beta0: f64, beta1: f64) -> f64 {
    let mut ll = 0.0_f64;
    for (&xi, &yi) in x.iter().zip(y.iter()) {
        let eta = beta0 + beta1 * xi;
        if eta >= 0.0 {
            ll += (yi - 1.0) * eta - (-eta).exp().ln_1p();
        } else {
            ll += yi * eta - eta.exp().ln_1p();
        }
    }
    ll
}

/// Detect exact collinearity, complete separation, or quasi-complete separation in 1D logistic regression.
fn detect_separation(x: &[f64], y: &[f64]) -> SimpleLogisticSeparation {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut min_x0 = f64::INFINITY;
    let mut max_x0 = f64::NEG_INFINITY;
    let mut min_x1 = f64::INFINITY;
    let mut max_x1 = f64::NEG_INFINITY;
    for (&xi, &yi) in x.iter().zip(y.iter()) {
        x_min = x_min.min(xi);
        x_max = x_max.max(xi);
        if yi == 0.0 {
            min_x0 = min_x0.min(xi);
            max_x0 = max_x0.max(xi);
        } else {
            min_x1 = min_x1.min(xi);
            max_x1 = max_x1.max(xi);
        }
    }
    if x_max == x_min {
        return SimpleLogisticSeparation::Collinear;
    }
    let scale = x_min.abs().max(x_max.abs());
    let span = x_max - x_min;
    let half_span = 0.5 * x_max - 0.5 * x_min;
    if !half_span.is_finite() || half_span <= 0.5e-12 * scale {
        return SimpleLogisticSeparation::Collinear;
    }
    let tol = if span.is_finite() { 1e-12 * span } else { 1e-12 * scale };
    if max_x0 + tol < min_x1 || max_x1 + tol < min_x0 {
        return SimpleLogisticSeparation::CompleteSeparation;
    }
    if max_x0 <= min_x1 + tol || max_x1 <= min_x0 + tol {
        return SimpleLogisticSeparation::QuasiCompleteSeparation;
    }
    SimpleLogisticSeparation::None
}

/// Standardized IRLS fit parameters retained for shift- and scale-invariant predictions,
/// log-likelihood evaluation, and profile-likelihood confidence intervals.
struct StandardizedLogisticFit {
    mean_x: f64,
    sd_x: f64,
    z: Vec<f64>,
    t0: f64,
    t1: f64,
    s00: f64,
    s01: f64,
    s11: f64,
    beta0: f64,
    beta1: f64,
    cov00: f64,
    cov01: f64,
    cov11: f64,
    converged: bool,
}

impl StandardizedLogisticFit {
    fn failed() -> Self {
        Self {
            mean_x: f64::NAN,
            sd_x: f64::NAN,
            z: Vec::new(),
            t0: f64::NAN,
            t1: f64::NAN,
            s00: f64::NAN,
            s01: f64::NAN,
            s11: f64::NAN,
            beta0: f64::NAN,
            beta1: f64::NAN,
            cov00: f64::NAN,
            cov01: f64::NAN,
            cov11: f64::NAN,
            converged: false,
        }
    }
}

/// Exact 2-parameter Newton-Raphson / Fisher scoring IRLS on standardized predictor `z = (x - mean) / sd`.
fn fit_irls_standardized(x: &[f64], y: &[f64], n_events: usize) -> StandardizedLogisticFit {
    let nf = x.len() as f64;
    let Ok(mom) = SummaryMoments::from_slice(x) else {
        return StandardizedLogisticFit::failed();
    };
    let mean_x = mom.mean;
    let max_dev = x.iter().map(|&xi| (xi - mean_x).abs()).fold(0.0_f64, f64::max);
    if !max_dev.is_finite() || max_dev <= 0.0 {
        return StandardizedLogisticFit::failed();
    }
    let var_scaled = x
        .iter()
        .map(|&xi| {
            let d = (xi - mean_x) / max_dev;
            d * d
        })
        .sum::<f64>()
        / nf;
    let sd_x = max_dev * var_scaled.sqrt();
    if !sd_x.is_finite() || sd_x <= 0.0 {
        return StandardizedLogisticFit::failed();
    }
    let z: Vec<f64> = x.iter().map(|&xi| (xi - mean_x) / sd_x).collect();

    let p_bar = (n_events as f64) / nf;
    let mut t0 = (p_bar / (1.0 - p_bar)).ln();
    let mut t1 = 0.0_f64;
    let mut ll_curr = log_likelihood(&z, y, t0, t1);
    let mut converged = false;

    for _ in 0..100 {
        let mut g0 = 0.0_f64;
        let mut g1 = 0.0_f64;
        let mut i00 = 0.0_f64;
        let mut i01 = 0.0_f64;
        let mut i11 = 0.0_f64;
        for (&zi, &yi) in z.iter().zip(y.iter()) {
            let p = logistic_sigmoid(t0 + t1 * zi);
            let r = yi - p;
            let w = p * (1.0 - p);
            g0 += r;
            g1 += r * zi;
            i00 += w;
            i01 += w * zi;
            i11 += w * zi * zi;
        }
        let det = i00 * i11 - i01 * i01;
        if !det.is_finite() || det <= 1e-24 {
            break;
        }
        let step0 = (i11 * g0 - i01 * g1) / det;
        let step1 = (-i01 * g0 + i00 * g1) / det;
        let step_norm = step0.abs().max(step1.abs());
        let grad_norm = g0.abs().max(g1.abs());

        // Backtracking line search with log-likelihood-scaled tolerance guaranteeing ascent.
        let ll_tol = 1e-12 * (1.0 + ll_curr.abs());
        let mut alpha = 1.0_f64;
        let mut accepted = false;
        for _ in 0..30 {
            let cand0 = t0 + alpha * step0;
            let cand1 = t1 + alpha * step1;
            let ll_cand = log_likelihood(&z, y, cand0, cand1);
            if ll_cand.is_finite() && ll_cand >= ll_curr - ll_tol {
                t0 = cand0;
                t1 = cand1;
                ll_curr = ll_cand;
                accepted = true;
                break;
            }
            alpha *= 0.5;
        }
        if !accepted {
            break;
        }
        let step_tol = 1e-12 * (1.0 + t0.abs().max(t1.abs()));
        let grad_tol = 1e-10 * (1.0 + nf * 1e-3);
        if step_norm < step_tol && grad_norm < grad_tol {
            converged = true;
            break;
        }
    }

    // Recompute exact Fisher information in standardized coordinates and transform via Jacobian J.
    let mut i00 = 0.0_f64;
    let mut i01 = 0.0_f64;
    let mut i11 = 0.0_f64;
    for &zi in &z {
        let p = logistic_sigmoid(t0 + t1 * zi);
        let w = p * (1.0 - p);
        i00 += w;
        i01 += w * zi;
        i11 += w * zi * zi;
    }
    let det = i00 * i11 - i01 * i01;
    if !det.is_finite() || det <= 1e-24 {
        return StandardizedLogisticFit::failed();
    }
    let s00 = i11 / det;
    let s01 = -i01 / det;
    let s11 = i00 / det;

    // Back-transform: beta1 = t1 / sd_x, beta0 = t0 - mean_x * beta1.
    // Jacobian J = [[1, -mean_x / sd_x], [0, 1 / sd_x]].
    let beta1 = t1 / sd_x;
    let beta0 = t0 - mean_x * beta1;
    let a = -mean_x / sd_x;
    let b = 1.0 / sd_x;
    let cov00 = (s00 + 2.0 * a * s01 + a * a * s11).max(0.0);
    let cov01 = b * (s01 + a * s11);
    let cov11 = (b * b * s11).max(0.0);
    StandardizedLogisticFit { mean_x, sd_x, z, t0, t1, s00, s01, s11, beta0, beta1, cov00, cov01, cov11, converged }
}

/// Evaluate log-likelihood, score, and observed Fisher information for a 1D affine linear
/// predictor `eta_i(u) = base_i + u * coeff_i` (McCullagh & Nelder 1989).
fn eval_affine_1d(y: &[f64], eval_eta_coeff: impl Fn(usize) -> (f64, f64)) -> (f64, f64, f64) {
    let mut ll = 0.0_f64;
    let mut g = 0.0_f64;
    let mut h = 0.0_f64;
    for (i, &yi) in y.iter().enumerate() {
        let (eta, c) = eval_eta_coeff(i);
        if eta >= 0.0 {
            ll += (yi - 1.0) * eta - (-eta).exp().ln_1p();
        } else {
            ll += yi * eta - eta.exp().ln_1p();
        }
        let p = logistic_sigmoid(eta);
        g += (yi - p) * c;
        h += p * (1.0 - p) * c * c;
    }
    (ll, g, h)
}

/// Bracket the unique root of a strictly decreasing score `g(u) = d(ll)/du = 0` from `init`.
fn bracket_concave_1d(eval: &impl Fn(f64) -> (f64, f64, f64), init: f64, g_init: f64, h_init: f64, step_base: f64) -> (f64, f64, f64, f64, f64) {
    let dir = if g_init > 0.0 { 1.0 } else { -1.0 };
    let newton_mag = if h_init > 1e-18 { (g_init.abs() / h_init).clamp(step_base, 1e4 * step_base) } else { step_base };
    let mut step = if newton_mag.is_finite() { newton_mag } else { step_base };
    let mut prev_u = init;
    let mut prev_g = g_init;
    let mut best_ll = f64::NEG_INFINITY;
    let mut curr_u = init + dir * step;
    let (mut curr_ll, mut curr_g, _) = eval(curr_u);
    if curr_ll.is_finite() {
        best_ll = best_ll.max(curr_ll);
    }
    for _ in 0..80 {
        if !curr_g.is_finite() || curr_g * dir <= 0.0 {
            break;
        }
        prev_u = curr_u;
        prev_g = curr_g;
        step *= 2.0;
        if step > 1e12 * step_base {
            break;
        }
        curr_u = prev_u + dir * step;
        let (ll_next, g_next, _) = eval(curr_u);
        curr_ll = ll_next;
        curr_g = g_next;
        if curr_ll.is_finite() {
            best_ll = best_ll.max(curr_ll);
        }
    }
    if dir > 0.0 { (prev_u, curr_u, prev_g, curr_g, best_ll) } else { (curr_u, prev_u, curr_g, prev_g, best_ll) }
}

/// Maximize a strictly concave 1D log-likelihood given `eval(u) -> (ll, g, h)` via exponential
/// score bracketing and safeguarded Newton-bisection (Press et al. 2007, §9.4), preventing early
/// termination in saturated tails where `h <= 1e-24`.
fn maximize_concave_1d(eval: impl Fn(f64) -> (f64, f64, f64), init: f64, init_step: f64) -> f64 {
    let u0 = if init.is_finite() { init } else { 0.0 };
    let (mut best_ll, g0, h0) = eval(u0);
    if !best_ll.is_finite() || !g0.is_finite() || g0.abs() <= 1e-12 {
        return best_ll;
    }
    let step_base = if init_step.is_finite() && init_step > 0.0 { init_step } else { 1.0 };
    let (mut lo, mut hi, g_lo, g_hi, b_ll) = bracket_concave_1d(&eval, u0, g0, h0, step_base);
    if b_ll.is_finite() && b_ll > best_ll {
        best_ll = b_ll;
    }
    if !g_lo.is_finite() || !g_hi.is_finite() || g_lo < 0.0 || g_hi > 0.0 {
        return best_ll;
    }

    let mut u = 0.5 * (lo + hi);
    for _ in 0..100 {
        let (ll_u, g_u, h_u) = eval(u);
        if ll_u.is_finite() && ll_u > best_ll {
            best_ll = ll_u;
        }
        if !g_u.is_finite() {
            u = 0.5 * (lo + hi);
            continue;
        }
        if g_u > 0.0 {
            lo = u;
        } else if g_u < 0.0 {
            hi = u;
        } else {
            return best_ll;
        }
        if g_u.abs() <= 1e-12 || (hi - lo) <= 1e-12 * (1.0 + lo.abs() + hi.abs()) {
            break;
        }
        let mid = 0.5 * (lo + hi);
        let margin = 1e-6 * (hi - lo);
        u = if h_u.is_finite() && h_u > 1e-24 {
            let cand = u + g_u / h_u;
            if cand > lo + margin && cand < hi - margin { cand } else { mid }
        } else {
            mid
        };
    }
    let (ll_mid, _, _) = eval(0.5 * (lo + hi));
    if ll_mid.is_finite() && ll_mid > best_ll { ll_mid } else { best_ll }
}

/// Maximize $\ell(\theta_0, \theta_1)$ over standardized intercept $\theta_0$ for fixed
/// standardized slope $\theta_1 = \beta_1 s_x$.
fn profile_ll_t1(z: &[f64], y: &[f64], t1: f64, init_t0: f64) -> f64 {
    maximize_concave_1d(|t0| eval_affine_1d(y, |i| (t0 + t1 * z[i], 1.0)), init_t0, 1.0)
}

/// Maximize $\ell(\beta_0, \beta_1)$ over $\beta_1$ for fixed raw intercept $\beta_0$,
/// reparameterized through $v = \beta_0 + \beta_1 \bar{x}$ when $|\bar{x}| > s_x$ to avoid
/// catastrophic shift cancellation and Hessian saturation.
fn profile_ll_beta0_std(fit: &StandardizedLogisticFit, y: &[f64], beta0: f64, logit_p_bar: f64) -> f64 {
    if fit.mean_x.abs() > fit.sd_x {
        let ratio = fit.sd_x / fit.mean_x;
        maximize_concave_1d(
            |v| {
                eval_affine_1d(y, |i| {
                    let r_i = fit.z[i] * ratio;
                    let eta = v + (v * r_i - beta0 * r_i);
                    (eta, 1.0 + r_i)
                })
            },
            logit_p_bar,
            1.0,
        )
    } else {
        let c = fit.mean_x / fit.sd_x;
        maximize_concave_1d(
            |t1| {
                eval_affine_1d(y, |i| {
                    let coeff = c + fit.z[i];
                    (beta0 + t1 * coeff, coeff)
                })
            },
            fit.t1,
            1.0,
        )
    }
}

/// Maximize $\ell(-\theta_1 z_{50}, \theta_1)$ over standardized slope $\theta_1$ for fixed
/// standardized midpoint $z_{50} = (x_{50} - \bar{x}) / s_x$, warm-started at $\theta_1 = 0$.
fn profile_ll_z50(z: &[f64], y: &[f64], z50: f64) -> f64 {
    let max_d = z.iter().map(|&zi| (zi - z50).abs()).fold(0.0_f64, f64::max);
    let init_step = 1.0 / max_d.max(1.0);
    maximize_concave_1d(
        |t1| {
            eval_affine_1d(y, |i| {
                let d = z[i] - z50;
                (t1 * d, d)
            })
        },
        0.0,
        init_step,
    )
}

/// Bracket and solve $f(t) = \text{target\_ll}$ on one side (`dir = -1.0` or `+1.0`) of `t_mle`.
/// Returns `None` if the profile likelihood never drops below `target_ll` (unbounded CI side).
fn solve_profile_bound(f: impl Fn(f64) -> f64, t_mle: f64, dir: f64, init_step: f64, target_ll: f64) -> Option<f64> {
    if !t_mle.is_finite() || !init_step.is_finite() || init_step <= 0.0 {
        return None;
    }
    let mut lo = t_mle;
    let mut f_lo = f(lo) - target_ll;
    if !f_lo.is_finite() || f_lo <= 0.0 {
        return Some(t_mle);
    }

    let mut step = init_step;
    let mut hi = t_mle + dir * step;
    let mut f_hi = f(hi) - target_ll;
    let mut bracketed = f_hi.is_finite() && f_hi <= 0.0;

    for _ in 0..40 {
        if bracketed {
            break;
        }
        lo = hi;
        f_lo = f_hi;
        step *= 2.0;
        if step > 1e12 * init_step.max(1.0) {
            break;
        }
        hi = t_mle + dir * step;
        f_hi = f(hi) - target_ll;
        if f_hi.is_finite() && f_hi <= 0.0 {
            bracketed = true;
        }
    }
    if !bracketed {
        return None;
    }

    // Safeguarded bisection-secant root-finder in [lo, hi] where f_lo >= 0 >= f_hi.
    let mut a = lo;
    let mut b = hi;
    let mut fa = f_lo;
    let mut fb = f_hi;
    for _ in 0..80 {
        if (b - a).abs() <= 1e-11 * (1.0 + a.abs() + b.abs()) {
            break;
        }
        let mid = 0.5 * (a + b);
        let cand = if (fa - fb).abs() > 1e-15 {
            let sec = a + fa * (b - a) / (fa - fb);
            let lo_end = a.min(b);
            let hi_end = a.max(b);
            let margin = 0.05 * (hi_end - lo_end);
            if sec > lo_end + margin && sec < hi_end - margin { sec } else { mid }
        } else {
            mid
        };
        let fc = f(cand) - target_ll;
        if !fc.is_finite() {
            let fm = f(mid) - target_ll;
            if fm >= 0.0 {
                a = mid;
                fa = fm;
            } else {
                b = mid;
                fb = fm;
            }
            continue;
        }
        if fc.abs() <= 1e-12 {
            return Some(cand);
        }
        if fc >= 0.0 {
            a = cand;
            fa = fc;
        } else {
            b = cand;
            fb = fc;
        }
    }
    Some(0.5 * (a + b))
}

fn compute_profile_ci_beta1_std(fit: &StandardizedLogisticFit, y: &[f64], logit_p_bar: f64, z_crit: f64, target_ll: f64) -> ProfileCi {
    let se_t1 = fit.s11.max(0.0).sqrt();
    let step = (z_crit * se_t1).max(1e-4);
    let lo_t1 = solve_profile_bound(|t1| profile_ll_t1(&fit.z, y, t1, logit_p_bar), fit.t1, -1.0, step, target_ll);
    let hi_t1 = solve_profile_bound(|t1| profile_ll_t1(&fit.z, y, t1, logit_p_bar), fit.t1, 1.0, step, target_ll);
    ProfileCi { lower: lo_t1.map(|v| v / fit.sd_x).filter(|v| v.is_finite()), upper: hi_t1.map(|v| v / fit.sd_x).filter(|v| v.is_finite()) }
}

fn compute_profile_ci_beta0_std(fit: &StandardizedLogisticFit, y: &[f64], se0: f64, logit_p_bar: f64, z_crit: f64, target_ll: f64) -> ProfileCi {
    let step = (z_crit * se0).max(1e-4);
    let lower = solve_profile_bound(|b0| profile_ll_beta0_std(fit, y, b0, logit_p_bar), fit.beta0, -1.0, step, target_ll);
    let upper = solve_profile_bound(|b0| profile_ll_beta0_std(fit, y, b0, logit_p_bar), fit.beta0, 1.0, step, target_ll);
    ProfileCi { lower, upper }
}

fn compute_profile_ci_x50_std(fit: &StandardizedLogisticFit, y: &[f64], z50: f64, z_crit: f64, target_ll: f64, ll_null: f64) -> ProfileCi {
    // Profile likelihood for X50 approaches ll_null as |X50| -> Inf.
    // Bounded CI exists iff ll_null < target_ll (equivalently G^2 > chi^2_{1, 1-alpha}).
    if ll_null >= target_ll - 1e-12 {
        return ProfileCi::OPEN;
    }
    let var_z50 = (fit.s00 + 2.0 * z50 * fit.s01 + z50 * z50 * fit.s11) / (fit.t1 * fit.t1);
    let se_z50 = if var_z50.is_finite() && var_z50 > 0.0 { var_z50.sqrt() } else { 1.0 };
    let step = (z_crit * se_z50).max(1e-3);
    let lo_z = solve_profile_bound(|z0| profile_ll_z50(&fit.z, y, z0), z50, -1.0, step, target_ll);
    let hi_z = solve_profile_bound(|z0| profile_ll_z50(&fit.z, y, z0), z50, 1.0, step, target_ll);
    ProfileCi {
        lower: lo_z.map(|z0| fit.mean_x + fit.sd_x * z0).filter(|v| v.is_finite()),
        upper: hi_z.map(|z0| fit.mean_x + fit.sd_x * z0).filter(|v| v.is_finite()),
    }
}

fn exp_profile_ci(ci: ProfileCi) -> ProfileCi {
    ProfileCi { lower: ci.lower.map(f64::exp).filter(|v| v.is_finite()), upper: ci.upper.map(f64::exp).filter(|v| v.is_finite()) }
}

fn predict_prob_with_ci_std(x_val: f64, fit: &StandardizedLogisticFit, z_crit: f64) -> (f64, (f64, f64)) {
    let z = (x_val - fit.mean_x) / fit.sd_x;
    let eta = fit.t0 + fit.t1 * z;
    let p = logistic_sigmoid(eta);
    let var_eta = (fit.s00 + 2.0 * z * fit.s01 + z * z * fit.s11).max(0.0);
    let se_eta = var_eta.sqrt();
    let lo = logistic_sigmoid(eta - z_crit * se_eta);
    let hi = logistic_sigmoid(eta + z_crit * se_eta);
    (p, (lo, hi))
}

fn build_separated_result(n: usize, n_events: usize, separation: SimpleLogisticSeparation, ll_null: f64, cutoff: f64) -> SimpleLogisticResult {
    SimpleLogisticResult {
        n,
        n_events,
        n_non_events: n - n_events,
        converged: false,
        separation,
        beta0: f64::NAN,
        beta1: f64::NAN,
        se_beta0: f64::NAN,
        se_beta1: f64::NAN,
        cov_01: f64::NAN,
        wald_z_beta0: f64::NAN,
        wald_p_beta0: f64::NAN,
        wald_z_beta1: f64::NAN,
        wald_p_beta1: f64::NAN,
        profile_ci_beta0: ProfileCi::OPEN,
        profile_ci_beta1: ProfileCi::OPEN,
        odds_ratio_beta0: f64::NAN,
        odds_ratio_beta1: f64::NAN,
        odds_ratio_ci_beta0: ProfileCi::OPEN,
        odds_ratio_ci_beta1: ProfileCi::OPEN,
        x50: None,
        x50_profile_ci: ProfileCi::OPEN,
        log_lik_null: ll_null,
        log_lik_model: f64::NAN,
        deviance_null: (-2.0 * ll_null).max(0.0),
        deviance_model: f64::NAN,
        lr_chi2: f64::NAN,
        lr_df: 1.0,
        lr_p: f64::NAN,
        aic: f64::NAN,
        aicc: f64::NAN,
        bic: f64::NAN,
        tjur_r2: f64::NAN,
        cox_snell_r2: f64::NAN,
        nagelkerke_r2: f64::NAN,
        mcfadden_r2: f64::NAN,
        roc: None,
        classification_cutoff: cutoff,
        tp: 0,
        tn: 0,
        fp: 0,
        fn_count: 0,
        accuracy: f64::NAN,
        sensitivity: f64::NAN,
        specificity: f64::NAN,
        row_predictions: Vec::new(),
    }
}

/// Fit standalone simple logistic regression on paired `(x, y)` observations with `y in {0, 1}`
/// and optional out-of-sample `unknown_x` (`(row_index, x_new)` pairs).
///
/// Rejects non-binary `Y`, non-finite inputs, invalid `cutoff` outside `(0, 1)`, or degenerate
/// all-0 / all-1 outcomes with a structured [`StatsError`]. Explicitly detects and reports
/// collinearity, complete separation, and quasi-complete separation without diverging.
pub fn simple_logistic_regression(
    x: &[f64],
    y: &[f64],
    unknown_x: &[(usize, f64)],
    cutoff: f64,
    confidence: ConfidenceLevel,
) -> Result<SimpleLogisticResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::DimensionMismatch { expected: x.len(), actual: y.len() });
    }
    let observed_rows: Vec<(usize, f64, f64)> = x.iter().zip(y.iter()).enumerate().map(|(i, (&xi, &yi))| (i, xi, yi)).collect();
    let (res, _) = simple_logistic_regression_rows(&observed_rows, unknown_x, cutoff, confidence)?;
    Ok(res)
}

/// Internal row-index-preserving entry point used by both [`simple_logistic_regression`] and
/// [`run_simple_logistic_analysis`].
fn simple_logistic_regression_rows(
    observed_rows: &[(usize, f64, f64)],
    unknown_x: &[(usize, f64)],
    cutoff: f64,
    confidence: ConfidenceLevel,
) -> Result<(SimpleLogisticResult, Option<StandardizedLogisticFit>), StatsError> {
    if !cutoff.is_finite() || cutoff <= 0.0 || cutoff >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "classification_cutoff", value: cutoff });
    }
    let n = observed_rows.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "simple logistic regression", required: 3, actual: n });
    }
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut n_events = 0_usize;
    for &(_, xi, yi) in observed_rows {
        if !xi.is_finite() {
            return Err(StatsError::InvalidParameter { name: "X", value: xi });
        }
        if !yi.is_finite() || (yi != 0.0 && yi != 1.0) {
            return Err(StatsError::InvalidParameter { name: "Y (must be binary 0 or 1)", value: yi });
        }
        if yi == 1.0 {
            n_events += 1;
        }
        x.push(xi);
        y.push(yi);
    }
    if n_events == 0 || n_events == n {
        return Err(StatsError::InvalidParameter { name: "Y (requires both 0 and 1 outcomes)", value: n_events as f64 });
    }

    let nf = n as f64;
    let p_bar = (n_events as f64) / nf;
    let logit_p_bar = (p_bar / (1.0 - p_bar)).ln();
    let ll_null = (n_events as f64) * p_bar.ln() + ((n - n_events) as f64) * (1.0 - p_bar).ln();

    let separation = detect_separation(&x, &y);
    if separation != SimpleLogisticSeparation::None {
        return Ok((build_separated_result(n, n_events, separation, ll_null, cutoff), None));
    }

    let fit = fit_irls_standardized(&x, &y, n_events);
    if !fit.converged || !fit.beta0.is_finite() || !fit.beta1.is_finite() {
        return Err(StatsError::NotConverged { algorithm: "simple logistic IRLS" });
    }
    let beta0 = fit.beta0;
    let beta1 = fit.beta1;
    let cov00 = fit.cov00;
    let cov01 = fit.cov01;
    let cov11 = fit.cov11;

    let se_beta0 = cov00.sqrt();
    let se_beta1 = cov11.sqrt();
    let wald_z_beta0 = if se_beta0 > 0.0 { beta0 / se_beta0 } else { 0.0 };
    let wald_p_beta0 = norm_two_sided_p(wald_z_beta0);
    let wald_z_beta1 = if se_beta1 > 0.0 { beta1 / se_beta1 } else { 0.0 };
    let wald_p_beta1 = norm_two_sided_p(wald_z_beta1);

    let ll_model = log_likelihood(&fit.z, &y, fit.t0, fit.t1);
    let deviance_null = (-2.0 * ll_null).max(0.0);
    let deviance_model = (-2.0 * ll_model).max(0.0);
    let lr_chi2 = (deviance_null - deviance_model).max(0.0);
    let lr_df = 1.0_f64;
    let lr_p = chi2_sf(lr_chi2, lr_df)?;

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let target_ll = ll_model - 0.5 * z_crit * z_crit;

    let profile_ci_beta0 = compute_profile_ci_beta0_std(&fit, &y, se_beta0, logit_p_bar, z_crit, target_ll);
    let profile_ci_beta1 = compute_profile_ci_beta1_std(&fit, &y, logit_p_bar, z_crit, target_ll);
    let odds_ratio_beta0 = beta0.exp();
    let odds_ratio_beta1 = beta1.exp();
    let odds_ratio_ci_beta0 = exp_profile_ci(profile_ci_beta0);
    let odds_ratio_ci_beta1 = exp_profile_ci(profile_ci_beta1);

    let (x50, x50_profile_ci) = if fit.t1.abs() > 1e-12 {
        let z50 = -fit.t0 / fit.t1;
        let est = fit.mean_x + fit.sd_x * z50;
        if est.is_finite() {
            let ci = compute_profile_ci_x50_std(&fit, &y, z50, z_crit, target_ll, ll_null);
            (Some(est), ci)
        } else {
            (None, ProfileCi::OPEN)
        }
    } else {
        (None, ProfileCi::OPEN)
    };

    let k = 2.0_f64;
    let aic = deviance_model + 2.0 * k;
    let aicc = if nf - k - 1.0 > 0.0 { aic + (2.0 * k * (k + 1.0)) / (nf - k - 1.0) } else { f64::INFINITY };
    let bic = deviance_model + k * nf.ln();

    let mcfadden_r2 = if deviance_null > 0.0 { (1.0 - deviance_model / deviance_null).clamp(0.0, 1.0) } else { 0.0 };
    let cox_snell_r2 = (1.0 - (-lr_chi2 / nf).exp()).clamp(0.0, 1.0);
    let max_cs = 1.0 - (2.0 * ll_null / nf).exp();
    let nagelkerke_r2 = if max_cs > 0.0 { (cox_snell_r2 / max_cs).clamp(0.0, 1.0) } else { 0.0 };

    let mut sum_p1 = 0.0_f64;
    let mut sum_p0 = 0.0_f64;
    let mut ctrl_probs = Vec::with_capacity(n - n_events);
    let mut case_probs = Vec::with_capacity(n_events);
    let mut tp = 0_usize;
    let mut tn = 0_usize;
    let mut fp = 0_usize;
    let mut fn_count = 0_usize;
    let mut row_predictions = Vec::with_capacity(n + unknown_x.len());

    for &(row_index, xi, yi) in observed_rows {
        let (p, prob_ci) = predict_prob_with_ci_std(xi, &fit, z_crit);
        let pred_class = u8::from(p >= cutoff);
        if yi == 1.0 {
            sum_p1 += p;
            case_probs.push(p);
            if pred_class == 1 {
                tp += 1;
            } else {
                fn_count += 1;
            }
        } else {
            sum_p0 += p;
            ctrl_probs.push(p);
            if pred_class == 0 {
                tn += 1;
            } else {
                fp += 1;
            }
        }
        row_predictions.push(SimpleLogisticRowPrediction {
            row_index,
            x: xi,
            observed_y: Some(yi),
            predicted_prob: p,
            prob_ci,
            predicted_class: pred_class,
            is_unknown_row: false,
        });
    }
    for &(row_index, x_new) in unknown_x {
        if !x_new.is_finite() {
            continue;
        }
        let (p, prob_ci) = predict_prob_with_ci_std(x_new, &fit, z_crit);
        row_predictions.push(SimpleLogisticRowPrediction {
            row_index,
            x: x_new,
            observed_y: None,
            predicted_prob: p,
            prob_ci,
            predicted_class: u8::from(p >= cutoff),
            is_unknown_row: true,
        });
    }
    row_predictions.sort_by_key(|r| r.row_index);

    let tjur_r2 = (sum_p1 / (n_events as f64) - sum_p0 / ((n - n_events) as f64)).clamp(0.0, 1.0);
    let roc = roc_curve_analysis(&ctrl_probs, &case_probs, RocCiMethod::DeLong, confidence).ok();
    let accuracy = ((tp + tn) as f64) / nf;
    let sensitivity = (tp as f64) / (n_events as f64);
    let specificity = (tn as f64) / ((n - n_events) as f64);

    let result = SimpleLogisticResult {
        n,
        n_events,
        n_non_events: n - n_events,
        converged: fit.converged,
        separation,
        beta0,
        beta1,
        se_beta0,
        se_beta1,
        cov_01: cov01,
        wald_z_beta0,
        wald_p_beta0,
        wald_z_beta1,
        wald_p_beta1,
        profile_ci_beta0,
        profile_ci_beta1,
        odds_ratio_beta0,
        odds_ratio_beta1,
        odds_ratio_ci_beta0,
        odds_ratio_ci_beta1,
        x50,
        x50_profile_ci,
        log_lik_null: ll_null,
        log_lik_model: ll_model,
        deviance_null,
        deviance_model,
        lr_chi2,
        lr_df,
        lr_p,
        aic,
        aicc,
        bic,
        tjur_r2,
        cox_snell_r2,
        nagelkerke_r2,
        mcfadden_r2,
        roc,
        classification_cutoff: cutoff,
        tp,
        tn,
        fp,
        fn_count,
        accuracy,
        sensitivity,
        specificity,
        row_predictions,
    };
    Ok((result, Some(fit)))
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

struct ExtractedXyBinaryDataset {
    data_set_index: usize,
    title: String,
    observed: Vec<(usize, f64, f64)>,
    unknown_x: Vec<(usize, f64)>,
    ignored_text_cells: usize,
}

fn extract_xy_binary_datasets(sheet: &DataSheet, spec: &AnalysisSpec) -> Result<Vec<ExtractedXyBinaryDataset>, StatsError> {
    let TableArchetype::Xy(xy) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "Simple logistic regression", actual: sheet.archetype.kind() });
    };
    if xy.y_subcolumn_mode.is_pre_summarized() || xy.y_subcolumn_mode.subcolumn_count() != 1 {
        return Err(StatsError::InvalidParameter {
            name: "y_subcolumn_mode (simple logistic regression requires single-subcolumn Y)",
            value: xy.y_subcolumn_mode.subcolumn_count() as f64,
        });
    }
    let selected = try_resolve_selected_indices(&spec.selected_data_sets, xy.data_sets.len())?;
    if selected.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let n_rows = xy.row_count();
    let mut datasets = Vec::with_capacity(selected.len());
    for ds_idx in selected {
        let col = &xy.data_sets[ds_idx];
        let mut observed = Vec::new();
        let mut unknown_x = Vec::new();
        let mut ignored_text_cells = 0_usize;

        for row in 0..n_rows {
            let x_val = xy.x_summary_at(row)?;
            let y_cell = col.cells.get(row).and_then(|r| r.first());
            if let Some(cell) = y_cell
                && !cell.excluded
            {
                match &cell.value {
                    CellValue::Text(_) => {
                        ignored_text_cells += 1;
                        continue;
                    }
                    CellValue::Number(yv) if !yv.is_finite() => {
                        return Err(StatsError::InvalidParameter { name: "Y (must be binary 0 or 1)", value: *yv });
                    }
                    _ => {}
                }
            }
            let Some(xv) = x_val.filter(|v| v.is_finite()) else {
                continue;
            };
            match y_cell {
                None => {
                    unknown_x.push((row, xv));
                }
                Some(cell) if cell.excluded => {}
                Some(cell) => match &cell.value {
                    CellValue::Empty => {
                        unknown_x.push((row, xv));
                    }
                    CellValue::Number(yv) => {
                        if !yv.is_finite() || (*yv != 0.0 && *yv != 1.0) {
                            return Err(StatsError::InvalidParameter { name: "Y (must be binary 0 or 1)", value: *yv });
                        }
                        observed.push((row, xv, *yv));
                    }
                    CellValue::Text(_) => {
                        ignored_text_cells += 1;
                    }
                },
            }
        }
        datasets.push(ExtractedXyBinaryDataset { data_set_index: ds_idx, title: col.title.clone(), observed, unknown_x, ignored_text_cells });
    }
    Ok(datasets)
}

fn build_sigmoid_overlay(
    ds: &ExtractedXyBinaryDataset,
    res: &SimpleLogisticResult,
    fit: Option<&StandardizedLogisticFit>,
    sl_spec: &SimpleLogisticSpec,
    confidence: ConfidenceLevel,
) -> Result<Option<FittedCurveOverlay>, StatsError> {
    let Some(std_fit) = fit.filter(|_| res.converged) else {
        return Ok(Some(FittedCurveOverlay {
            data_set_index: ds.data_set_index,
            equation_id: String::from("simple_logistic"),
            points: Vec::new(),
            converged: false,
            ambiguous: true,
        }));
    };
    let segments = sl_spec.plot_segments.unwrap_or(SimpleLogisticSpec::DEFAULT_PLOT_SEGMENTS);
    if !(1..=SimpleLogisticSpec::MAX_PLOT_SEGMENTS).contains(&segments) {
        return Err(StatsError::InvalidParameter { name: "plot_segments", value: segments as f64 });
    }
    let mut data_min = f64::INFINITY;
    let mut data_max = f64::NEG_INFINITY;
    for &(_, x, _) in &ds.observed {
        data_min = data_min.min(x);
        data_max = data_max.max(x);
    }
    let x_min = sl_spec.plot_x_min.unwrap_or(data_min);
    let x_max = sl_spec.plot_x_max.unwrap_or(data_max);
    if !x_min.is_finite() || !x_max.is_finite() || x_max <= x_min {
        return Err(StatsError::InvalidParameter { name: "plot_x_range", value: x_max - x_min });
    }
    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let n_pts = segments + 1;
    let mut points = Vec::with_capacity(n_pts);
    for i in 0..n_pts {
        let frac = (i as f64) / (segments as f64);
        let x = (1.0 - frac) * x_min + frac * x_max;
        let (p, (lo, hi)) = predict_prob_with_ci_std(x, std_fit, z_crit);
        let (ci_lower, ci_upper) = if sl_spec.confidence_bands { (Some(lo), Some(hi)) } else { (None, None) };
        points.push(CurveSamplePoint { x, y: p, ci_lower, ci_upper, pred_lower: None, pred_upper: None });
    }
    Ok(Some(FittedCurveOverlay {
        data_set_index: ds.data_set_index,
        equation_id: String::from("simple_logistic"),
        points,
        converged: true,
        ambiguous: false,
    }))
}

fn append_parameter_rows(tab: &mut TabBuilder, ds_label: Option<&str>, res: &SimpleLogisticResult, p_fmt: curvebook_doc::PValueFormat, alpha: f64) {
    let prefix = ds_label.map(|l| format!("{l}: ")).unwrap_or_default();
    tab.push_row(
        format!("{prefix}beta_0 (Intercept)"),
        vec![
            opt_num_cell(res.beta0.is_finite().then_some(res.beta0)),
            opt_num_cell(res.se_beta0.is_finite().then_some(res.se_beta0)),
            res.profile_ci_beta0.to_result_cell(),
            opt_num_cell(res.wald_z_beta0.is_finite().then_some(res.wald_z_beta0)),
            if res.wald_p_beta0.is_finite() { p_cell_at_alpha(res.wald_p_beta0, p_fmt, alpha) } else { ResultCell::Empty },
        ],
    );
    tab.push_row(
        format!("{prefix}beta_1 (Slope)"),
        vec![
            opt_num_cell(res.beta1.is_finite().then_some(res.beta1)),
            opt_num_cell(res.se_beta1.is_finite().then_some(res.se_beta1)),
            res.profile_ci_beta1.to_result_cell(),
            opt_num_cell(res.wald_z_beta1.is_finite().then_some(res.wald_z_beta1)),
            if res.wald_p_beta1.is_finite() { p_cell_at_alpha(res.wald_p_beta1, p_fmt, alpha) } else { ResultCell::Empty },
        ],
    );
    tab.push_row(
        format!("{prefix}Odds Ratio (beta_0)"),
        vec![
            opt_num_cell(res.odds_ratio_beta0.is_finite().then_some(res.odds_ratio_beta0)),
            ResultCell::Empty,
            res.odds_ratio_ci_beta0.to_result_cell(),
            ResultCell::Empty,
            ResultCell::Empty,
        ],
    );
    tab.push_row(
        format!("{prefix}Odds Ratio (beta_1)"),
        vec![
            opt_num_cell(res.odds_ratio_beta1.is_finite().then_some(res.odds_ratio_beta1)),
            ResultCell::Empty,
            res.odds_ratio_ci_beta1.to_result_cell(),
            ResultCell::Empty,
            ResultCell::Empty,
        ],
    );
    tab.push_row(
        format!("{prefix}X at 50%"),
        vec![opt_num_cell(res.x50), ResultCell::Empty, res.x50_profile_ci.to_result_cell(), ResultCell::Empty, ResultCell::Empty],
    );
}

fn append_gof_rows(kv: &mut KvTabBuilder, ds_label: Option<&str>, res: &SimpleLogisticResult, p_fmt: curvebook_doc::PValueFormat, alpha: f64) {
    let pfx = |name: &str| ds_label.map_or_else(|| name.to_owned(), |l| format!("{l}: {name}"));
    kv.push_cell(pfx("N"), num_cell(res.n as f64));
    kv.push_cell(pfx("Events (Y = 1)"), num_cell(res.n_events as f64));
    kv.push_cell(pfx("Non-events (Y = 0)"), num_cell(res.n_non_events as f64));
    kv.push_cell(pfx("Separation status"), text_cell(res.separation.label()));
    kv.push_cell(pfx("Null deviance (D0)"), num_cell(res.deviance_null));
    kv.push_cell(pfx("Model deviance (D1)"), opt_num_cell(res.deviance_model.is_finite().then_some(res.deviance_model)));
    kv.push_cell(pfx("Likelihood ratio G^2"), opt_num_cell(res.lr_chi2.is_finite().then_some(res.lr_chi2)));
    kv.push_cell(pfx("LR df"), num_cell(res.lr_df));
    kv.push_cell(pfx("Likelihood ratio P"), if res.lr_p.is_finite() { p_cell_at_alpha(res.lr_p, p_fmt, alpha) } else { ResultCell::Empty });
    kv.push_cell(pfx("AIC"), opt_num_cell(res.aic.is_finite().then_some(res.aic)));
    kv.push_cell(pfx("AICc"), opt_num_cell(res.aicc.is_finite().then_some(res.aicc)));
    kv.push_cell(pfx("BIC"), opt_num_cell(res.bic.is_finite().then_some(res.bic)));
    kv.push_cell(pfx("Tjur's R squared"), opt_num_cell(res.tjur_r2.is_finite().then_some(res.tjur_r2)));
    kv.push_cell(pfx("Cox-Snell R squared"), opt_num_cell(res.cox_snell_r2.is_finite().then_some(res.cox_snell_r2)));
    kv.push_cell(pfx("Nagelkerke R squared"), opt_num_cell(res.nagelkerke_r2.is_finite().then_some(res.nagelkerke_r2)));
    kv.push_cell(pfx("McFadden R squared"), opt_num_cell(res.mcfadden_r2.is_finite().then_some(res.mcfadden_r2)));
    if let Some(roc) = &res.roc {
        kv.push_cell(pfx("ROC AUC"), num_cell(roc.auc));
        kv.push_cell(pfx("ROC AUC SE (DeLong)"), num_cell(roc.se));
        kv.push_cell(pfx("ROC AUC CI"), ci_cell(roc.ci.0, roc.ci.1));
        kv.push_cell(pfx("ROC P (vs 0.5)"), p_cell_at_alpha(roc.p_value_vs_05, p_fmt, alpha));
    }
    kv.push_cell(pfx("Classification cutoff"), num_cell(res.classification_cutoff));
    kv.push_cell(pfx("Accuracy"), opt_num_cell(res.accuracy.is_finite().then_some(res.accuracy)));
    kv.push_cell(pfx("Sensitivity"), opt_num_cell(res.sensitivity.is_finite().then_some(res.sensitivity)));
    kv.push_cell(pfx("Specificity"), opt_num_cell(res.specificity.is_finite().then_some(res.specificity)));
}

fn push_dataset_scalars(out: &mut ResultSheet, data_set_index: usize, res: &SimpleLogisticResult) {
    let ds = Some(data_set_index);
    push_scalar(out, "Beta0", ds, res.beta0);
    push_scalar(out, "Beta1", ds, res.beta1);
    push_scalar(out, "SE_Beta0", ds, res.se_beta0);
    push_scalar(out, "SE_Beta1", ds, res.se_beta1);
    push_scalar(out, "OR_Beta0", ds, res.odds_ratio_beta0);
    push_scalar(out, "OR_Beta1", ds, res.odds_ratio_beta1);
    if let Some(x50) = res.x50 {
        push_scalar(out, "X50", ds, x50);
    }
    push_scalar(out, "Wald_Z", ds, res.wald_z_beta1);
    push_scalar(out, "Wald_P", ds, res.wald_p_beta1);
    push_scalar(out, "LR_G2", ds, res.lr_chi2);
    push_scalar(out, "LR_P", ds, res.lr_p);
    push_scalar(out, "Deviance_Null", ds, res.deviance_null);
    push_scalar(out, "Deviance_Model", ds, res.deviance_model);
    push_scalar(out, "AIC", ds, res.aic);
    push_scalar(out, "AICc", ds, res.aicc);
    push_scalar(out, "Tjur_R2", ds, res.tjur_r2);
    push_scalar(out, "CoxSnell_R2", ds, res.cox_snell_r2);
    push_scalar(out, "Nagelkerke_R2", ds, res.nagelkerke_r2);
    if let Some(roc) = &res.roc {
        push_scalar(out, "ROC_AUC", ds, roc.auc);
        push_scalar(out, "ROC_AUC_SE", ds, roc.se);
    }
    push_scalar(out, "Accuracy", ds, res.accuracy);
}

/// Execute [`curvebook_doc::AnalysisKind::SimpleLogisticRegression`] against an `XY` [`DataSheet`]
/// and assemble a complete [`ResultSheet`].
pub fn run_simple_logistic_analysis(sheet: &DataSheet, spec: &AnalysisSpec, sl_spec: &SimpleLogisticSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::SimpleLogisticRegression(*sl_spec))?;
    let datasets = extract_xy_binary_datasets(sheet, spec)?;
    let mut out = init_result_sheet(spec)?;
    let alpha = spec.confidence_level.alpha();
    let multi = datasets.len() > 1;

    let mut params_tab = TabBuilder::new("parameter-estimates", "Parameter Estimates", ResultTabKind::ParametersAndConfidenceIntervals).headers([
        "Estimate",
        "Asymptotic SE",
        "Profile CI",
        "Wald Z",
        "Wald P",
    ]);
    let mut gof_kv = KvTabBuilder::new("goodness-of-fit", "Goodness of Fit", ResultTabKind::MainSummary, "Value");
    let mut roc_tab = TabBuilder::new("roc-curve", "ROC Curve", ResultTabKind::CurveCoordinates).headers([
        "Probability Cutoff",
        "Sensitivity",
        "Specificity",
        "1 - Specificity",
        "Youden's J",
    ]);
    let mut class_tab = TabBuilder::new("row-classification", "Row Classification", ResultTabKind::InterpolatedUnknowns).headers([
        "Row",
        "X",
        "Observed Y",
        "Predicted P(Y=1)",
        "CI Lower",
        "CI Upper",
        "Predicted Class",
        "Row Type",
    ]);

    let mut ignored_text_details: Vec<(String, usize)> = Vec::new();
    let mut has_roc_rows = false;
    let mut has_class_rows = false;

    for ds in &datasets {
        if ds.ignored_text_cells > 0 {
            ignored_text_details.push((ds.title.clone(), ds.ignored_text_cells));
        }
        let (res, std_fit) = simple_logistic_regression_rows(&ds.observed, &ds.unknown_x, sl_spec.classification_cutoff, spec.confidence_level)?;
        let ds_label = multi.then_some(ds.title.as_str());

        append_parameter_rows(&mut params_tab, ds_label, &res, spec.p_value_format, alpha);
        append_gof_rows(&mut gof_kv, ds_label, &res, spec.p_value_format, alpha);
        push_dataset_scalars(&mut out, ds.data_set_index, &res);

        if res.separation != SimpleLogisticSeparation::None {
            params_tab.push_footnote(format!("{}: {}", ds.title, res.separation.explanation()));
            out.checklist.push(checklist_item(
                format!("Simple logistic separation ({})", ds.title),
                ChecklistStatus::Violated,
                Some(res.separation.label().to_owned()),
                res.separation.explanation(),
            ));
        } else {
            out.checklist.push(checklist_item(
                format!("Simple logistic convergence ({})", ds.title),
                ChecklistStatus::Pass,
                Some(format!("G^2 = {:.3}, P = {:.4}", res.lr_chi2, res.lr_p)),
                res.separation.explanation(),
            ));
            if res.n_events.min(res.n_non_events) < 10 {
                out.checklist.push(checklist_item(
                    format!("Events per variable ({})", ds.title),
                    ChecklistStatus::Warn,
                    Some(format!("min(n1, n0) = {}", res.n_events.min(res.n_non_events))),
                    format!(
                        "Minority outcome count is {} (< 10); asymptotic Wald approximations may be less accurate than profile-likelihood CIs.",
                        res.n_events.min(res.n_non_events)
                    ),
                ));
            }
        }

        if let Some(roc) = &res.roc {
            has_roc_rows = true;
            for (pt_idx, pt) in roc.points.iter().enumerate() {
                let row_label = if multi { format!("{} #{}", ds.title, pt_idx + 1) } else { format!("Point {}", pt_idx + 1) };
                roc_tab.push_row(
                    row_label,
                    vec![
                        num_cell(pt.cutoff),
                        num_cell(pt.sensitivity),
                        num_cell(pt.specificity),
                        num_cell(pt.one_minus_specificity),
                        num_cell(pt.youden_j),
                    ],
                );
            }
        }

        if sl_spec.emit_row_classification && !res.row_predictions.is_empty() {
            has_class_rows = true;
            for row in &res.row_predictions {
                let row_label = if multi { format!("{} Row {}", ds.title, row.row_index + 1) } else { format!("Row {}", row.row_index + 1) };
                class_tab.push_row(
                    row_label,
                    vec![
                        num_cell((row.row_index + 1) as f64),
                        num_cell(row.x),
                        opt_num_cell(row.observed_y),
                        num_cell(row.predicted_prob),
                        num_cell(row.prob_ci.0),
                        num_cell(row.prob_ci.1),
                        num_cell(f64::from(row.predicted_class)),
                        text_cell(if row.is_unknown_row { "Predicted (blank Y)" } else { "Observed" }),
                    ],
                );
            }
        }

        if let Some(overlay) = build_sigmoid_overlay(ds, &res, std_fit.as_ref(), sl_spec, spec.confidence_level)? {
            out.fitted_curves.push(overlay);
        }
        if let Some(roc) = &res.roc {
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
                data_set_index: ds.data_set_index,
                equation_id: String::from("ROC"),
                converged: true,
                ambiguous: false,
                points: curve_pts,
            });
        }
    }

    out.tabs.push(params_tab.build());
    out.tabs.push(gof_kv.build());
    if has_roc_rows {
        out.tabs.push(roc_tab.build());
    }
    if sl_spec.emit_row_classification && has_class_rows {
        out.tabs.push(class_tab.build());
    }
    append_ignored_text_checklist_and_footnote(&mut out, "Simple logistic regression", &ignored_text_details);

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}
