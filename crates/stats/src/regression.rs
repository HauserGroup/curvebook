//! Generalized linear and multiple regression models: Multiple Linear Regression (OLS with
//! Type I/III partial *F*, VIF multicollinearity, Cook's distance, leverage, Durbin–Watson),
//! Multiple Logistic Regression (IRLS with separation safeguard, Hosmer–Lemeshow, pseudo-*R*^2,
//! ROC AUC), and Multiple Poisson Regression (log-link IRLS, deviance & Pearson dispersion).

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, GlmFamily, GlmSpec, PassingBablokSpec,
    ResultCell, ResultSheet, ResultTabKind, SheetId, TableArchetype, VariableRole,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{chi2_sf, f_sf, norm_two_sided_isf, norm_two_sided_p, student_t_two_sided_isf, student_t_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares};
use lumencraft_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::common::{build_tab, checklist_item, ci_cell, init_result_sheet, num_cell, opt_num_cell, p_cell, push_scalar, validate_finite_slice};
use crate::error::StatsError;
use crate::roc::{RocCiMethod, roc_curve_analysis};

// ============================================================================
// Typed Outputs
// ============================================================================

/// Parameter row for Multiple Linear Regression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearTermEstimate {
    pub name: String,
    pub estimate: f64,
    pub se: f64,
    pub t_statistic: f64,
    pub p_value: f64,
    pub ci: (f64, f64),
    pub vif: Option<f64>,
    pub type_iii_ss: Option<f64>,
    pub type_iii_f: Option<f64>,
    pub type_iii_p: Option<f64>,
}

/// Complete Multiple Linear Regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleLinearResult {
    pub n: usize,
    pub p: usize,
    pub rank: usize,
    pub has_intercept: bool,
    pub aliased_columns: Vec<bool>,
    pub terms: Vec<LinearTermEstimate>,
    pub r_squared: f64,
    pub adjusted_r_squared: f64,
    pub rss: f64,
    pub ss_regression: f64,
    pub ss_total: f64,
    pub residual_sd: f64,
    pub df_model: f64,
    pub df_residual: f64,
    pub f_statistic: f64,
    pub f_p_value: f64,
    pub aic: f64,
    pub aicc: f64,
    pub bic: f64,
    pub durbin_watson: f64,
    pub fitted: Vec<f64>,
    pub residuals: Vec<f64>,
    pub leverage: Vec<f64>,
    pub cooks_distance: Vec<f64>,
    /// Parameter covariance matrix `sigma^2 * (X'X)^-1` (`p x p`).
    #[serde(default)]
    pub cov_beta: Vec<Vec<f64>>,
}

/// Parameter row for Logistic or Poisson GLM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlmTermEstimate {
    pub name: String,
    pub beta: f64,
    pub se: f64,
    pub z_statistic: f64,
    pub p_value: f64,
    pub exp_beta: f64,
    pub exp_beta_ci: (f64, f64),
    pub vif: Option<f64>,
}

/// Complete Multiple Logistic Regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleLogisticResult {
    pub n: usize,
    pub n_events: usize,
    pub converged: bool,
    pub separation_detected: bool,
    pub terms: Vec<GlmTermEstimate>,
    pub log_lik_null: f64,
    pub log_lik_model: f64,
    pub deviance_null: f64,
    pub deviance_residual: f64,
    pub lr_chi2: f64,
    pub lr_df: f64,
    pub lr_p: f64,
    pub aic: f64,
    pub aicc: f64,
    pub bic: f64,
    pub mcfadden_r2: f64,
    pub nagelkerke_r2: f64,
    pub tjur_r2: f64,
    pub hosmer_lemeshow_chi2: f64,
    pub hosmer_lemeshow_df: f64,
    pub hosmer_lemeshow_p: f64,
    pub roc_auc: Option<f64>,
    pub predicted_probs: Vec<f64>,
}

/// Complete Multiple Poisson Regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiplePoissonResult {
    pub n: usize,
    pub converged: bool,
    pub terms: Vec<GlmTermEstimate>,
    pub log_lik_null: f64,
    pub log_lik_model: f64,
    pub deviance_null: f64,
    pub deviance_residual: f64,
    pub pearson_chi2: f64,
    pub df_residual: f64,
    pub dispersion_pearson: f64,
    pub lr_chi2: f64,
    pub lr_df: f64,
    pub lr_p: f64,
    pub aic: f64,
    pub aicc: f64,
    pub bic: f64,
    pub predicted_means: Vec<f64>,
}

// ============================================================================
// Multiple Linear Regression (OLS via QR/SVD)
// ============================================================================

