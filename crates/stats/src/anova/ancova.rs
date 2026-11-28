//! General one-way / *k*-group Analysis of Covariance (ANCOVA) with one or more continuous
//! covariates, Type III (marginal) and Type I (sequential) sums of squares, homogeneity-of-slopes
//! interaction diagnostics, covariate-adjusted group means (estimated marginal means at grand
//! covariate means), and covariate-adjusted pairwise post-hoc comparisons.
//!
//! Uses a cell-means plus grand-mean-centered covariate parameterization
//! $Y = \sum_{g=0}^{k-1} \gamma_g \mathbf{1}_{G=g} + \sum_{j=1}^q \beta_j (X_j - \bar{x}_j) + \varepsilon$
//! so that $\hat{\gamma}_g$ equals the covariate-adjusted group mean evaluated at the grand
//! covariate centroid $\bar{X}$ and $\mathrm{Cov}(\hat{\theta}) = \mathrm{MS}_e (Z^\top Z)^{-1}$
//! supplies exact standard errors for both adjusted group means and arbitrary pairwise contrasts.
//!
//! References:
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed., Ch. 9). Lawrence Erlbaum Associates.
//! - Kutner, M. H., Nachtsheim, C. J., Neter, J., & Li, W. (2005). *Applied Linear Statistical
//!   Models* (5th ed., Ch. 22). McGraw-Hill/Irwin.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - Milliken, G. A., & Johnson, D. E. (2002). *Analysis of Messy Data, Volume III: Analysis of
//!   Covariance*. Chapman & Hall/CRC.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed., Ch. 18). Pearson Prentice Hall.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AncovaSpec, Cell, ChecklistStatus, DataSheet, MultiplicityMethod, MultiplicitySpec, PValueFormat, ResultCell,
    ResultSheet, ResultTabKind, SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{f_sf, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, solve_linear_least_squares, svd, sym_eigen};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};
use serde::{Deserialize, Serialize};

use crate::anova::effect_sizes::compute_anova_effect_sizes_for_n;
use crate::anova::factorial::checked_dense_vec;
use crate::anova::posthoc::{ContrastEstimate, ContrastFamilyEval, collect_factor_pairs, compare_contrast_family};
use crate::anova::reml::symmetrize_dense;
use crate::common::{
    build_tab, checklist_item, enforce_summary_mode_matrix, finite_ci_cell, init_result_sheet, num_cell, opt_num_cell, p_cell_at_alpha, push_scalar,
    resolve_selected_indices, sanitize_result_sheet_floats, t_crit_two_sided,
};
use crate::error::StatsError;
use crate::multiplicity::{PairwiseComparisonDetail, build_multiplicity_tab};

/// Structured complete-case observations for general one-way ANCOVA (`k >= 2` groups, `q >= 1` covariates).
#[derive(Debug, Clone)]
pub struct AncovaObservations {
    /// Display name of the continuous response variable.
    pub response_name: String,
    /// Display name of the categorical grouping factor.
    pub group_name: String,
    /// Ordered distinct level labels of the grouping factor (`len == k >= 2`).
    pub group_levels: Vec<String>,
    /// Display names of the continuous covariates (`len == q >= 1`).
    pub covariate_names: Vec<String>,
    /// Per complete observation: `(group_index, covariate_values, response_y)`.
    pub rows: Vec<(usize, Vec<f64>, f64)>,
    /// Number of input rows omitted due to missing, excluded, or non-numeric cells.
    pub omitted_rows: usize,
}

/// One source-of-variation row in the ANCOVA table (Group or Covariate term).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AncovaTermRow {
    /// Source name (group factor name or covariate name).
    pub name: String,
    /// Sum of squares (Type III marginal or Type I sequential).
    pub ss: f64,
    /// Numerator degrees of freedom.
    pub df: f64,
    /// Mean square `ss / df`.
    pub ms: f64,
    /// *F*-ratio `ms / ms_error`.
    pub f_statistic: f64,
    /// Upper-tail *F* *P*-value.
    pub p_value: f64,
    /// Partial eta-squared `SS_term / (SS_term + SS_error)`.
    pub partial_eta_squared: f64,
    /// Partial omega-squared `max(0, (SS_term - df_term * MS_error) / (SS_term + (n - df_term) * MS_error))`.
    pub partial_omega_squared: f64,
}

/// Unadjusted and covariate-adjusted (estimated marginal) mean summary for one factor level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AncovaAdjustedMean {
    /// Zero-based level index in [`AncovaObservations::group_levels`].
    pub group_index: usize,
    /// Level display label.
    pub label: String,
    /// Complete-case sample size in this group.
    pub n: usize,
    /// Raw unadjusted sample mean of *Y* in this group.
    pub unadjusted_mean: f64,
    /// Covariate-adjusted group mean (EMM / LS-mean) evaluated at grand covariate means.
    pub adjusted_mean: f64,
    /// Exact standard error of the adjusted group mean from `MS_e (Z^T Z)^{-1}`.
    pub se: f64,
    /// Two-sided confidence interval `(lower, upper)` for the adjusted group mean.
    pub ci: (f64, f64),
}

/// Common regression slope estimate for one continuous covariate in the additive ANCOVA model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AncovaCovariateSlope {
    /// Covariate display label.
    pub name: String,
    /// Grand mean of the covariate across all complete observations.
    pub grand_mean: f64,
    /// Pooled common regression slope estimate $\hat{\beta}_j$.
    pub estimate: f64,
    /// Exact standard error of $\hat{\beta}_j$.
    pub se: f64,
    /// Student *t*-statistic `estimate / se` on `df_error`.
    pub t_statistic: f64,
    /// Two-sided *P*-value testing $H_0: \beta_j = 0$.
    pub p_value: f64,
    /// Two-sided confidence interval `(lower, upper)` for $\beta_j$.
    pub ci: (f64, f64),
}

/// Homogeneity-of-regression-slopes interaction test (`Group × Covariates`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AncovaHomogeneityTest {
    /// Interaction sum of squares `SSE_additive - SSE_interaction`.
    pub ss_interaction: f64,
    /// Interaction numerator degrees of freedom.
    pub df_interaction: f64,
    /// Interaction mean square `ss_interaction / df_interaction`.
    pub ms_interaction: f64,
    /// Residual sum of squares under the interaction model.
    pub ss_residual_full: f64,
    /// Residual degrees of freedom under the interaction model.
    pub df_residual_full: f64,
    /// Residual mean square under the interaction model.
    pub ms_residual_full: f64,
    /// Omnibus homogeneity-of-slopes *F*-statistic.
    pub f_statistic: f64,
    /// Upper-tail *P*-value testing equal regression slopes across groups.
    pub p_value: f64,
    /// Per-covariate `Group × Covariate` partial interaction tests when multiple covariates are present.
    pub per_covariate_interactions: Vec<AncovaTermRow>,
}

/// Complete numerical output from [`fit_general_ancova`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AncovaResult {
    /// Number of complete observations included in the fit.
    pub n: usize,
    /// Number of distinct groups `k >= 2`.
    pub k_groups: usize,
    /// Number of continuous covariates `q >= 1`.
    pub n_covariates: usize,
    /// Sum-of-squares convention used (`TypeIII` or `TypeI`).
    pub ss_type: SumOfSquaresType,
    /// Group factor ANCOVA term row.
    pub group_term: AncovaTermRow,
    /// Covariate ANCOVA term rows (matching [`AncovaObservations::covariate_names`]).
    pub covariate_terms: Vec<AncovaTermRow>,
    /// Residual sum of squares of the additive ANCOVA model.
    pub ss_error: f64,
    /// Residual degrees of freedom `n - k - q`.
    pub df_error: f64,
    /// Residual mean square `ss_error / df_error`.
    pub ms_error: f64,
    /// Total corrected sum of squares $\sum (y_i - \bar{y})^2$.
    pub ss_total: f64,
    /// Total corrected degrees of freedom `n - 1`.
    pub df_total: f64,
    /// Overall model $R^2$.
    pub r_squared: f64,
    /// Adjusted $R^2$.
    pub adjusted_r_squared: f64,
    /// Unadjusted and covariate-adjusted group means (EMMs) at grand covariate means.
    pub adjusted_means: Vec<AncovaAdjustedMean>,
    /// Common covariate slopes and inference.
    pub covariate_slopes: Vec<AncovaCovariateSlope>,
    /// Optional homogeneity-of-regression-slopes interaction test (`None` if disabled or insufficient df).
    pub homogeneity_of_slopes: Option<AncovaHomogeneityTest>,
    /// Reason why homogeneity-of-slopes testing could not be computed when requested.
    pub homogeneity_unavailable_reason: Option<String>,
    /// Optional covariate-adjusted pairwise comparisons across group levels.
    pub pairwise_comparisons: Vec<PairwiseComparisonDetail>,
    /// Optional explanatory note attached to pairwise comparisons.
    pub pairwise_note: Option<String>,
    /// Observed *Y* values in complete-case order.
    pub observed_y: Vec<f64>,
    /// Additive-model fitted values $\hat{Y}_i$.
    pub fitted: Vec<f64>,
    /// Additive-model residuals $Y_i - \hat{Y}_i$.
    pub residuals: Vec<f64>,
}

fn partial_omega_squared(ss_term: f64, df_term: f64, ms_error: f64, n: usize) -> f64 {
    let nf = n as f64;
    let denom = ss_term + (nf - df_term) * ms_error;
    if denom > 0.0 && denom.is_finite() { ((ss_term - df_term * ms_error) / denom).clamp(0.0, 1.0) } else { 0.0 }
}

fn build_term_row(name: String, ss: f64, df: f64, ss_error: f64, df_error: f64, ms_error: f64, n: usize) -> Result<AncovaTermRow, StatsError> {
    let ss_clean = ss.max(0.0);
    let ms = if df > 0.0 { ss_clean / df } else { 0.0 };
    let (f_stat, p_val) = if ms_error > 0.0 && df > 0.0 {
        let f = ms / ms_error;
        (f, f_sf(f, df, df_error)?)
    } else if ss_clean > 0.0 && df > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };
    let partial_eta_squared = if ss_clean + ss_error > 0.0 { (ss_clean / (ss_clean + ss_error)).clamp(0.0, 1.0) } else { 0.0 };
    let partial_omega_squared = partial_omega_squared(ss_clean, df, ms_error, n);
    Ok(AncovaTermRow { name, ss: ss_clean, df, ms, f_statistic: f_stat, p_value: p_val, partial_eta_squared, partial_omega_squared })
}

fn ols_sse(x: &DenseMatrix, y: &[f64]) -> Result<f64, StatsError> {
    let (sol, _) = solve_linear_least_squares(x, y)?;
    Ok(sol.rss.max(0.0))
}

fn matrix_numerical_rank(x: &DenseMatrix) -> Result<usize, StatsError> {
    let dec = svd(x)?;
    let max_sv = dec.singular_values.iter().copied().fold(0.0_f64, f64::max);
    if !max_sv.is_finite() || max_sv <= 0.0 {
        return Ok(0);
    }
    let dim = x.nrows().max(x.ncols()).max(1) as f64;
    let tol = max_sv * dim * 1e-11;
    Ok(dec.singular_values.iter().filter(|&&s| s > tol).count())
}

fn invert_spd_full_rank(x: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let p = x.ncols();
    let xtx = symmetrize_dense(&x.transpose().matmul(x)?)?;
    let eig = sym_eigen(&xtx)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if !max_ev.is_finite() || max_ev <= 0.0 {
        return Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank: 0, expected_rank: p }));
    }
    let tol = max_ev.max(1.0) * (p.max(1) as f64) * 1e-11;
    let rank = eig.eigenvalues.iter().filter(|&&ev| ev.is_finite() && ev > tol).count();
    if rank < p {
        return Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank, expected_rank: p }));
    }
    let mut inv_data = checked_dense_vec(p, p)?;
    for (k, &ev) in eig.eigenvalues.iter().enumerate() {
        let inv_ev = 1.0 / ev;
        for i in 0..p {
            let vi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in 0..p {
                let vj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                inv_data[i * p + j] += vi * inv_ev * vj;
            }
        }
    }
    let inv = DenseMatrix::from_row_major(p, p, inv_data)?;
    symmetrize_dense(&inv)
}