/// Fit Ordinary Least Squares multiple linear regression on design matrix `x` (`n x p`,
/// where column `0` is `1.0` if `has_intercept == true`) and response vector `y`.
///
/// Uses `lumencraft_math::linalg::solve_linear_least_squares` (column-equilibrated pivoted QR with
/// Bjorck mixed-precision refinement and SVD fallback) for full precision on stiff/collinear
/// benchmarks (`Wampler1..5`, `Filip`, `Longley`, `NoInt1`, `NoInt2`).
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
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
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
    let adj_r2 = if has_intercept && n > rank {
        1.0 - (rss / df_resid_f) / (ss_total / ((n - 1) as f64)).max(1e-30)
    } else if !has_intercept && n > rank {
        1.0 - (rss / df_resid_f) / (ss_total / (n as f64)).max(1e-30)
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

    // Fitted values & residuals using compensated dot products.
    let mut fitted = Vec::with_capacity(n);
    let mut residuals = Vec::with_capacity(n);
    for (r, &yr) in y.iter().enumerate() {
        let mut dot_acc = NeumaierSum::new();
        for (c, &b) in sol.coefficients.iter().enumerate() {
            dot_acc.add(x.get(r, c).unwrap_or(0.0) * b);
        }
        let yhat = dot_acc.total();
        fitted.push(yhat);
        residuals.push(yr - yhat);
    }

    // Durbin-Watson autocorrelation statistic.
    let mut dw_num = NeumaierSum::new();
    for i in 1..n {
        let d = residuals[i] - residuals[i - 1];
        dw_num.add(d * d);
    }
    let durbin_watson = if rss > 0.0 { dw_num.total() / rss } else { 2.0 };

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

    let vifs = if compute_vif { compute_vifs_from_matrix(x, has_intercept) } else { vec![None; p] };
    let t_crit = student_t_two_sided_isf(confidence.alpha(), df_resid_f)?;

    let mut terms = Vec::with_capacity(p);
    for j in 0..p {
        let est = sol.coefficients[j];
        let se = (mse * cov_unscaled.get(j, j).unwrap_or(0.0).max(0.0)).sqrt();
        let t_stat = if se > 0.0 { est / se } else { 0.0 };
        let p_val = if se > 0.0 { student_t_two_sided_p(t_stat, df_resid_f)? } else { 1.0 };
        let ci = (est - t_crit * se, est + t_crit * se);

        let (ss_iii, f_iii, p_iii) = if p > 1 && !(has_intercept && j == 0) && !sol.aliased_columns.contains(&j) {
            let x_red = drop_matrix_column(x, j)?;
            if let Ok((red_sol, _)) = solve_linear_least_squares(&x_red, y) {
                let ss_drop = (red_sol.rss - rss).max(0.0);
                let f_drop = if mse > 0.0 { ss_drop / mse } else { 0.0 };
                let p_drop = f_sf(f_drop, 1.0, df_resid_f).unwrap_or(1.0);
                (Some(ss_drop), Some(f_drop), Some(p_drop))
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

    let mut aliased_flags = vec![false; p];
    for &idx in &sol.aliased_columns {
        if idx < p {
            aliased_flags[idx] = true;
        }
    }

    let cov_beta: Vec<Vec<f64>> = (0..p).map(|j| (0..p).map(|k| mse * cov_unscaled.get(j, k).unwrap_or(0.0)).collect()).collect();

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

fn drop_matrix_column(x: &DenseMatrix, drop_col: usize) -> Result<DenseMatrix, StatsError> {
    let nr = x.nrows();
    let nc = x.ncols();
    let mut data = Vec::with_capacity(nr * (nc - 1));
    for r in 0..nr {
        for c in 0..nc {
            if c != drop_col {
                data.push(x.get(r, c).unwrap_or(0.0));
            }
        }
    }
    Ok(DenseMatrix::from_row_major(nr, nc - 1, data)?)
}

fn compute_vifs_from_matrix(x: &DenseMatrix, has_intercept: bool) -> Vec<Option<f64>> {
    let n = x.nrows();
    let p = x.ncols();
    let mut vifs = vec![None; p];
    let start = usize::from(has_intercept);
    if p <= start + 1 {
        return vifs;
    }
    for (j, slot) in vifs.iter_mut().enumerate().skip(start) {
        let mut target = Vec::with_capacity(n);
        let mut pred_cols = Vec::with_capacity(p - 1);
        for r in 0..n {
            target.push(x.get(r, j).unwrap_or(0.0));
            for c in 0..p {
                if c != j {
                    pred_cols.push(x.get(r, c).unwrap_or(0.0));
                }
            }
        }
        let Ok(xm) = DenseMatrix::from_row_major(n, p - 1, pred_cols) else { continue };
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

fn logistic_irls_loop(x: &DenseMatrix, y_binary: &[f64], beta: &mut [f64]) -> Result<(bool, bool), StatsError> {
    let p = beta.len();
    let mut ridge = 0.0_f64;
    let mut separation_detected = false;
    let mut converged = false;
    let (mut ll_curr, mut probs, mut weights) = eval_logistic_ll(x, y_binary, beta, 0.0);

    for _iter in 0..50 {
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
            separation_detected = true;
            ridge = 1e-3;
            let (ls, _) = solve_linear_least_squares(&info_mat, &grad)?;
            ls.coefficients
        };

        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_step > 3.0 { 3.0 / max_step } else { 1.0 };
        let mut accepted = false;
        for _ in 0..16 {
            let cand: Vec<f64> = (0..p).map(|j| beta[j] + scale * step[j]).collect();
            let (ll_cand, p_cand, w_cand) = eval_logistic_ll(x, y_binary, &cand, ridge);
            if ll_cand.is_finite() && ll_cand >= ll_curr - 1e-10 {
                let delta = (ll_cand - ll_curr).abs();
                beta.copy_from_slice(&cand);
                ll_curr = ll_cand;
                probs = p_cand;
                weights = w_cand;
                accepted = true;
                if delta < 1e-9 && max_step * scale < 1e-7 {
                    converged = true;
                }
                break;
            }
            scale *= 0.5;
        }
        if beta.iter().skip(1).any(|b| b.abs() > 15.0) {
            separation_detected = true;
            if ridge == 0.0 {
                ridge = 1e-2;
            }
        }
        if converged || !accepted {
            break;
        }
    }
    Ok((converged, separation_detected))
}

/// Fit binary Multiple Logistic Regression (`y in {0, 1}`) via IRLS Newton–Raphson with
/// backtracking line search and mild ridge/Firth-like safeguard when complete or quasi-complete
/// separation occurs.
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
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y_binary.len() }));
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

    let nf = n as f64;
    let p_bar = (n_events as f64) / nf;
    let ll_null = (n_events as f64) * p_bar.ln() + ((n - n_events) as f64) * (1.0 - p_bar).ln();
    let dev_null = -2.0 * ll_null;

    let mut beta = vec![0.0_f64; p];
    beta[0] = (p_bar / (1.0 - p_bar)).ln();
    let (converged, separation_detected) = logistic_irls_loop(x, y_binary, &mut beta)?;

    // Recompute unpenalized log-likelihood & covariance matrix.
    let (ll_unpen, probs_final, weights_final) = eval_logistic_ll(x, y_binary, &beta, 0.0);
    let mut info_final = vec![0.0_f64; p * p];
    for (r, &wr) in weights_final.iter().enumerate() {
        for j in 0..p {
            let xrj = x.get(r, j).unwrap_or(0.0);
            for k in j..p {
                info_final[j * p + k] += wr * xrj * x.get(r, k).unwrap_or(0.0);
            }
        }
    }
    for j in 0..p {
        for k in 0..j {
            info_final[j * p + k] = info_final[k * p + j];
        }
    }
    let info_mat = DenseMatrix::from_row_major(p, p, info_final)?;
    let cov_beta = match cholesky(&info_mat).and_then(|c| c.inverse()) {
        Ok(inv) => inv,
        Err(_) => DenseMatrix::identity(p)?,
    };

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let vifs = if compute_vif { compute_vifs_from_matrix(x, true) } else { vec![None; p] };
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
    let g = g_groups.min(n / 2).max(2);
    let mut pairs: Vec<(f64, f64)> = probs.iter().copied().zip(y.iter().copied()).collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut chi2 = 0.0_f64;
    let mut actual_bins = 0_usize;
    for b in 0..g {
        let start = b * n / g;
        let end = (b + 1) * n / g;
        if end <= start {
            continue;
        }
        let slice = &pairs[start..end];
        let nb = slice.len() as f64;
        let obs1: f64 = slice.iter().map(|(_, yv)| *yv).sum();
        let exp1: f64 = slice.iter().map(|(pr, _)| *pr).sum();
        let pi_bar = (exp1 / nb).clamp(1e-12, 1.0 - 1e-12);
        let denom = nb * pi_bar * (1.0 - pi_bar);
        if denom > 0.0 {
            chi2 += (obs1 - exp1).powi(2) / denom;
            actual_bins += 1;
        }
    }
    let df = (actual_bins.saturating_sub(2)).max(1) as f64;
    let p_val = chi2_sf(chi2, df)?;
    Ok((chi2, df, p_val))
}

// ============================================================================
// Multiple Poisson Regression (Log-Link IRLS)
// ============================================================================

/// Fit count-data Multiple Poisson Regression (`y >= 0`, log link) via IRLS with backtracking
/// line search, Rate Ratios (`exp(beta)`), deviance, and Pearson dispersion check.
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
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: n, actual: y_counts.len() }));
    }
    if n <= p || p == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "GLM regression", required: p + 1, actual: n });
    }
    for &yc in y_counts {
        if !yc.is_finite() || yc < 0.0 {
            return Err(StatsError::InvalidParameter { name: "poisson_y", value: yc });
        }
    }

    let nf = n as f64;
    let y_mean = (y_counts.iter().sum::<f64>() / nf).max(1e-6);
    let mut beta = vec![0.0_f64; p];
    beta[0] = y_mean.ln();

    let ln_fact = |y: f64| -> f64 { lumencraft_math::special::ln_gamma(y + 1.0).unwrap_or(0.0) };
    let ll_null: f64 = y_counts.iter().map(|&y| y * y_mean.ln() - y_mean - ln_fact(y)).sum();

    let eval_poisson = |b: &[f64]| -> (f64, Vec<f64>) {
        let mut ll = 0.0_f64;
        let mut mu_vec = Vec::with_capacity(n);
        for (r, &yr) in y_counts.iter().enumerate() {
            let mut eta = 0.0_f64;
            for (c, &bc) in b.iter().enumerate() {
                eta += x.get(r, c).unwrap_or(0.0) * bc;
            }
            eta = eta.clamp(-25.0, 25.0);
            let mu = eta.exp();
            ll += yr * eta - mu - ln_fact(yr);
            mu_vec.push(mu);
        }
        (ll, mu_vec)
    };

    let (mut ll_curr, mut mus) = eval_poisson(&beta);
    let mut converged = false;

    for _iter in 0..50 {
        let mut grad = vec![0.0_f64; p];
        let mut info = vec![0.0_f64; p * p];
        for r in 0..n {
            let diff = y_counts[r] - mus[r];
            let wr = mus[r].max(1e-12);
            for j in 0..p {
                let xrj = x.get(r, j).unwrap_or(0.0);
                grad[j] += xrj * diff;
                for k in j..p {
                    info[j * p + k] += wr * xrj * x.get(r, k).unwrap_or(0.0);
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
        let max_step = step.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
        let mut scale = if max_step > 2.0 { 2.0 / max_step } else { 1.0 };
        let mut accepted = false;
        for _ in 0..16 {
            let cand: Vec<f64> = (0..p).map(|j| beta[j] + scale * step[j]).collect();
            let (ll_cand, mu_cand) = eval_poisson(&cand);
            if ll_cand.is_finite() && ll_cand >= ll_curr - 1e-10 {
                let delta = (ll_cand - ll_curr).abs();
                beta = cand;
                ll_curr = ll_cand;
                mus = mu_cand;
                accepted = true;
                if delta < 1e-9 && max_step * scale < 1e-7 {
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

    let mut info_final = vec![0.0_f64; p * p];
    let mut dev_null = 0.0_f64;
    let mut dev_resid = 0.0_f64;
    let mut pearson_chi2 = 0.0_f64;
    for (r, (&yr, &mur)) in y_counts.iter().zip(mus.iter()).enumerate() {
        let wr = mur.max(1e-12);
        for j in 0..p {
            let xrj = x.get(r, j).unwrap_or(0.0);
            for k in j..p {
                info_final[j * p + k] += wr * xrj * x.get(r, k).unwrap_or(0.0);
            }
        }
        if yr > 0.0 {
            dev_null += 2.0 * (yr * (yr / y_mean).ln() - (yr - y_mean));
            dev_resid += 2.0 * (yr * (yr / wr).ln() - (yr - wr));
        } else {
            dev_null += 2.0 * y_mean;
            dev_resid += 2.0 * wr;
        }
        pearson_chi2 += (yr - wr).powi(2) / wr;
    }
    for j in 0..p {
        for k in 0..j {
            info_final[j * p + k] = info_final[k * p + j];
        }
    }
    let info_mat = DenseMatrix::from_row_major(p, p, info_final)?;
    let cov_beta = match cholesky(&info_mat).and_then(|c| c.inverse()) {
        Ok(inv) => inv,
        Err(_) => DenseMatrix::identity(p)?,
    };

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let vifs = if compute_vif { compute_vifs_from_matrix(x, true) } else { vec![None; p] };
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
    let lr_chi2 = (2.0 * (ll_curr - ll_null)).max(0.0);
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
        deviance_null: dev_null.max(0.0),
        deviance_residual: dev_resid.max(0.0),
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

/// Assemble `(X, y, term_names)` from a `MultipleVariablesTable` (with categorical reference-level
/// dummy encoding) or `XyTable` (polynomial/single predictor).
fn build_glm_design(sheet: &DataSheet, glm_spec: &GlmSpec) -> Result<(DenseMatrix, Vec<f64>, Vec<String>), StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let resp_var = mv.variables.get(glm_spec.response_column).ok_or(StatsError::MissingColumn { index: glm_spec.response_column })?;
            if glm_spec.predictor_columns.is_empty() {
                return Err(StatsError::InsufficientSampleSize { context: "GLM predictors", required: 1, actual: 0 });
            }
            for &pc in &glm_spec.predictor_columns {
                if pc >= mv.variables.len() {
                    return Err(StatsError::MissingColumn { index: pc });
                }
            }

            // Prepare reference-coded levels for categorical columns.
            let mut cat_levels: Vec<Option<(String, Vec<String>)>> = Vec::with_capacity(glm_spec.predictor_columns.len());
            for &pc in &glm_spec.predictor_columns {
                let var = &mv.variables[pc];
                if var.role == VariableRole::Categorical {
                    let mut lvls: Vec<String> = var.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
                    let ref_lvl =
                        glm_spec.reference_levels.iter().find(|(idx, _)| *idx == pc).map(|(_, l)| l.clone()).or_else(|| lvls.first().cloned());
                    if let Some(rl) = ref_lvl {
                        lvls.retain(|l| l != &rl);
                        cat_levels.push(Some((rl, lvls)));
                    } else {
                        cat_levels.push(None);
                    }
                } else {
                    cat_levels.push(None);
                }
            }

            let mut term_names = vec![String::from("Intercept")];
            for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
                let title = &mv.variables[pc].title;
                if let Some((ref_lvl, non_refs)) = &cat_levels[pos] {
                    for lv in non_refs {
                        term_names.push(format!("{title}[{lv} vs {ref_lvl}]"));
                    }
                } else {
                    term_names.push(title.clone());
                }
            }

            let n_cols = term_names.len();
            let mut x_rows = Vec::new();
            let mut y_vec = Vec::new();
            for r in 0..mv.row_count() {
                let Some(yv) = resp_var.cells.get(r).and_then(|c| c.active_number()) else { continue };
                let mut row_x = Vec::with_capacity(n_cols);
                row_x.push(1.0);
                let mut ok = true;
                for (pos, &pc) in glm_spec.predictor_columns.iter().enumerate() {
                    let pred_var = mv.variables.get(pc).ok_or(StatsError::MissingColumn { index: pc })?;
                    let cell =
                        pred_var.cells.get(r).ok_or(StatsError::DimensionMismatch { expected: mv.row_count(), actual: pred_var.cells.len() })?;
                    if cell.excluded {
                        ok = false;
                        break;
                    }
                    if let Some((ref_lvl, non_refs)) = &cat_levels[pos] {
                        let tok = match &cell.value {
                            lumencraft_doc::CellValue::Text(s) => s.trim().to_owned(),
                            lumencraft_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
                            _ => {
                                ok = false;
                                break;
                            }
                        };
                        if tok != *ref_lvl && !non_refs.contains(&tok) {
                            ok = false;
                            break;
                        }
                        for lv in non_refs {
                            row_x.push(if &tok == lv { 1.0 } else { 0.0 });
                        }
                    } else if let Some(xv) = cell.active_number() {
                        row_x.push(xv);
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok {
                    x_rows.extend_from_slice(&row_x);
                    y_vec.push(yv);
                }
            }
            let n_obs = y_vec.len();
            if n_obs <= n_cols {
                return Err(StatsError::InsufficientSampleSize { context: "GLM observations", required: n_cols + 1, actual: n_obs });
            }
            let x_mat = DenseMatrix::from_row_major(n_obs, n_cols, x_rows)?;
            Ok((x_mat, y_vec, term_names))
        }
        TableArchetype::Xy(xy) => {
            let mut x_rows = Vec::new();
            let mut y_vec = Vec::new();
            let ds = xy.data_sets.first().ok_or(StatsError::EmptyInput)?;
            for r in 0..xy.row_count() {
                let Some(xv) = xy.x_cells.get(r).and_then(|s| s.first()).and_then(|c| c.active_number()) else { continue };
                let Some(yv) = ds.cell(r, 0).and_then(|c| c.active_number()) else { continue };
                x_rows.push(1.0);
                x_rows.push(xv);
                y_vec.push(yv);
            }
            let n_obs = y_vec.len();
            if n_obs < 3 {
                return Err(StatsError::InsufficientSampleSize { context: "GLM XY observations", required: 3, actual: n_obs });
            }
            let x_mat = DenseMatrix::from_row_major(n_obs, 2, x_rows)?;
            Ok((x_mat, y_vec, vec![String::from("Intercept"), xy.x_title.clone()]))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "generalized linear model", actual: other.kind() }),
    }
}

/// Complete Passing–Bablok (1983) nonparametric method-comparison regression output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassingBablokResult {
    pub n: usize,
    pub valid_slopes: usize,
    pub k_offset: usize,
    pub slope: f64,
    pub slope_ci: (f64, f64),
    pub intercept: f64,
    pub intercept_ci: (f64, f64),
    /// Raw maximum absolute cumulative sum `H = max_k |S_k|` across ordered residuals.
    pub cusum_h: f64,
    /// Standardized CUSUM statistic `H / sqrt(L + l)` distributed as Kolmogorov supremum.
    pub cusum_statistic: f64,
    /// Two-sided asymptotic Kolmogorov CUSUM linearity *p*-value.
    pub cusum_p_value: f64,
    pub linearity_passed: bool,
    pub slope_ci_clamped: bool,
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

    let mut slopes = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[j] - x[i];
            if dx == 0.0 {
                continue;
            }
            let s = (y[j] - y[i]) / dx;
            if !s.is_finite() || (s + 1.0).abs() <= 1e-12 {
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

    let slope = if n_slopes % 2 == 1 { angular[n_slopes / 2] } else { 0.5 * (angular[n_slopes / 2 - 1] + angular[n_slopes / 2]) };

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

    let (slope_ci, intercept_ci) = if raw_lo <= slope && slope <= raw_hi {
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

type PassingBablokInput = (Vec<f64>, Vec<f64>, String, String);

fn extract_passing_bablok_pairs(sheet: &DataSheet, x_col: usize, y_col: usize) -> Result<PassingBablokInput, StatsError> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (x_label, y_label) = match &sheet.archetype {
        TableArchetype::Column(t) => {
            if t.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let cx = t.data_sets.get(x_col).ok_or(StatsError::MissingColumn { index: x_col })?;
            let cy = t.data_sets.get(y_col).ok_or(StatsError::MissingColumn { index: y_col })?;
            for r in 0..t.row_count() {
                if let (Some(xv), Some(yv)) = (cx.cell(r, 0).and_then(Cell::active_number), cy.cell(r, 0).and_then(Cell::active_number)) {
                    xs.push(xv);
                    ys.push(yv);
                }
            }
            (cx.title.clone(), cy.title.clone())
        }
        TableArchetype::MultipleVariables(t) => {
            let vx = t.variables.get(x_col).ok_or(StatsError::MissingColumn { index: x_col })?;
            let vy = t.variables.get(y_col).ok_or(StatsError::MissingColumn { index: y_col })?;
            for r in 0..t.row_count() {
                if let (Some(xv), Some(yv)) = (vx.cells.get(r).and_then(Cell::active_number), vy.cells.get(r).and_then(Cell::active_number)) {
                    xs.push(xv);
                    ys.push(yv);
                }
            }
            (vx.title.clone(), vy.title.clone())
        }
        TableArchetype::Xy(t) => {
            if t.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let total_cols = 1 + t.data_sets.len();
            if x_col >= total_cols {
                return Err(StatsError::MissingColumn { index: x_col });
            }
            if y_col >= total_cols {
                return Err(StatsError::MissingColumn { index: y_col });
            }
            let label_for = |col: usize| -> String { if col == 0 { t.x_title.clone() } else { t.data_sets[col - 1].title.clone() } };
            let val_for = |row: usize, col: usize| -> Option<f64> {
                if col == 0 { t.x_value_at(row) } else { t.data_sets[col - 1].cell(row, 0).and_then(Cell::active_number) }
            };
            for r in 0..t.row_count() {
                if let (Some(xv), Some(yv)) = (val_for(r, x_col), val_for(r, y_col)) {
                    xs.push(xv);
                    ys.push(yv);
                }
            }
            (label_for(x_col), label_for(y_col))
        }
        other => return Err(StatsError::IncompatibleTable { analysis: "Passing-Bablok regression", actual: other.kind() }),
    };

    if xs.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    Ok((xs, ys, x_label, y_label))
}

/// Execute a [`PassingBablokSpec`] against a `Column`, `Xy`, or `MultipleVariables` [`DataSheet`]
/// using the caller's [`AnalysisSpec`] metadata.
pub fn run_passing_bablok_analysis(sheet: &DataSheet, spec: &AnalysisSpec, pb_spec: &PassingBablokSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let (xs, ys, x_label, y_label) = extract_passing_bablok_pairs(sheet, pb_spec.x_column, pb_spec.y_column)?;
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

    let param_headers = vec![String::from("Estimate"), format!("{}% CI", conf.percent()), String::from("Identity check")];
    let param_rows = vec![
        vec![
            num_cell(res.slope),
            ci_cell(res.slope_ci.0, res.slope_ci.1),
            ResultCell::Text(String::from(if prop_ok { "CI includes 1 (no proportional bias)" } else { "CI excludes 1 (proportional bias)" })),
        ],
        vec![
            num_cell(res.intercept),
            ci_cell(res.intercept_ci.0, res.intercept_ci.1),
            ResultCell::Text(String::from(if sys_ok { "CI includes 0 (no systematic bias)" } else { "CI excludes 0 (systematic bias)" })),
        ],
    ];
    out.tabs.push(build_tab(
        "pb-parameters",
        format!("Passing-Bablok regression ({y_label} vs {x_label})"),
        ResultTabKind::ParametersAndConfidenceIntervals,
        param_headers,
        vec![String::from("Slope (b)"), String::from("Intercept (a)")],
        param_rows,
    ));

    let summary_rows = vec![
        vec![num_cell(res.n as f64)],
        vec![num_cell(res.valid_slopes as f64)],
        vec![num_cell(res.k_offset as f64)],
        vec![num_cell(res.cusum_h)],
        vec![num_cell(res.cusum_statistic)],
        vec![p_cell(res.cusum_p_value, p_fmt)],
    ];
    out.tabs.push(build_tab(
        "pb-summary",
        "Passing-Bablok linearity & slope counts",
        ResultTabKind::MainSummary,
        vec![String::from("Value")],
        vec![
            String::from("Paired observations (n)"),
            String::from("Valid pairwise slopes (N)"),
            String::from("Negative-slope offset (K)"),
            String::from("CUSUM linearity H"),
            String::from("CUSUM standardized H / sqrt(L + l)"),
            String::from("CUSUM linearity P value"),
        ],
        summary_rows,
    ));

    let resid_labels: Vec<String> = (1..=res.n).map(|i| format!("Row {i}")).collect();
    let resid_rows: Vec<Vec<ResultCell>> = xs
        .iter()
        .zip(ys.iter())
        .zip(res.residuals.iter())
        .map(|((&xv, &yv), &rv)| vec![num_cell(xv), num_cell(yv), num_cell(res.intercept + res.slope * xv), num_cell(rv)])
        .collect();
    out.tabs.push(build_tab(
        "pb-residuals",
        "Passing-Bablok residuals",
        ResultTabKind::Residuals,
        vec![x_label, y_label, String::from("Fitted Y"), String::from("Residual (Y - a - bX)")],
        resid_labels,
        resid_rows,
    ));

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

    Ok(out)
}

/// Execute Passing–Bablok nonparametric regression against a [`DataSheet`] at confidence `conf`.
pub fn execute_passing_bablok(sheet: &DataSheet, spec: &PassingBablokSpec, conf: ConfidenceLevel) -> Result<ResultSheet, StatsError> {
    let mut analysis = AnalysisSpec::new(SheetId::new("passing-bablok"), sheet.id.clone(), AnalysisKind::PassingBablok(*spec))?;
    analysis.confidence_level = conf;
    run_passing_bablok_analysis(sheet, &analysis, spec)
}

/// Execute a [`GlmSpec`] (`Linear`, `Logistic`, or `Poisson`) against a `DataSheet`.
pub fn run_glm_analysis(sheet: &DataSheet, spec: &AnalysisSpec, glm_spec: &GlmSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let (x_mat, y_vec, term_names) = build_glm_design(sheet, glm_spec)?;

    match glm_spec.family {
        GlmFamily::Linear => {
            let res = multiple_linear_regression(&x_mat, &y_vec, &term_names, true, glm_spec.compute_vif, confidence)?;
            push_scalar(&mut out, "R squared", None, res.r_squared);
            push_scalar(&mut out, "Adjusted R squared", None, res.adjusted_r_squared);
            push_scalar(&mut out, "Residual SD (Sy.x)", None, res.residual_sd);
            push_scalar(&mut out, "F statistic", None, res.f_statistic);
            push_scalar(&mut out, "P value", None, res.f_p_value);
            push_scalar(&mut out, "AIC", None, res.aic);
            push_scalar(&mut out, "AICc", None, res.aicc);
            push_scalar(&mut out, "Durbin-Watson", None, res.durbin_watson);

            let mut headers = vec![
                String::from("Estimate"),
                String::from("SE"),
                String::from("t"),
                String::from("P value"),
                format!("{}% CI", confidence.percent()),
            ];
            if glm_spec.compute_vif {
                headers.push(String::from("VIF"));
            }
            let mut labels = Vec::with_capacity(res.terms.len());
            let mut rows = Vec::with_capacity(res.terms.len());
            for t in &res.terms {
                labels.push(t.name.clone());
                let mut r = vec![num_cell(t.estimate), num_cell(t.se), num_cell(t.t_statistic), p_cell(t.p_value, p_fmt), ci_cell(t.ci.0, t.ci.1)];
                if glm_spec.compute_vif {
                    r.push(opt_num_cell(t.vif));
                }
                rows.push(r);
            }
            out.tabs.push(build_tab(
                "glm-linear-params",
                "Multiple linear regression parameters",
                ResultTabKind::ParametersAndConfidenceIntervals,
                headers,
                labels.clone(),
                rows,
            ));

            // ANOVA table for regression.
            let anova_rows = vec![
                vec![
                    num_cell(res.ss_regression),
                    num_cell(res.df_model),
                    num_cell(res.ss_regression / res.df_model.max(1.0)),
                    num_cell(res.f_statistic),
                    p_cell(res.f_p_value, p_fmt),
                ],
                vec![num_cell(res.rss), num_cell(res.df_residual), num_cell(res.residual_sd * res.residual_sd), ResultCell::Empty, ResultCell::Empty],
                vec![num_cell(res.ss_total), num_cell((res.n - 1) as f64), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
            ];
            out.tabs.push(build_tab(
                "glm-linear-anova",
                "ANOVA of regression",
                ResultTabKind::AnovaTable,
                vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")],
                vec![String::from("Regression"), String::from("Residual"), String::from("Total")],
                anova_rows,
            ));

            // Type III partial sum of squares table.
            let mut type_iii_labels = Vec::new();
            let mut type_iii_rows = Vec::new();
            for t in &res.terms {
                if let (Some(ss), Some(f_val), Some(p_val)) = (t.type_iii_ss, t.type_iii_f, t.type_iii_p) {
                    type_iii_labels.push(t.name.clone());
                    type_iii_rows.push(vec![num_cell(ss), num_cell(1.0), num_cell(ss), num_cell(f_val), p_cell(p_val, p_fmt)]);
                }
            }
            if !type_iii_rows.is_empty() {
                out.tabs.push(build_tab(
                    "glm-linear-type-iii",
                    "Type III partial sums of squares",
                    ResultTabKind::AnovaTable,
                    vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")],
                    type_iii_labels,
                    type_iii_rows,
                ));
            }

            // Parameter covariance matrix sigma^2 * (X'X)^-1.
            let cov_rows: Vec<Vec<ResultCell>> = res.cov_beta.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
            out.tabs.push(build_tab(
                "glm-linear-covariance",
                "Parameter covariance matrix",
                ResultTabKind::CovarianceMatrix,
                labels.clone(),
                labels,
                cov_rows,
            ));

            // Residuals and influence diagnostics.
            let resid_labels: Vec<String> = (1..=res.n).map(|i| format!("Row {i}")).collect();
            let mut resid_rows = Vec::with_capacity(res.n);
            for (i, &yi) in y_vec.iter().enumerate().take(res.n) {
                resid_rows.push(vec![
                    num_cell(yi),
                    num_cell(res.fitted[i]),
                    num_cell(res.residuals[i]),
                    num_cell(res.leverage[i]),
                    num_cell(res.cooks_distance[i]),
                ]);
            }
            out.tabs.push(build_tab(
                "glm-linear-residuals",
                "Linear regression residuals and influence",
                ResultTabKind::Residuals,
                vec![
                    String::from("Observed Y"),
                    String::from("Predicted Y"),
                    String::from("Residual"),
                    String::from("Leverage"),
                    String::from("Cook's D"),
                ],
                resid_labels,
                resid_rows,
            ));

            // Checklists: VIF, Cook's distance, and residual normality (QOL-7 diagnostics-only).
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
        }
        GlmFamily::Logistic => {
            let res = multiple_logistic_regression(&x_mat, &y_vec, &term_names, glm_spec.compute_vif, confidence)?;
            push_scalar(&mut out, "Log-likelihood (model)", None, res.log_lik_model);
            push_scalar(&mut out, "LR chi-square", None, res.lr_chi2);
            push_scalar(&mut out, "LR P value", None, res.lr_p);
            push_scalar(&mut out, "McFadden R squared", None, res.mcfadden_r2);
            push_scalar(&mut out, "Nagelkerke R squared", None, res.nagelkerke_r2);
            push_scalar(&mut out, "Tjur R squared", None, res.tjur_r2);
            push_scalar(&mut out, "AIC", None, res.aic);
            push_scalar(&mut out, "AICc", None, res.aicc);
            push_scalar(&mut out, "Hosmer-Lemeshow chi2", None, res.hosmer_lemeshow_chi2);
            push_scalar(&mut out, "Hosmer-Lemeshow P", None, res.hosmer_lemeshow_p);
            if let Some(auc) = res.roc_auc {
                push_scalar(&mut out, "ROC AUC", None, auc);
            }

            let mut headers = vec![
                String::from("beta"),
                String::from("SE"),
                String::from("Wald z"),
                String::from("P value"),
                String::from("Odds Ratio"),
                format!("{}% CI (OR)", confidence.percent()),
            ];
            if glm_spec.compute_vif {
                headers.push(String::from("VIF"));
            }
            let mut labels = Vec::with_capacity(res.terms.len());
            let mut rows = Vec::with_capacity(res.terms.len());
            for t in &res.terms {
                labels.push(t.name.clone());
                let mut r = vec![
                    num_cell(t.beta),
                    num_cell(t.se),
                    num_cell(t.z_statistic),
                    p_cell(t.p_value, p_fmt),
                    num_cell(t.exp_beta),
                    ci_cell(t.exp_beta_ci.0, t.exp_beta_ci.1),
                ];
                if glm_spec.compute_vif {
                    r.push(opt_num_cell(t.vif));
                }
                rows.push(r);
            }
            out.tabs.push(build_tab(
                "glm-logistic-params",
                "Logistic regression odds ratios",
                ResultTabKind::ParametersAndConfidenceIntervals,
                headers,
                labels,
                rows,
            ));

            out.tabs.push(build_logistic_classification_tab(&y_vec, &res.predicted_probs));

            let pred_labels: Vec<String> = (1..=res.n).map(|i| format!("Row {i}")).collect();
            let pred_rows: Vec<Vec<ResultCell>> = y_vec
                .iter()
                .zip(res.predicted_probs.iter())
                .map(|(&yi, &pi)| vec![num_cell(yi), num_cell(pi), num_cell(if pi >= 0.5 { 1.0 } else { 0.0 })])
                .collect();
            out.tabs.push(build_tab(
                "glm-logistic-predictions",
                "Logistic regression predicted probabilities",
                ResultTabKind::Residuals,
                vec![String::from("Observed Y"), String::from("Predicted probability"), String::from("Predicted class")],
                pred_labels,
                pred_rows,
            ));

            out.checklist.push(checklist_item(
                "Logistic separation & calibration",
                if res.separation_detected { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
                Some(format!("Hosmer-Lemeshow P = {:.4}, AUC = {:.3}", res.hosmer_lemeshow_p, res.roc_auc.unwrap_or(0.5))),
                if res.separation_detected {
                    "Complete or quasi-complete separation detected; penalized safeguard applied to stabilize odds ratios."
                } else {
                    "IRLS converged without separation; Hosmer-Lemeshow P and ROC AUC summarize calibration and discrimination."
                },
            ));
        }
        GlmFamily::Poisson => {
            let res = multiple_poisson_regression(&x_mat, &y_vec, &term_names, glm_spec.compute_vif, confidence)?;
            let mcfadden_r2 = if res.log_lik_null.abs() > 1e-12 && res.log_lik_null.is_finite() && res.log_lik_model.is_finite() {
                (1.0 - res.log_lik_model / res.log_lik_null).clamp(0.0, 1.0)
            } else {
                0.0
            };
            push_scalar(&mut out, "Residual deviance", None, res.deviance_residual);
            push_scalar(&mut out, "Pearson dispersion", None, res.dispersion_pearson);
            push_scalar(&mut out, "LR chi-square", None, res.lr_chi2);
            push_scalar(&mut out, "LR P value", None, res.lr_p);
            push_scalar(&mut out, "AIC", None, res.aic);
            push_scalar(&mut out, "AICc", None, res.aicc);
            push_scalar(&mut out, "McFadden R squared", None, mcfadden_r2);

            let mut headers = vec![
                String::from("beta"),
                String::from("SE"),
                String::from("Wald z"),
                String::from("P value"),
                String::from("Rate Ratio"),
                format!("{}% CI (RR)", confidence.percent()),
            ];
            if glm_spec.compute_vif {
                headers.push(String::from("VIF"));
            }
            let mut labels = Vec::with_capacity(res.terms.len());
            let mut rows = Vec::with_capacity(res.terms.len());
            for t in &res.terms {
                labels.push(t.name.clone());
                let mut r = vec![
                    num_cell(t.beta),
                    num_cell(t.se),
                    num_cell(t.z_statistic),
                    p_cell(t.p_value, p_fmt),
                    num_cell(t.exp_beta),
                    ci_cell(t.exp_beta_ci.0, t.exp_beta_ci.1),
                ];
                if glm_spec.compute_vif {
                    r.push(opt_num_cell(t.vif));
                }
                rows.push(r);
            }
            out.tabs.push(build_tab(
                "glm-poisson-params",
                "Poisson regression rate ratios",
                ResultTabKind::ParametersAndConfidenceIntervals,
                headers,
                labels,
                rows,
            ));

            let resid_labels: Vec<String> = (1..=res.n).map(|i| format!("Row {i}")).collect();
            let resid_rows: Vec<Vec<ResultCell>> = y_vec
                .iter()
                .zip(res.predicted_means.iter())
                .map(|(&yi, &mui)| {
                    let mu = mui.max(1e-12);
                    let pearson = (yi - mui) / mu.sqrt();
                    let dev_sq = if yi > 0.0 { 2.0 * (yi * (yi / mu).ln() - (yi - mu)) } else { 2.0 * mu };
                    let dev_res = if yi >= mui { dev_sq.max(0.0).sqrt() } else { -dev_sq.max(0.0).sqrt() };
                    vec![num_cell(yi), num_cell(mui), num_cell(pearson), num_cell(dev_res)]
                })
                .collect();
            out.tabs.push(build_tab(
                "glm-poisson-residuals",
                "Poisson regression Pearson and deviance residuals",
                ResultTabKind::Residuals,
                vec![
                    String::from("Observed Y"),
                    String::from("Predicted count"),
                    String::from("Pearson residual"),
                    String::from("Deviance residual"),
                ],
                resid_labels,
                resid_rows,
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
        }
    }

    Ok(out)
}

/// Evaluate residual Gaussian normality for linear GLM without remediation suggestions (`QOL-7`).
fn residual_normality_checklist_item(residuals: &[f64], alpha: f64) -> lumencraft_doc::ChecklistItem {
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

/// Build logistic classification summary (cutoff 0.50) and cutoff-accuracy sweep (`0.1..=0.9`).
fn build_logistic_classification_tab(y: &[f64], probs: &[f64]) -> lumencraft_doc::ResultTab {
    fn metrics_row(y: &[f64], probs: &[f64], cutoff: f64) -> Vec<ResultCell> {
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
        vec![
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

    let headers = vec![
        String::from("Cutoff"),
        String::from("TP"),
        String::from("FP"),
        String::from("FN"),
        String::from("TN"),
        String::from("Sensitivity"),
        String::from("Specificity"),
        String::from("PPV"),
        String::from("NPV"),
        String::from("Accuracy"),
    ];
    let mut row_labels = Vec::with_capacity(10);
    let mut rows = Vec::with_capacity(10);
    row_labels.push(String::from("Cutoff 0.50 (2x2)"));
    rows.push(metrics_row(y, probs, 0.5));
    for k in 1..=9 {
        let cutoff = (k as f64) * 0.1;
        row_labels.push(format!("Sweep cutoff {cutoff:.1}"));
        rows.push(metrics_row(y, probs, cutoff));
    }
    build_tab("glm-logistic-classification", "Logistic classification table & cutoff sweep", ResultTabKind::Diagnostics, headers, row_labels, rows)
}