/// Fit general one-way / *k*-group ANCOVA with `q >= 1` continuous covariates.
///
/// Validates finite observations, at least `2` groups with observations in every level, at least `1`
/// covariate, full rank of the additive design matrix, and residual degrees of freedom `n - k - q >= 1`.
pub fn fit_general_ancova(
    obs: &AncovaObservations,
    ss_type: SumOfSquaresType,
    test_homogeneity_of_slopes: bool,
    multiplicity: &MultiplicitySpec,
    confidence: ConfidenceLevel,
) -> Result<AncovaResult, StatsError> {
    let k = obs.group_levels.len();
    let q = obs.covariate_names.len();
    let n = obs.rows.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ANCOVA groups", required: 2, actual: k });
    }
    if q < 1 {
        return Err(StatsError::InsufficientSampleSize { context: "ANCOVA covariates", required: 1, actual: q });
    }
    let p_add = k + q;
    if n <= p_add {
        return Err(StatsError::InsufficientSampleSize { context: "ANCOVA complete observations", required: p_add + 1, actual: n });
    }

    let mut group_counts = vec![0_usize; k];
    let mut group_y_sums = vec![NeumaierSum::new(); k];
    let mut cov_sums = vec![NeumaierSum::new(); q];
    let mut y_vec = Vec::with_capacity(n);

    for &(g, ref covs, y) in &obs.rows {
        if g >= k {
            return Err(StatsError::InvalidParameter { name: "group_index", value: g as f64 });
        }
        if covs.len() != q {
            return Err(StatsError::DimensionMismatch { expected: q, actual: covs.len() });
        }
        if !y.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "ancova_y", value: y }));
        }
        for &xv in covs {
            if !xv.is_finite() {
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "ancova_covariate", value: xv }));
            }
        }
        group_counts[g] += 1;
        group_y_sums[g].add(y);
        for (j, &xv) in covs.iter().enumerate() {
            cov_sums[j].add(xv);
        }
        y_vec.push(y);
    }

    if let Some(min_cnt) = group_counts.iter().copied().min()
        && min_cnt == 0
    {
        return Err(StatsError::InsufficientSampleSize { context: "ANCOVA observations per group", required: 1, actual: 0 });
    }

    let nf = n as f64;
    let cov_means: Vec<f64> = cov_sums.iter().map(|s| s.total() / nf).collect();
    let y_moments = SummaryMoments::from_slice(&y_vec)?;
    let ss_total = y_moments.css.max(0.0);
    let df_total = (n - 1) as f64;

    // Build cell-means + grand-mean-centered covariate additive design matrix Z_add (n x (k + q)).
    let mut z_add_data = checked_dense_vec(n, p_add)?;
    for (i, &(g, ref covs, _)) in obs.rows.iter().enumerate() {
        z_add_data[i * p_add + g] = 1.0;
        for j in 0..q {
            z_add_data[i * p_add + k + j] = covs[j] - cov_means[j];
        }
    }
    let z_add = DenseMatrix::from_row_major(n, p_add, z_add_data)?;
    let rank_add = matrix_numerical_rank(&z_add)?;
    if rank_add < p_add {
        return Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { rank: rank_add, expected_rank: p_add }));
    }
    let xtx_inv = invert_spd_full_rank(&z_add)?;
    let (sol_add, _) = solve_linear_least_squares(&z_add, &y_vec)?;
    let theta = sol_add.coefficients;

    let mut fitted = Vec::with_capacity(n);
    let mut residuals = Vec::with_capacity(n);
    let mut sse_sum = NeumaierSum::new();
    for i in 0..n {
        let mut yhat = NeumaierSum::new();
        for j in 0..p_add {
            yhat.add(z_add.get(i, j).unwrap_or(0.0) * theta[j]);
        }
        let yh = yhat.total();
        let r = y_vec[i] - yh;
        fitted.push(yh);
        residuals.push(r);
        sse_sum.add(r * r);
    }
    let ss_error = sse_sum.total().max(0.0);
    let df_error = (n - p_add) as f64;
    let ms_error = ss_error / df_error;

    let r_squared = if ss_total > 0.0 { (1.0 - ss_error / ss_total).clamp(0.0, 1.0) } else { 0.0 };
    let adjusted_r_squared = if ss_total > 0.0 && df_error > 0.0 { 1.0 - (1.0 - r_squared) * (df_total / df_error) } else { 0.0 };

    // Reduced model without Group (intercept + all covariates) supplies Group SS for both Type III and Type I (covariates entered before group).
    let mut z_no_group_data = checked_dense_vec(n, 1 + q)?;
    for (i, (_, covs, _)) in obs.rows.iter().enumerate() {
        z_no_group_data[i * (1 + q)] = 1.0;
        for j in 0..q {
            z_no_group_data[i * (1 + q) + 1 + j] = covs[j] - cov_means[j];
        }
    }
    let z_no_group = DenseMatrix::from_row_major(n, 1 + q, z_no_group_data)?;
    let sse_no_group = ols_sse(&z_no_group, &y_vec)?;
    let ss_group = (sse_no_group - ss_error).max(0.0);
    let group_term = build_term_row(obs.group_name.clone(), ss_group, (k - 1) as f64, ss_error, df_error, ms_error, n)?;

    // Covariate sums of squares (Type III marginal vs Type I sequential).
    let mut covariate_terms = Vec::with_capacity(q);
    match ss_type {
        SumOfSquaresType::TypeIII => {
            for (j, name) in obs.covariate_names.iter().enumerate() {
                let diag = xtx_inv.get(k + j, k + j).unwrap_or(0.0);
                let ss_j = if diag > 0.0 { (theta[k + j] * theta[k + j] / diag).max(0.0) } else { 0.0 };
                covariate_terms.push(build_term_row(name.clone(), ss_j, 1.0, ss_error, df_error, ms_error, n)?);
            }
        }
        SumOfSquaresType::TypeI => {
            let mut prev_sse = ss_total;
            for (j, name) in obs.covariate_names.iter().enumerate() {
                let cols_j = 1 + (j + 1);
                let mut z_seq_data = checked_dense_vec(n, cols_j)?;
                for (i, (_, covs, _)) in obs.rows.iter().enumerate() {
                    z_seq_data[i * cols_j] = 1.0;
                    for m in 0..=j {
                        z_seq_data[i * cols_j + 1 + m] = covs[m] - cov_means[m];
                    }
                }
                let z_seq = DenseMatrix::from_row_major(n, cols_j, z_seq_data)?;
                let cur_sse = ols_sse(&z_seq, &y_vec)?;
                let ss_j = (prev_sse - cur_sse).max(0.0);
                prev_sse = cur_sse;
                covariate_terms.push(build_term_row(name.clone(), ss_j, 1.0, ss_error, df_error, ms_error, n)?);
            }
        }
    }

    let t_crit = t_crit_two_sided(confidence, df_error)?;

    // Adjusted group means (EMMs) at grand covariate means.
    let mut adjusted_means = Vec::with_capacity(k);
    for (g, label) in obs.group_levels.iter().enumerate() {
        let est = theta[g];
        let var_g = (ms_error * xtx_inv.get(g, g).unwrap_or(0.0)).max(0.0);
        let se = var_g.sqrt();
        let ci = (est - t_crit * se, est + t_crit * se);
        let unadj = group_y_sums[g].total() / (group_counts[g] as f64);
        adjusted_means.push(AncovaAdjustedMean {
            group_index: g,
            label: label.clone(),
            n: group_counts[g],
            unadjusted_mean: unadj,
            adjusted_mean: est,
            se,
            ci,
        });
    }

    // Covariate common slopes.
    let mut covariate_slopes = Vec::with_capacity(q);
    for (j, name) in obs.covariate_names.iter().enumerate() {
        let est = theta[k + j];
        let var_b = (ms_error * xtx_inv.get(k + j, k + j).unwrap_or(0.0)).max(0.0);
        let se = var_b.sqrt();
        let (t_stat, p_val) = if se > 0.0 {
            let t = est / se;
            (t, student_t_two_sided_p(t, df_error)?)
        } else if est != 0.0 {
            (est.signum() * f64::INFINITY, 0.0)
        } else {
            (0.0, 1.0)
        };
        let ci = (est - t_crit * se, est + t_crit * se);
        covariate_slopes.push(AncovaCovariateSlope {
            name: name.clone(),
            grand_mean: cov_means[j],
            estimate: est,
            se,
            t_statistic: t_stat,
            p_value: p_val,
            ci,
        });
    }

    // Optional homogeneity-of-slopes interaction test (`Y ~ Group + Covs + Group:Covs`).
    let (homogeneity_of_slopes, homogeneity_unavailable_reason) =
        if test_homogeneity_of_slopes { compute_homogeneity_of_slopes(obs, &cov_means, &y_vec, k, q, p_add, ss_error)? } else { (None, None) };

    // Optional covariate-adjusted pairwise comparisons across group levels.
    let (pairwise_comparisons, pairwise_note) = if multiplicity.method != MultiplicityMethod::None {
        if matches!(
            multiplicity.method,
            MultiplicityMethod::GamesHowell
                | MultiplicityMethod::TamhaneT2
                | MultiplicityMethod::DunnettT3
                | MultiplicityMethod::DunnRankSum
                | MultiplicityMethod::LinearTrend
        ) {
            return Err(StatsError::IncompatibleAnalysisSpec {
                expected: "ANCOVA pooled-covariance pairwise multiplicity procedure (None, TukeyHsd, Dunnett, Bonferroni, Sidak, HolmBonferroni, HolmSidak, NewmanKeuls, Duncan, FisherLsd, or FDR)",
            });
        }
        let pairs = collect_factor_pairs(k, multiplicity, |a, b| {
            let diff = theta[a] - theta[b];
            let v_aa = xtx_inv.get(a, a).unwrap_or(0.0);
            let v_bb = xtx_inv.get(b, b).unwrap_or(0.0);
            let v_ab = xtx_inv.get(a, b).unwrap_or(0.0);
            let se = (ms_error * (v_aa + v_bb - 2.0 * v_ab)).max(0.0).sqrt();
            let mut l_vec = vec![0.0_f64; k];
            l_vec[a] = 1.0;
            l_vec[b] = -1.0;
            let mut cov_l = vec![0.0_f64; k];
            for col in 0..k {
                cov_l[col] = ms_error * (xtx_inv.get(a, col).unwrap_or(0.0) - xtx_inv.get(b, col).unwrap_or(0.0));
            }
            Ok(Some(ContrastEstimate { diff, se, df: df_error, ctrl_var_pair: None, cov_vectors: Some((l_vec, cov_l)) }))
        })?;
        let levels: Vec<(usize, String)> = obs.group_levels.iter().enumerate().map(|(i, l)| (i, l.clone())).collect();
        let level_means: Vec<Option<f64>> = adjusted_means.iter().map(|m| Some(m.adjusted_mean)).collect();
        compare_contrast_family(ContrastFamilyEval {
            levels: &levels,
            level_means: &level_means,
            mult_spec: multiplicity,
            confidence,
            n_slices: 1,
            pairs,
        })?
    } else {
        (Vec::new(), None)
    };

    Ok(AncovaResult {
        n,
        k_groups: k,
        n_covariates: q,
        ss_type,
        group_term,
        covariate_terms,
        ss_error,
        df_error,
        ms_error,
        ss_total,
        df_total,
        r_squared,
        adjusted_r_squared,
        adjusted_means,
        covariate_slopes,
        homogeneity_of_slopes,
        homogeneity_unavailable_reason,
        pairwise_comparisons,
        pairwise_note,
        observed_y: y_vec,
        fitted,
        residuals,
    })
}

fn compute_homogeneity_of_slopes(
    obs: &AncovaObservations,
    cov_means: &[f64],
    y_vec: &[f64],
    k: usize,
    q: usize,
    p_add: usize,
    ss_error: f64,
) -> Result<(Option<AncovaHomogeneityTest>, Option<String>), StatsError> {
    let n = obs.rows.len();
    let p_full = k * (1 + q);
    if n <= p_full {
        return Ok((
            None,
            Some(format!(
                "Insufficient observations (n = {n}) to estimate all {p_full} separate-slope interaction parameters ({k} groups x {q} covariate(s))."
            )),
        ));
    }
    let mut z_full_data = checked_dense_vec(n, p_full)?;
    for (i, &(g, ref covs, _)) in obs.rows.iter().enumerate() {
        z_full_data[i * p_full + g] = 1.0;
        for j in 0..q {
            z_full_data[i * p_full + k * (1 + j) + g] = covs[j] - cov_means[j];
        }
    }
    let z_full = DenseMatrix::from_row_major(n, p_full, z_full_data)?;
    let rank_full = matrix_numerical_rank(&z_full)?;
    if rank_full <= p_add || n <= rank_full {
        return Ok((None, Some(String::from("Group-by-covariate interaction matrix is rank-deficient or leaves zero residual degrees of freedom."))));
    }
    let df_int = (rank_full - p_add) as f64;
    let df_res_full = (n - rank_full) as f64;
    let sse_full = ols_sse(&z_full, y_vec)?;
    let ss_int = (ss_error - sse_full).max(0.0);
    let ms_int = ss_int / df_int;
    let ms_res_full = sse_full / df_res_full;
    let (f_hom, p_hom) = if ms_res_full > 0.0 {
        let f = ms_int / ms_res_full;
        (f, f_sf(f, df_int, df_res_full)?)
    } else if ss_int > 0.0 {
        (f64::INFINITY, 0.0)
    } else {
        (0.0, 1.0)
    };

    let mut per_cov = Vec::new();
    if q > 1 {
        for (j_drop, cov_name) in obs.covariate_names.iter().enumerate() {
            // Drop Group:X_{j_drop} from the full interaction model (keeping common slope for X_{j_drop}).
            let p_red = p_full - (k - 1);
            let mut z_red_data = checked_dense_vec(n, p_red)?;
            for (i, &(g, ref covs, _)) in obs.rows.iter().enumerate() {
                let mut col_out = 0_usize;
                z_red_data[i * p_red + g] = 1.0;
                col_out += k;
                for j in 0..q {
                    if j == j_drop {
                        z_red_data[i * p_red + col_out] = covs[j] - cov_means[j];
                        col_out += 1;
                    } else {
                        z_red_data[i * p_red + col_out + g] = covs[j] - cov_means[j];
                        col_out += k;
                    }
                }
            }
            let z_red = DenseMatrix::from_row_major(n, p_red, z_red_data)?;
            let rank_red = matrix_numerical_rank(&z_red)?;
            if rank_full > rank_red {
                let sse_red = ols_sse(&z_red, y_vec)?;
                let ss_drop = (sse_red - sse_full).max(0.0);
                let df_drop = (rank_full - rank_red) as f64;
                per_cov.push(build_term_row(format!("{} × {cov_name}", obs.group_name), ss_drop, df_drop, sse_full, df_res_full, ms_res_full, n)?);
            }
        }
    }

    Ok((
        Some(AncovaHomogeneityTest {
            ss_interaction: ss_int,
            df_interaction: df_int,
            ms_interaction: ms_int,
            ss_residual_full: sse_full,
            df_residual_full: df_res_full,
            ms_residual_full: ms_res_full,
            f_statistic: f_hom,
            p_value: p_hom,
            per_covariate_interactions: per_cov,
        }),
        None,
    ))
}

fn cell_category_label(cell: &Cell) -> Option<String> {
    if cell.excluded {
        return None;
    }
    match &cell.value {
        curvebook_doc::CellValue::Empty => None,
        curvebook_doc::CellValue::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => {
            let rounded = v.round();
            if (*v - rounded).abs() <= 1e-12 && rounded.abs() < 1e15 { Some(format!("{}", rounded as i64)) } else { Some(format!("{v}")) }
        }
        _ => None,
    }
}

fn extract_ancova_observations(sheet: &DataSheet, spec: &AnalysisSpec, ancova_spec: &AncovaSpec) -> Result<AncovaObservations, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let n_cols = mv.variables.len();
            if ancova_spec.response_column >= n_cols {
                return Err(StatsError::MissingColumn { index: ancova_spec.response_column });
            }
            if ancova_spec.group_column >= n_cols {
                return Err(StatsError::MissingColumn { index: ancova_spec.group_column });
            }
            if ancova_spec.response_column == ancova_spec.group_column {
                return Err(StatsError::InvalidParameter { name: "group_column", value: ancova_spec.group_column as f64 });
            }
            if ancova_spec.covariate_columns.is_empty() {
                return Err(StatsError::InsufficientSampleSize { context: "ANCOVA covariates", required: 1, actual: 0 });
            }
            let mut cov_cols = Vec::with_capacity(ancova_spec.covariate_columns.len());
            for &c in &ancova_spec.covariate_columns {
                if c >= n_cols {
                    return Err(StatsError::MissingColumn { index: c });
                }
                if c == ancova_spec.response_column || c == ancova_spec.group_column {
                    return Err(StatsError::InvalidParameter { name: "covariate_columns", value: c as f64 });
                }
                if !cov_cols.contains(&c) {
                    cov_cols.push(c);
                }
            }

            let y_var = &mv.variables[ancova_spec.response_column];
            let g_var = &mv.variables[ancova_spec.group_column];
            let cov_names: Vec<String> = cov_cols.iter().map(|&c| mv.variables[c].title.clone()).collect();

            let mut level_map: BTreeMap<String, usize> = BTreeMap::new();
            let mut group_levels: Vec<String> = Vec::new();
            let mut rows = Vec::new();
            let mut omitted_rows = 0_usize;

            for r in 0..mv.row_count() {
                let y_opt = y_var.cells.get(r).and_then(Cell::active_number).filter(|v| v.is_finite());
                let g_opt = g_var.cells.get(r).and_then(cell_category_label);
                let mut cov_vals = Vec::with_capacity(cov_cols.len());
                let mut covs_ok = true;
                for &c in &cov_cols {
                    if let Some(v) = mv.variables[c].cells.get(r).and_then(Cell::active_number).filter(|v| v.is_finite()) {
                        cov_vals.push(v);
                    } else {
                        covs_ok = false;
                        break;
                    }
                }
                let (Some(y), Some(g_lbl)) = (y_opt, g_opt) else {
                    omitted_rows += 1;
                    continue;
                };
                if !covs_ok {
                    omitted_rows += 1;
                    continue;
                }
                let g_idx = if let Some(&idx) = level_map.get(&g_lbl) {
                    idx
                } else {
                    let idx = group_levels.len();
                    level_map.insert(g_lbl.clone(), idx);
                    group_levels.push(g_lbl);
                    idx
                };
                rows.push((g_idx, cov_vals, y));
            }

            Ok(AncovaObservations {
                response_name: y_var.title.clone(),
                group_name: g_var.title.clone(),
                group_levels,
                covariate_names: cov_names,
                rows,
                omitted_rows,
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let selected = resolve_selected_indices(&spec.selected_data_sets, xy.data_sets.len());
            if selected.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "ANCOVA XY groups", required: 2, actual: selected.len() });
            }
            let group_levels: Vec<String> = selected.iter().map(|&c| xy.data_sets[c].title.clone()).collect();
            let cov_name = if xy.x_title.trim().is_empty() { String::from("X") } else { xy.x_title.clone() };
            let mut rows = Vec::new();
            let mut omitted_rows = 0_usize;
            for r in 0..xy.row_count() {
                let x_opt = xy.x_summary_at(r).ok().flatten().filter(|v| v.is_finite());
                for (g_pos, &ds_idx) in selected.iter().enumerate() {
                    let Some(sub_cells) = xy.data_sets[ds_idx].cells.get(r) else {
                        continue;
                    };
                    let y_subs = match xy.y_subcolumn_mode {
                        SubcolumnMode::Single => 1,
                        SubcolumnMode::RawReplicates(k) => k.max(1),
                        _ => 1,
                    };
                    for sub in 0..y_subs.min(sub_cells.len()) {
                        let cell = &sub_cells[sub];
                        if let Some(y) = cell.active_number().filter(|v| v.is_finite()) {
                            if let Some(x) = x_opt {
                                rows.push((g_pos, vec![x], y));
                            } else {
                                omitted_rows += 1;
                            }
                        } else if !cell.is_empty() || cell.excluded {
                            omitted_rows += 1;
                        }
                    }
                }
            }
            Ok(AncovaObservations {
                response_name: String::from("Y"),
                group_name: String::from("Group"),
                group_levels,
                covariate_names: vec![cov_name],
                rows,
                omitted_rows,
            })
        }
        other => Err(StatsError::IncompatibleTable { analysis: "general ANCOVA", actual: other.kind() }),
    }
}

/// Execute [`AnalysisKind::Ancova`] against a [`DataSheet`] (`MultipleVariables` or `Xy`) and
/// produce a complete [`ResultSheet`].
pub fn run_ancova_analysis(sheet: &DataSheet, spec: &AnalysisSpec, ancova_spec: &AncovaSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::Ancova(ancova_spec.clone()))?;
    let obs = extract_ancova_observations(sheet, spec, ancova_spec)?;
    let res =
        fit_general_ancova(&obs, ancova_spec.ss_type, ancova_spec.test_homogeneity_of_slopes, &ancova_spec.multiplicity, spec.confidence_level)?;

    let mut out = init_result_sheet(spec)?;
    let p_fmt: PValueFormat = spec.p_value_format;
    let alpha = spec.confidence_level.alpha();
    let ci_header = format!("{}% CI", spec.confidence_level.percent());
    let ss_label = match res.ss_type {
        SumOfSquaresType::TypeIII => "Type III SS",
        SumOfSquaresType::TypeI => "Type I SS",
    };

    // Hookable scalars.
    push_scalar(&mut out, "Group F", None, res.group_term.f_statistic);
    push_scalar(&mut out, "Group P value", None, res.group_term.p_value);
    push_scalar(&mut out, "Group partial eta^2", None, res.group_term.partial_eta_squared);
    push_scalar(&mut out, "R squared", None, res.r_squared);
    push_scalar(&mut out, "Adjusted R squared", None, res.adjusted_r_squared);
    push_scalar(&mut out, "Residual MS", None, res.ms_error);
    push_scalar(&mut out, "Residual df", None, res.df_error);
    push_scalar(&mut out, "Complete observations (n)", None, res.n as f64);
    for adj in &res.adjusted_means {
        push_scalar(&mut out, format!("Adjusted mean ({})", adj.label), None, adj.adjusted_mean);
    }
    if let Some(ref hom) = res.homogeneity_of_slopes {
        push_scalar(&mut out, "Homogeneity of slopes F", None, hom.f_statistic);
        push_scalar(&mut out, "Homogeneity of slopes P value", None, hom.p_value);
    }

    // 1. ANCOVA Source Table.
    let mut table_headers = vec![String::from(ss_label), String::from("DF"), String::from("MS"), String::from("F"), String::from("P value")];
    if ancova_spec.compute_effect_sizes {
        table_headers.push(String::from("Partial eta^2"));
        table_headers.push(String::from("Partial omega^2"));
    }
    let mut table_labels = Vec::new();
    let mut table_rows = Vec::new();

    let push_term = |term: &AncovaTermRow, labels: &mut Vec<String>, rows: &mut Vec<Vec<ResultCell>>| {
        labels.push(term.name.clone());
        let mut r =
            vec![num_cell(term.ss), num_cell(term.df), num_cell(term.ms), num_cell(term.f_statistic), p_cell_at_alpha(term.p_value, p_fmt, alpha)];
        if ancova_spec.compute_effect_sizes {
            r.push(num_cell(term.partial_eta_squared));
            r.push(num_cell(term.partial_omega_squared));
        }
        rows.push(r);
    };

    if res.ss_type == SumOfSquaresType::TypeI {
        for cov in &res.covariate_terms {
            push_term(cov, &mut table_labels, &mut table_rows);
        }
        push_term(&res.group_term, &mut table_labels, &mut table_rows);
    } else {
        push_term(&res.group_term, &mut table_labels, &mut table_rows);
        for cov in &res.covariate_terms {
            push_term(cov, &mut table_labels, &mut table_rows);
        }
    }

    table_labels.push(String::from("Residual"));
    let mut resid_row = vec![num_cell(res.ss_error), num_cell(res.df_error), num_cell(res.ms_error), ResultCell::Empty, ResultCell::Empty];
    if ancova_spec.compute_effect_sizes {
        resid_row.push(ResultCell::Empty);
        resid_row.push(ResultCell::Empty);
    }
    table_rows.push(resid_row);

    table_labels.push(String::from("Total"));
    let mut tot_row = vec![num_cell(res.ss_total), num_cell(res.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if ancova_spec.compute_effect_sizes {
        tot_row.push(ResultCell::Empty);
        tot_row.push(ResultCell::Empty);
    }
    table_rows.push(tot_row);

    let mut ancova_tab =
        build_tab("ancova-table", format!("ANCOVA table ({ss_label})"), ResultTabKind::AnovaTable, table_headers, table_labels, table_rows);
    let group_eff = compute_anova_effect_sizes_for_n(
        res.group_term.ss,
        res.ss_error,
        res.ss_total,
        res.group_term.df,
        res.df_error,
        res.n,
        spec.confidence_level,
    );
    let eff_ci_note = group_eff
        .partial_eta_squared_ci
        .map(|(lo, hi)| format!(" Group partial eta^2 {}% CI: [{lo:.4}, {hi:.4}].", spec.confidence_level.percent()))
        .unwrap_or_default();
    ancova_tab.footnotes.push(format!(
        "Additive ANCOVA model: {} ~ {} + {}; R^2 = {:.4}, Adjusted R^2 = {:.4}, Residual SD = {:.4}.{eff_ci_note}",
        obs.response_name,
        obs.group_name,
        obs.covariate_names.join(" + "),
        res.r_squared,
        res.adjusted_r_squared,
        res.ms_error.sqrt()
    ));
    out.tabs.push(ancova_tab);

    // 2. Adjusted Group Means (EMMs) tab.
    let adj_labels: Vec<String> = res.adjusted_means.iter().map(|m| m.label.clone()).collect();
    let adj_rows: Vec<Vec<ResultCell>> = res
        .adjusted_means
        .iter()
        .map(|m| vec![num_cell(m.n as f64), num_cell(m.unadjusted_mean), num_cell(m.adjusted_mean), num_cell(m.se), finite_ci_cell(m.ci.0, m.ci.1)])
        .collect();
    let cov_at_desc: Vec<String> = res.covariate_slopes.iter().map(|c| format!("{} = {:.4}", c.name, c.grand_mean)).collect();
    let mut adj_tab = build_tab(
        "ancova-adjusted-means",
        "Covariate-adjusted group means (EMMs)",
        ResultTabKind::MainSummary,
        vec![String::from("N"), String::from("Unadjusted Mean"), String::from("Adjusted Mean"), String::from("SE"), ci_header.clone()],
        adj_labels,
        adj_rows,
    );
    adj_tab.footnotes.push(format!("Estimated marginal means evaluated at grand covariate means: {}.", cov_at_desc.join(", ")));
    out.tabs.push(adj_tab);

    // 3. Covariate Common Slopes tab.
    let slope_labels: Vec<String> = res.covariate_slopes.iter().map(|c| c.name.clone()).collect();
    let slope_rows: Vec<Vec<ResultCell>> = res
        .covariate_slopes
        .iter()
        .map(|c| {
            vec![
                num_cell(c.grand_mean),
                num_cell(c.estimate),
                num_cell(c.se),
                num_cell(c.t_statistic),
                p_cell_at_alpha(c.p_value, p_fmt, alpha),
                finite_ci_cell(c.ci.0, c.ci.1),
            ]
        })
        .collect();
    out.tabs.push(build_tab(
        "ancova-covariate-slopes",
        "Common covariate regression slopes",
        ResultTabKind::ParametersAndConfidenceIntervals,
        vec![String::from("Grand Mean"), String::from("Slope (beta)"), String::from("SE"), String::from("t"), String::from("P value"), ci_header],
        slope_labels,
        slope_rows,
    ));

    // 4. Homogeneity-of-Slopes Interaction Check tab.
    if let Some(ref hom) = res.homogeneity_of_slopes {
        let mut h_labels = vec![format!("{} × Covariates (Omnibus)", obs.group_name)];
        let mut h_rows = vec![vec![
            num_cell(hom.ss_interaction),
            num_cell(hom.df_interaction),
            num_cell(hom.ms_interaction),
            num_cell(hom.f_statistic),
            p_cell_at_alpha(hom.p_value, p_fmt, alpha),
        ]];
        for term in &hom.per_covariate_interactions {
            h_labels.push(term.name.clone());
            h_rows.push(vec![
                num_cell(term.ss),
                num_cell(term.df),
                num_cell(term.ms),
                num_cell(term.f_statistic),
                p_cell_at_alpha(term.p_value, p_fmt, alpha),
            ]);
        }
        h_labels.push(String::from("Interaction-model Residual"));
        h_rows.push(vec![
            num_cell(hom.ss_residual_full),
            num_cell(hom.df_residual_full),
            num_cell(hom.ms_residual_full),
            ResultCell::Empty,
            ResultCell::Empty,
        ]);
        let mut h_tab = build_tab(
            "ancova-homogeneity",
            "Homogeneity of regression slopes (interaction check)",
            ResultTabKind::ModelComparison,
            vec![String::from("SS"), String::from("DF"), String::from("MS"), String::from("F"), String::from("P value")],
            h_labels,
            h_rows,
        );
        h_tab.footnotes.push(String::from(
            "Tests whether group regression slopes are parallel (H0: Group × Covariate interactions = 0). A small P value indicates non-parallel slopes.",
        ));
        out.tabs.push(h_tab);

        let parallel_ok = hom.p_value.is_finite() && hom.p_value >= alpha;
        out.checklist.push(checklist_item(
            "Homogeneity of regression slopes",
            if parallel_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("F({:.0}, {:.0}) = {:.4}, P = {:.4}", hom.df_interaction, hom.df_residual_full, hom.f_statistic, hom.p_value)),
            if parallel_ok {
                "No significant group-by-covariate interaction was detected; the parallel-slopes ANCOVA model is supported."
            } else {
                "Significant group-by-covariate interaction detected; group differences vary with covariate level, so interpret adjusted means with caution."
            },
        ));
    } else if let Some(ref reason) = res.homogeneity_unavailable_reason {
        out.checklist.push(checklist_item(
            "Homogeneity of regression slopes",
            ChecklistStatus::Info,
            Some(String::from("Interaction test skipped")),
            reason.clone(),
        ));
    }

    // 5. Covariate-adjusted pairwise comparisons tab.
    if !res.pairwise_comparisons.is_empty() {
        let mut mc_tab =
            build_multiplicity_tab(&res.pairwise_comparisons, "Covariate-adjusted pairwise comparisons (EMMs)", p_fmt, spec.confidence_level);
        mc_tab.id = String::from("ancova-posthoc");
        if let Some(ref note) = res.pairwise_note {
            mc_tab.footnotes.push(note.clone());
        }
        out.tabs.push(mc_tab);
        for d in &res.pairwise_comparisons {
            out.pairwise_comparisons.push(d.to_record());
        }
    }

    // 6. Residuals tab for diagnostic plotting (`ResidualPlot`, `NormalQqPlot`, `HomoscedasticityPlot`, `ActualVsPredictedPlot`).
    let resid_labels: Vec<String> = (1..=res.n).map(|i| format!("Obs {i}")).collect();
    let resid_rows: Vec<Vec<ResultCell>> =
        (0..res.n).map(|i| vec![num_cell(res.observed_y[i]), num_cell(res.fitted[i]), num_cell(res.residuals[i]), opt_num_cell(None)]).collect();
    out.tabs.push(build_tab(
        "ancova-residuals",
        "ANCOVA fitted values & residuals",
        ResultTabKind::Residuals,
        vec![format!("Observed ({})", obs.response_name), String::from("Predicted Y"), String::from("Residual"), String::from("Flag")],
        resid_labels,
        resid_rows,
    ));

    // Residual normality diagnostic (Shapiro-Wilk, 3 <= n <= 5000).
    out.checklist.push(build_ancova_residual_normality_checklist(&res.residuals, alpha));

    // Observation accounting & model fit checklist items.
    out.checklist.push(checklist_item(
        "Complete-case observations analyzed",
        if obs.omitted_rows == 0 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("n = {} complete rows across {} groups, {} covariate(s); {} omitted", res.n, res.k_groups, res.n_covariates, obs.omitted_rows)),
        if obs.omitted_rows > 0 {
            "Rows with missing, excluded, or non-numeric response, group, or covariate entries were excluded listwise."
        } else {
            "All rows had finite response, group, and covariate values."
        },
    ));

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

fn build_ancova_residual_normality_checklist(residuals: &[f64], alpha: f64) -> curvebook_doc::ChecklistItem {
    if residuals.len() < 3 {
        return checklist_item(
            "Residual normality (Shapiro-Wilk)",
            ChecklistStatus::Info,
            Some(format!("n = {}", residuals.len())),
            "Fewer than 3 residuals are available; Shapiro-Wilk normality testing requires 3 to 5000 residuals.",
        );
    }
    if residuals.len() > 5000 {
        return checklist_item(
            "Residual normality (Shapiro-Wilk)",
            ChecklistStatus::Info,
            Some(format!("n = {}", residuals.len())),
            "Residual count exceeds the Shapiro-Wilk upper limit of 5000 observations; inspect the Normal Q-Q plot.",
        );
    }
    match crate::column::shapiro_wilk(residuals, alpha) {
        Ok(sw) => checklist_item(
            "Residual normality (Shapiro-Wilk)",
            if sw.passed { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("Shapiro-Wilk W = {:.4}, P = {:.4}", sw.statistic, sw.p_value)),
            if sw.passed {
                "ANCOVA residuals do not depart significantly from Gaussian normality."
            } else {
                "ANCOVA residuals depart significantly from Gaussian normality (P < alpha); inspect the Normal Q-Q plot."
            },
        ),
        Err(StatsError::ZeroVariance { .. }) => checklist_item(
            "Residual normality (Shapiro-Wilk)",
            ChecklistStatus::Info,
            Some(String::from("exact fit (zero residual variance)")),
            "ANCOVA residuals have zero variance (exact model fit); Shapiro-Wilk W is degenerate.",
        ),
        Err(err) => checklist_item(
            "Residual normality (Shapiro-Wilk)",
            ChecklistStatus::Info,
            None,
            format!("Residual normality diagnostic unavailable: {err}."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, AncovaSpec, Cell, DataSheet, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, SheetId,
        SubcolumnMode, SumOfSquaresType, TableArchetype, VariableRole, XyTable,
    };
    use curvebook_math::ConfidenceLevel;

    use super::{AncovaObservations, fit_general_ancova, run_ancova_analysis};
    use crate::error::StatsError;

    fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
        assert!((actual - expected).abs() <= tol, "{label}: expected {expected:.12}, got {actual:.12} (diff {:.3e})", (actual - expected).abs());
    }

    /// Cross-validated against R 4.5.2 (`lm(y ~ g + x)`, `drop1(..., test="F")`, `anova(lm(y ~ x + g))`,
    /// `anova(lm(y ~ g + x), lm(y ~ g * x))`, `predict(..., se.fit=TRUE)`, and `p.adjust(..., "holm")`).
    #[test]
    fn d1_oracle_ancova_single_covariate_unbalanced_matches_r_4_5_2() -> Result<(), Box<dyn std::error::Error>> {
        let obs = AncovaObservations {
            response_name: String::from("Y"),
            group_name: String::from("Group"),
            group_levels: vec![String::from("Ctrl"), String::from("Low"), String::from("High")],
            covariate_names: vec![String::from("X")],
            rows: vec![
                (0, vec![1.0], 10.2),
                (0, vec![2.5], 13.1),
                (0, vec![4.0], 16.5),
                (0, vec![5.5], 19.0),
                (0, vec![7.0], 22.4),
                (1, vec![1.5], 14.8),
                (1, vec![3.0], 17.5),
                (1, vec![5.0], 22.1),
                (1, vec![6.5], 24.9),
                (2, vec![2.0], 19.2),
                (2, vec![3.5], 22.8),
                (2, vec![4.5], 24.6),
                (2, vec![6.0], 28.0),
                (2, vec![8.0], 31.9),
            ],
            omitted_rows: 0,
        };
        let mult = MultiplicitySpec { method: MultiplicityMethod::HolmBonferroni, ..MultiplicitySpec::default() };
        let res = fit_general_ancova(&obs, SumOfSquaresType::TypeIII, true, &mult, ConfidenceLevel::DEFAULT_95)?;

        // Additive model summary vs R 4.5.2
        assert_close(res.r_squared, 0.998_822_236_376_964, 1e-11, "R2");
        assert_close(res.adjusted_r_squared, 0.998_468_907_290_053, 1e-11, "AdjR2");
        assert_close(res.ss_error, 0.537_201_543_739_28, 1e-11, "SSE");
        assert_close(res.df_error, 10.0, 1e-12, "df_error");
        assert_close(res.ms_error, 0.053_720_154_373_928, 1e-11, "MSE");

        // Type III SS (`drop1`) vs R 4.5.2
        assert_close(res.group_term.ss, 133.626_538_101_2, 1e-9, "Group Type III SS");
        assert_close(res.group_term.f_statistic, 1_243.728_17, 1e-4, "Group Type III F");
        assert_close(res.group_term.p_value, 1.029_2e-12, 1e-15, "Group Type III P");
        assert_close(res.covariate_terms[0].ss, 247.822_298_456_3, 1e-9, "X Type III SS");
        assert_close(res.covariate_terms[0].f_statistic, 4_613.208_98, 1e-4, "X Type III F");
        assert_close(res.covariate_terms[0].p_value, 1.166_2e-14, 1e-16, "X Type III P");

        // Common covariate slope vs R 4.5.2
        assert_close(res.covariate_slopes[0].estimate, 2.061_749_571_183_53, 1e-11, "X slope");
        assert_close(res.covariate_slopes[0].se, 0.030_355_287_368_939_7, 1e-11, "X SE");

        // Homogeneity of slopes (`anova(m_add, m_int)`) vs R 4.5.2
        let hom = res.homogeneity_of_slopes.as_ref().ok_or("missing homogeneity test")?;
        assert_close(hom.ss_interaction, 0.084_885_937_457_91, 1e-10, "Homogeneity SS");
        assert_close(hom.df_interaction, 2.0, 1e-12, "Homogeneity df1");
        assert_close(hom.df_residual_full, 8.0, 1e-12, "Homogeneity df2");
        assert_close(hom.f_statistic, 0.750_68, 1e-4, "Homogeneity F");
        assert_close(hom.p_value, 0.502_59, 1e-4, "Homogeneity P");

        // Adjusted means (EMMs at grand mean x = 4.28571428571429) vs R 4.5.2
        assert_close(res.adjusted_means[0].adjusted_mean, 16.829_071_306_052_4, 1e-10, "EMM Ctrl");
        assert_close(res.adjusted_means[0].se, 0.104_015_627_431_602, 1e-10, "EMM Ctrl SE");
        assert_close(res.adjusted_means[0].ci.0, 16.597_310_045_358_4, 1e-10, "EMM Ctrl CI lo");
        assert_close(res.adjusted_means[0].ci.1, 17.060_832_566_746_5, 1e-10, "EMM Ctrl CI hi");
        assert_close(res.adjusted_means[1].adjusted_mean, 20.414_071_306_052_4, 1e-10, "EMM Low");
        assert_close(res.adjusted_means[1].se, 0.116_212_127_029_352, 1e-10, "EMM Low SE");
        assert_close(res.adjusted_means[2].adjusted_mean, 24.239_671_649_105_6, 1e-10, "EMM High");
        assert_close(res.adjusted_means[2].se, 0.104_822_436_865_621, 1e-10, "EMM High SE");

        // Pairwise contrast SEs and Holm-Bonferroni adjusted P values vs R 4.5.2
        assert_eq!(res.pairwise_comparisons.len(), 3);
        assert_close(res.pairwise_comparisons[0].mean_diff, -3.585_0, 1e-10, "Pair 0-1 diff");
        assert_close(res.pairwise_comparisons[0].se_diff, 0.155_480_125_638, 1e-10, "Pair 0-1 SE");
        assert_close(res.pairwise_comparisons[0].adjusted_p_or_q, 6.321_468_594_220_84e-10, 1e-13, "Pair 0-1 Holm P");
        assert_close(res.pairwise_comparisons[1].mean_diff, -7.410_600_343_053, 1e-10, "Pair 0-2 diff");
        assert_close(res.pairwise_comparisons[1].adjusted_p_or_q, 7.611_653_846_004_1e-13, 1e-15, "Pair 0-2 Holm P");

        // Type I SS (`anova(lm(y ~ x + g))`) vs R 4.5.2
        let res_t1 = fit_general_ancova(&obs, SumOfSquaresType::TypeI, false, &MultiplicitySpec::default(), ConfidenceLevel::DEFAULT_95)?;
        assert_close(res_t1.covariate_terms[0].ss, 321.956_260_355_0, 1e-9, "X Type I SS");
        assert_close(res_t1.group_term.ss, 133.626_538_101_2, 1e-9, "Group Type I SS");
        Ok(())
    }

    /// Cross-validated against R 4.5.2 (`lm(y2 ~ g + x + x2)`, `drop1`, `anova(m2_add, m2_int)`, `predict`).
    #[test]
    fn d1_oracle_ancova_two_covariates_matches_r_4_5_2() -> Result<(), Box<dyn std::error::Error>> {
        let g = [0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2];
        let x1 = [1.0, 2.5, 4.0, 5.5, 7.0, 1.5, 3.0, 5.0, 6.5, 2.0, 3.5, 4.5, 6.0, 8.0];
        let x2 = [5.0, 3.0, 6.0, 2.0, 8.0, 4.0, 7.0, 2.0, 5.0, 6.0, 3.0, 7.0, 4.0, 9.0];
        let y_base = [10.2, 13.1, 16.5, 19.0, 22.4, 14.8, 17.5, 22.1, 24.9, 19.2, 22.8, 24.6, 28.0, 31.9];
        let eps = [0.1, -0.2, 0.3, -0.1, 0.2, -0.3, 0.4, -0.2, 0.1, 0.2, -0.1, 0.3, -0.4, 0.1];
        let rows: Vec<_> = (0..14).map(|i| (g[i], vec![x1[i], x2[i]], y_base[i] + 0.7 * x2[i] + eps[i])).collect();

        let obs = AncovaObservations {
            response_name: String::from("Y2"),
            group_name: String::from("Group"),
            group_levels: vec![String::from("Ctrl"), String::from("Low"), String::from("High")],
            covariate_names: vec![String::from("X1"), String::from("X2")],
            rows,
            omitted_rows: 0,
        };
        let res = fit_general_ancova(&obs, SumOfSquaresType::TypeIII, true, &MultiplicitySpec::default(), ConfidenceLevel::DEFAULT_95)?;
        assert_close(res.ss_error, 0.504_716_806_56, 1e-9, "2-cov SSE");
        assert_close(res.group_term.ss, 126.661_037_883_06, 1e-8, "2-cov Group SS");
        assert_close(res.group_term.f_statistic, 1_129.296_00, 1e-3, "2-cov Group F");
        assert_close(res.covariate_terms[0].ss, 225.988_856_572_28, 1e-8, "2-cov X1 SS");
        assert_close(res.covariate_terms[1].ss, 32.430_417_841_81, 1e-8, "2-cov X2 SS");
        assert_close(res.adjusted_means[0].adjusted_mean, 20.454_005_719_266_0, 1e-9, "2-cov EMM Ctrl");
        assert_close(res.adjusted_means[1].adjusted_mean, 24.000_812_214_273_5, 1e-9, "2-cov EMM Low");
        assert_close(res.adjusted_means[2].adjusted_mean, 27.765_344_509_315_1, 1e-9, "2-cov EMM High");
        let hom = res.homogeneity_of_slopes.as_ref().ok_or("missing 2-cov homogeneity")?;
        assert_close(hom.ss_interaction, 0.241_176_796_946_2, 1e-9, "2-cov Homogeneity SS");
        assert_close(hom.f_statistic, 1.143_93, 1e-4, "2-cov Homogeneity F");
        assert_close(hom.p_value, 0.432_37, 1e-4, "2-cov Homogeneity P");
        assert_eq!(hom.per_covariate_interactions.len(), 2);
        Ok(())
    }

    #[test]
    fn ancova_handles_mv_xy_sheets_and_degenerate_inputs() -> Result<(), Box<dyn std::error::Error>> {
        // MultipleVariables sheet end-to-end with Duncan post-hoc
        let mut mv = MultipleVariablesTable::new(3, 9)?;
        mv.variables[0].title = String::from("Response");
        mv.variables[1].title = String::from("Treatment");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[2].title = String::from("Baseline");
        let data = [
            (10.0, "A", 1.0),
            (12.2, "A", 2.0),
            (14.1, "A", 3.0),
            (14.0, "B", 1.0),
            (16.1, "B", 2.0),
            (18.0, "B", 3.0),
            (20.0, "C", 1.0),
            (22.2, "C", 2.0),
            (24.1, "C", 3.0),
        ];
        for (r, (y, g, x)) in data.into_iter().enumerate() {
            mv.variables[0].cells[r] = Cell::number(y);
            mv.variables[1].cells[r] = Cell::text(g);
            mv.variables[2].cells[r] = Cell::number(x);
        }
        let sheet = DataSheet::new(SheetId::new("mv-anc"), "ANCOVA MV", TableArchetype::MultipleVariables(mv))?;
        let anc_spec = AncovaSpec {
            response_column: 0,
            group_column: 1,
            covariate_columns: vec![2],
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Duncan, ..MultiplicitySpec::default() },
            ..AncovaSpec::default()
        };
        let spec = AnalysisSpec::new(SheetId::new("res-anc"), SheetId::new("mv-anc"), AnalysisKind::Ancova(anc_spec.clone()))?;
        let rs = run_ancova_analysis(&sheet, &spec, &anc_spec)?;
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-table"));
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-adjusted-means"));
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-covariate-slopes"));
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-homogeneity"));
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-posthoc"));
        assert!(rs.tabs.iter().any(|t| t.id == "ancova-residuals"));
        assert_eq!(rs.pairwise_comparisons.len(), 3);
        let sw_item = rs
            .checklist
            .iter()
            .find(|c| c.assumption == "Residual normality (Shapiro-Wilk)")
            .ok_or("missing Shapiro-Wilk residual checklist item")?;
        assert!(matches!(sw_item.status, curvebook_doc::ChecklistStatus::Pass | curvebook_doc::ChecklistStatus::Warn));
        assert!(sw_item.diagnostic_metric.as_deref().is_some_and(|m| m.starts_with("Shapiro-Wilk W = ")));

        // XY sheet end-to-end
        let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4)?;
        xy.x_title = String::from("Dose");
        xy.data_sets[0].title = String::from("Control");
        xy.data_sets[1].title = String::from("Drug");
        for (r, (x, y0, y1)) in [(1.0, 5.0, 9.1), (2.0, 7.1, 11.0), (3.0, 8.9, 13.2), (4.0, 11.2, 15.0)].into_iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x))?;
            xy.data_sets[0].cells[r][0] = Cell::number(y0);
            xy.data_sets[1].cells[r][0] = Cell::number(y1);
        }
        let xy_sheet = DataSheet::new(SheetId::new("xy-anc"), "ANCOVA XY", TableArchetype::Xy(xy))?;
        let xy_spec = AnalysisSpec::new(SheetId::new("res-xy-anc"), SheetId::new("xy-anc"), AnalysisKind::Ancova(AncovaSpec::default()))?;
        let xy_rs = run_ancova_analysis(&xy_sheet, &xy_spec, &AncovaSpec::default())?;
        assert!(xy_rs.lookup_scalar("Group F", None).is_some_and(|v| v > 100.0));
        let xy_sw = xy_rs
            .checklist
            .iter()
            .find(|c| c.assumption == "Residual normality (Shapiro-Wilk)")
            .ok_or("missing XY Shapiro-Wilk residual checklist item")?;
        assert_eq!(xy_sw.status, curvebook_doc::ChecklistStatus::Pass);
        assert!(xy_sw.diagnostic_metric.as_deref().is_some_and(|m| m.starts_with("Shapiro-Wilk W = ")));

        // Collinear constant covariate -> typed SingularMatrix error (no panic)
        let bad = AncovaObservations {
            response_name: String::from("Y"),
            group_name: String::from("G"),
            group_levels: vec![String::from("A"), String::from("B")],
            covariate_names: vec![String::from("ConstX")],
            rows: vec![(0, vec![5.0], 1.0), (0, vec![5.0], 2.0), (1, vec![5.0], 3.0), (1, vec![5.0], 4.0)],
            omitted_rows: 0,
        };
        assert!(matches!(
            fit_general_ancova(&bad, SumOfSquaresType::TypeIII, true, &MultiplicitySpec::default(), ConfidenceLevel::DEFAULT_95),
            Err(StatsError::Math(curvebook_math::MathError::SingularMatrix { .. }))
        ));

        // Non-finite input -> typed DomainError (no panic)
        let nan_obs = AncovaObservations {
            response_name: String::from("Y"),
            group_name: String::from("G"),
            group_levels: vec![String::from("A"), String::from("B")],
            covariate_names: vec![String::from("X")],
            rows: vec![(0, vec![1.0], f64::NAN), (0, vec![2.0], 2.0), (1, vec![1.0], 3.0), (1, vec![2.0], 4.0)],
            omitted_rows: 0,
        };
        assert!(fit_general_ancova(&nan_obs, SumOfSquaresType::TypeIII, true, &MultiplicitySpec::default(), ConfidenceLevel::DEFAULT_95).is_err());
        Ok(())
    }
}
