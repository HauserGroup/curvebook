//! Multiple Imputation by Chained Equations (MICE) using Type-1 Predictive Mean Matching (PMM)
//! with Rubin's (1987) combination rules and Barnard–Rubin (1999) small-sample degrees of freedom.
//!
//! References:
//! - Rubin, D. B. (1987). *Multiple Imputation for Nonresponse in Surveys*. Wiley.
//! - Barnard, J., & Rubin, D. B. (1999). Small-sample degrees of freedom with multiple imputation.
//!   *Biometrika*, 86(4), 948-955.
//! - Rubin, D. B., & Schenker, N. (1986). Multiple imputation for interval estimation from simple
//!   random samples with ignorable nonresponse. *Journal of the American Statistical Association*,
//!   81(394), 366-374.
//! - van Buuren, S. (2018). *Flexible Imputation of Missing Data* (2nd ed.). Chapman & Hall/CRC.
//! - Marsaglia, G., & Tsang, W. W. (2000). A simple method for generating gamma variables. *ACM
//!   Transactions on Mathematical Software*, 26(3), 363-372.

use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_p;
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, sym_eigen};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::{NeumaierSum, SummaryMoments, pearson_r};

use super::types::{MultipleImputationConfig, MultipleImputationResult, RubinPooledEstimate};
use crate::common::{t_crit_two_sided, validate_finite_slice};
use crate::error::StatsError;
use crate::regression::multiple_linear_regression;

/// Combine `m` scalar parameter estimates `estimates` (`Q_1..Q_m`) and corresponding
/// within-imputation variances `variances` (`U_1..U_m`, i.e., `SE_k^2`) via Rubin's (1987)
/// combination rules with optional Barnard–Rubin (1999) small-sample df adjustment.
pub fn rubin_pool_estimates(
    term: impl Into<String>,
    estimates: &[f64],
    variances: &[f64],
    complete_data_df: Option<f64>,
    confidence: ConfidenceLevel,
) -> Result<RubinPooledEstimate, StatsError> {
    let m = estimates.len();
    if variances.len() != m {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: m, actual: variances.len() }));
    }
    if m < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Rubin pooling imputations (m)", required: 2, actual: m });
    }
    validate_finite_slice(estimates)?;
    validate_finite_slice(variances)?;
    for &u in variances {
        if u < 0.0 {
            return Err(StatsError::InvalidParameter { name: "within_variance", value: u });
        }
    }
    if let Some(vc) = complete_data_df
        && (!vc.is_finite() || vc <= 0.0)
    {
        return Err(StatsError::InvalidParameter { name: "complete_data_df", value: vc });
    }

    let m_f = m as f64;
    let mut sum_q = NeumaierSum::new();
    let mut sum_u = NeumaierSum::new();
    for (&q, &u) in estimates.iter().zip(variances.iter()) {
        sum_q.add(q);
        sum_u.add(u);
    }
    let q_bar = sum_q.total() / m_f;
    let u_bar = sum_u.total() / m_f;

    let mut sum_sq = NeumaierSum::new();
    for &q in estimates {
        let diff = q - q_bar;
        sum_sq.add(diff * diff);
    }
    let b = sum_sq.total() / (m_f - 1.0);
    let b_inflated = (1.0 + 1.0 / m_f) * b;
    let t_total = u_bar + b_inflated;

    let term = term.into();
    if t_total <= 0.0 {
        let df = complete_data_df.unwrap_or(1.0e6);
        return Ok(RubinPooledEstimate {
            term,
            m,
            estimate: q_bar,
            within_variance: u_bar,
            between_variance: 0.0,
            total_variance: 0.0,
            pooled_se: 0.0,
            t_statistic: f64::NAN,
            df,
            p_value: f64::NAN,
            ci: (f64::NAN, f64::NAN),
            riv: 0.0,
            lambda: 0.0,
            fmi: 0.0,
        });
    }

    let pooled_se = t_total.sqrt();
    let lambda = (b_inflated / t_total).clamp(0.0, 1.0);
    let one_minus_lambda = (u_bar / t_total).clamp(0.0, 1.0);
    let riv = if u_bar > 0.0 { b_inflated / u_bar } else { f64::INFINITY };

    let (df, fmi) = if b <= 0.0 || lambda <= 1.0e-15 {
        (complete_data_df.unwrap_or(1.0e6), 0.0)
    } else if u_bar <= 0.0 || one_minus_lambda <= 1.0e-12 {
        // When within-imputation variance is zero (or negligible relative to between-imputation
        // variance so 1 - lambda <= 1e-12), avoid collapsing Barnard-Rubin nu_obs to ~0 and use
        // Rubin's large-sample df nu_old = (m - 1) / lambda^2 capped at complete_data_df (Rubin 1987).
        let nu_old = (m_f - 1.0) / (lambda * lambda);
        let df_val = complete_data_df.map_or(nu_old, |vc| nu_old.min(vc));
        let fmi_val = (lambda + one_minus_lambda * (2.0 / (df_val + 3.0))).clamp(0.0, 1.0);
        (df_val, fmi_val)
    } else {
        let nu_old = (m_f - 1.0) / (lambda * lambda);
        let df_val = if let Some(vc) = complete_data_df {
            let nu_obs = ((vc + 1.0) / (vc + 3.0)) * vc * one_minus_lambda;
            (nu_old * nu_obs) / (nu_old + nu_obs)
        } else {
            nu_old
        };
        let fmi_val = (lambda + one_minus_lambda * (2.0 / (df_val + 3.0))).clamp(0.0, 1.0);
        (df_val, fmi_val)
    };

    let t_stat = q_bar / pooled_se;
    let p_val = student_t_two_sided_p(t_stat, df)?;
    let t_crit = match t_crit_two_sided(confidence, df) {
        Ok(tc) => tc,
        Err(StatsError::Math(curvebook_math::MathError::NotConverged { .. })) => f64::INFINITY,
        Err(e) => return Err(e),
    };

    Ok(RubinPooledEstimate {
        term,
        m,
        estimate: q_bar,
        within_variance: u_bar,
        between_variance: b,
        total_variance: t_total,
        pooled_se,
        t_statistic: t_stat,
        df,
        p_value: p_val,
        ci: (q_bar - t_crit * pooled_se, q_bar + t_crit * pooled_se),
        riv,
        lambda,
        fmi,
    })
}

/// Run seeded MICE with Bayesian posterior linear parameter draws and Type-1 Predictive Mean
/// Matching (PMM) across `cols`, pooling variable means and optional complete-data OLS estimates
/// with Rubin's rules.
pub fn mice_predictive_mean_matching(
    labels: &[String],
    cols: &[Vec<Option<f64>>],
    config: &MultipleImputationConfig,
) -> Result<MultipleImputationResult, StatsError> {
    let p = cols.len();
    if p == 0 {
        return Err(StatsError::EmptyInput);
    }
    if labels.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: p, actual: labels.len() }));
    }
    let n_rows = cols[0].len();
    if n_rows < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "multiple imputation rows", required: 3, actual: n_rows });
    }
    for col in cols {
        if col.len() != n_rows {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n_rows, actual: col.len() }));
        }
    }

    validate_mi_model_indices(config.response_col, &config.predictor_cols, p)?;

    let mut obs_indices = Vec::with_capacity(p);
    let mut mis_indices = Vec::with_capacity(p);
    let mut missing_counts = Vec::with_capacity(p);
    let mut total_missing = 0_usize;

    for col in cols {
        let mut obs = Vec::new();
        let mut mis = Vec::new();
        for (r, &cell) in col.iter().enumerate() {
            match cell {
                Some(v) if v.is_finite() => obs.push(r),
                _ => mis.push(r),
            }
        }
        if obs.len() < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "multiple imputation observed cells", required: 2, actual: obs.len() });
        }
        total_missing += mis.len();
        missing_counts.push(mis.len());
        obs_indices.push(obs);
        mis_indices.push(mis);
    }

    if !(2..=50).contains(&config.imputations_m) {
        return Err(StatsError::InvalidParameter { name: "imputations_m", value: config.imputations_m as f64 });
    }
    if !(1..=100).contains(&config.max_iters) {
        return Err(StatsError::InvalidParameter { name: "max_iters", value: config.max_iters as f64 });
    }
    let m = config.imputations_m;
    let max_iters = config.max_iters;
    let mut rng = DeterministicRng::from_seed(config.seed);

    let mut base_rows = vec![vec![0.0_f64; p]; n_rows];
    for (j, col) in cols.iter().enumerate() {
        for (r, &cell) in col.iter().enumerate() {
            base_rows[r][j] = match cell {
                Some(v) if v.is_finite() => v,
                _ => 0.0,
            };
        }
    }

    let mut imputed_datasets = Vec::with_capacity(m);
    if total_missing == 0 {
        for _ in 0..m {
            imputed_datasets.push(base_rows.clone());
        }
    } else {
        for _imp_idx in 0..m {
            let mut work = base_rows.clone();
            initialize_missing_cells(&mut work, &obs_indices, &mis_indices, &mut rng);
            for _iter in 0..max_iters {
                for j in 0..p {
                    if mis_indices[j].is_empty() {
                        continue;
                    }
                    if p == 1 {
                        impute_single_variable(&mut work, &obs_indices[0], &mis_indices[0], &mut rng);
                    } else {
                        impute_column_pmm(&mut work, j, &obs_indices[j], &mis_indices[j], &mut rng)?;
                    }
                }
            }
            imputed_datasets.push(work);
        }
    }

    let pooled_means = pool_imputed_means(labels, &imputed_datasets, n_rows, config.confidence)?;
    let pooled_regression = pool_imputed_regression(labels, &imputed_datasets, n_rows, config)?;

    Ok(MultipleImputationResult {
        m,
        max_iters,
        seed: config.seed,
        n_rows,
        variable_labels: labels.to_vec(),
        missing_counts,
        total_missing,
        pooled_means,
        pooled_regression,
        imputed_datasets,
    })
}

fn validate_mi_model_indices(response_col: Option<usize>, predictor_cols: &[usize], p: usize) -> Result<(), StatsError> {
    if let Some(resp) = response_col
        && resp >= p
    {
        return Err(StatsError::MissingColumn { index: resp });
    }
    for &pred in predictor_cols {
        if pred >= p {
            return Err(StatsError::MissingColumn { index: pred });
        }
    }
    Ok(())
}

fn initialize_missing_cells(rows: &mut [Vec<f64>], obs_indices: &[Vec<usize>], mis_indices: &[Vec<usize>], rng: &mut DeterministicRng) {
    for (j, (obs, mis)) in obs_indices.iter().zip(mis_indices.iter()).enumerate() {
        for &r_mis in mis {
            let pick = rng.gen_index(obs.len()).unwrap_or(0);
            rows[r_mis][j] = rows[obs[pick]][j];
        }
    }
}

fn impute_single_variable(rows: &mut [Vec<f64>], obs_rows: &[usize], mis_rows: &[usize], rng: &mut DeterministicRng) {
    let n_obs = obs_rows.len();
    if n_obs == 0 {
        return;
    }
    // Approximate Bayesian Bootstrap / intercept-only full-donor PMM (Rubin & Schenker 1986;
    // van Buuren 2018 §3.2 & §3.4): sample n_obs donors with replacement from y_obs, then draw
    // each missing cell uniformly from that bootstrap donor pool.
    let mut donor_pool = Vec::with_capacity(n_obs);
    for _ in 0..n_obs {
        let pick = rng.gen_index(n_obs).unwrap_or(0);
        donor_pool.push(rows[obs_rows[pick]][0]);
    }
    for &r_mis in mis_rows {
        let pick = rng.gen_index(n_obs).unwrap_or(0);
        rows[r_mis][0] = donor_pool[pick];
    }
}

fn impute_column_pmm(
    rows: &mut [Vec<f64>],
    target_col: usize,
    obs_rows: &[usize],
    mis_rows: &[usize],
    rng: &mut DeterministicRng,
) -> Result<(), StatsError> {
    let n_obs = obs_rows.len();
    let y_obs: Vec<f64> = obs_rows.iter().map(|&r| rows[r][target_col]).collect();
    let predictors = select_pmm_predictors(rows, obs_rows, target_col, rows[0].len());

    let x_obs = build_design_matrix(rows, obs_rows, &predictors)?;
    let (sol, cov_unscaled) = solve_linear_least_squares(&x_obs, &y_obs)?;
    let beta_hat = &sol.coefficients;
    let df_res = n_obs.saturating_sub(sol.rank).max(1);
    let g = chi2_draw(df_res, rng);
    let sigma_star = if sol.rss > 0.0 { (sol.rss / g).sqrt() } else { 0.0 };
    let beta_star = posterior_beta_draw(beta_hat, &cov_unscaled, sigma_star, rng);

    let y_hat_obs = predict_rows(&x_obs, beta_hat);
    let x_mis = build_design_matrix(rows, mis_rows, &predictors)?;
    let y_hat_mis = predict_rows(&x_mis, &beta_star);

    let donors_k = if n_obs <= 3 { n_obs } else { (n_obs / 2).clamp(2, 5) };
    let mut distances: Vec<(usize, f64)> = Vec::with_capacity(n_obs);

    for (m_pos, &r_mis) in mis_rows.iter().enumerate() {
        let target_pred = y_hat_mis[m_pos];
        distances.clear();
        for (o_pos, &obs_pred) in y_hat_obs.iter().enumerate() {
            distances.push((o_pos, (obs_pred - target_pred).abs()));
        }
        distances.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

        // Break boundary ties fairly so discrete predictors draw across the tied donor pool.
        let cutoff = distances[donors_k - 1].1 + 1.0e-12 * target_pred.abs().max(1.0);
        let pool_len = distances.iter().take_while(|(_, d)| *d <= cutoff).count().max(donors_k);
        let pick = rng.gen_index(pool_len).unwrap_or(0);
        rows[r_mis][target_col] = y_obs[distances[pick].0];
    }
    Ok(())
}

fn select_pmm_predictors(rows: &[Vec<f64>], obs_rows: &[usize], target_col: usize, n_cols: usize) -> Vec<usize> {
    let n_obs = obs_rows.len();
    let max_preds = n_obs.saturating_sub(2);
    if max_preds == 0 {
        return Vec::new();
    }
    let y_obs: Vec<f64> = obs_rows.iter().map(|&r| rows[r][target_col]).collect();
    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for k in (0..n_cols).filter(|&col_idx| col_idx != target_col) {
        let x_k: Vec<f64> = obs_rows.iter().map(|&r| rows[r].get(k).copied().unwrap_or(0.0)).collect();
        if let Ok(mx) = SummaryMoments::from_slice(&x_k)
            && mx.css > 0.0
        {
            let score = pearson_r(&x_k, &y_obs).map_or(0.0, f64::abs);
            candidates.push((k, score));
        }
    }
    if candidates.len() > max_preds {
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        candidates.truncate(max_preds);
    }
    candidates.into_iter().map(|(k, _)| k).collect()
}

fn build_design_matrix(rows: &[Vec<f64>], row_indices: &[usize], predictor_cols: &[usize]) -> Result<DenseMatrix, StatsError> {
    let nr = row_indices.len();
    let nc = 1 + predictor_cols.len();
    let mut flat = Vec::with_capacity(nr * nc);
    for &r in row_indices {
        flat.push(1.0);
        for &c in predictor_cols {
            flat.push(rows[r][c]);
        }
    }
    Ok(DenseMatrix::from_row_major(nr, nc, flat)?)
}

fn predict_rows(x: &DenseMatrix, beta: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.nrows());
    for i in 0..x.nrows() {
        let mut s = NeumaierSum::new();
        for (j, &bj) in beta.iter().enumerate().take(x.ncols()) {
            s.add(x.get(i, j).unwrap_or(0.0) * bj);
        }
        out.push(s.total());
    }
    out
}

fn posterior_beta_draw(beta_hat: &[f64], cov_unscaled: &DenseMatrix, sigma_star: f64, rng: &mut DeterministicRng) -> Vec<f64> {
    let q = beta_hat.len();
    if q == 0 || sigma_star <= 0.0 || !sigma_star.is_finite() {
        return beta_hat.to_vec();
    }
    let z: Vec<f64> = (0..q).map(|_| rng.standard_normal()).collect();
    let delta = draw_cov_vector(cov_unscaled, &z);
    beta_hat.iter().zip(delta.iter()).map(|(&b, &d)| b + sigma_star * d).collect()
}

fn draw_cov_vector(cov: &DenseMatrix, z: &[f64]) -> Vec<f64> {
    let q = z.len();
    if let Ok(chol) = cholesky(cov) {
        let l = chol.l();
        let mut out = vec![0.0_f64; q];
        for (i, slot) in out.iter_mut().enumerate() {
            let mut s = NeumaierSum::new();
            for (j, &zj) in z.iter().enumerate().take(i + 1) {
                s.add(l.get(i, j).unwrap_or(0.0) * zj);
            }
            *slot = s.total();
        }
        return out;
    }
    if let Ok(eig) = sym_eigen(cov) {
        let mut out = vec![0.0_f64; q];
        for (k, &lam) in eig.eigenvalues.iter().enumerate() {
            if lam > 0.0 && lam.is_finite() {
                let scale = lam.sqrt() * z[k];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot += eig.eigenvectors.get(i, k).unwrap_or(0.0) * scale;
                }
            }
        }
        return out;
    }
    vec![0.0_f64; q]
}

fn chi2_draw(df: usize, rng: &mut DeterministicRng) -> f64 {
    let df_safe = df.max(1);
    if df_safe <= 30 {
        let mut sum = 0.0_f64;
        for _ in 0..df_safe {
            let z = rng.standard_normal();
            sum += z * z;
        }
        return sum.max(1.0e-12);
    }
    // Marsaglia & Tsang (2000) transformation for Gamma(alpha = df / 2 >= 15.5, 1), scaled by 2.
    let alpha = 0.5 * (df_safe as f64);
    let d = alpha - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    for _ in 0..1024 {
        let x = rng.standard_normal();
        let v_root = 1.0 + c * x;
        if v_root <= 0.0 {
            continue;
        }
        let v = v_root * v_root * v_root;
        let u = rng.next_open_f64();
        let x2 = x * x;
        if u < 1.0 - 0.0331 * x2 * x2 || u.ln() < 0.5 * x2 + d * (1.0 - v + v.ln()) {
            return (2.0 * d * v).max(1.0e-12);
        }
    }
    df_safe as f64
}

fn pool_imputed_means(
    labels: &[String],
    imputed_datasets: &[Vec<Vec<f64>>],
    n_rows: usize,
    confidence: ConfidenceLevel,
) -> Result<Vec<RubinPooledEstimate>, StatsError> {
    let p = labels.len();
    let m = imputed_datasets.len();
    let complete_df = (n_rows.saturating_sub(1).max(1)) as f64;
    let denom_df = complete_df;
    let n_f = n_rows as f64;

    let mut out = Vec::with_capacity(p);
    for (j, label) in labels.iter().enumerate() {
        let mut qs = Vec::with_capacity(m);
        let mut us = Vec::with_capacity(m);
        for ds in imputed_datasets {
            let col: Vec<f64> = ds.iter().map(|row| row[j]).collect();
            let mom = SummaryMoments::from_slice(&col)?;
            let var_sample = mom.css / denom_df;
            qs.push(mom.mean);
            us.push(var_sample / n_f);
        }
        out.push(rubin_pool_estimates(label.clone(), &qs, &us, Some(complete_df), confidence)?);
    }
    Ok(out)
}

fn pool_imputed_regression(
    labels: &[String],
    imputed_datasets: &[Vec<Vec<f64>>],
    n_rows: usize,
    config: &MultipleImputationConfig,
) -> Result<Option<Vec<RubinPooledEstimate>>, StatsError> {
    let Some(resp) = config.response_col else {
        return Ok(None);
    };
    let preds: Vec<usize> = config.predictor_cols.iter().copied().filter(|&c| c != resp).collect();
    if preds.is_empty() {
        return Ok(None);
    }

    let mut term_names = vec![String::from("Intercept")];
    for &c in &preds {
        term_names.push(labels[c].clone());
    }
    let m = imputed_datasets.len();
    let n_terms = term_names.len();
    if n_rows <= n_terms {
        return Err(StatsError::InsufficientSampleSize { context: "multiple imputation regression df", required: n_terms + 1, actual: n_rows });
    }

    let mut fits = Vec::with_capacity(m);
    for ds in imputed_datasets {
        let mut x_flat = Vec::with_capacity(n_rows * n_terms);
        let mut y_vec = Vec::with_capacity(n_rows);
        for row in ds {
            x_flat.push(1.0);
            for &c in &preds {
                x_flat.push(row[c]);
            }
            y_vec.push(row[resp]);
        }
        let x_mat = DenseMatrix::from_row_major(n_rows, n_terms, x_flat)?;
        fits.push(multiple_linear_regression(&x_mat, &y_vec, &term_names, true, false, config.confidence)?);
    }

    let complete_df = fits[0].df_residual.max(1.0);
    let mut pooled = Vec::with_capacity(n_terms);
    for (t_idx, name) in term_names.iter().enumerate() {
        let any_aliased = fits.iter().any(|fit| {
            fit.aliased_columns.get(t_idx).copied().unwrap_or(false) || !fit.terms[t_idx].estimate.is_finite() || !fit.terms[t_idx].se.is_finite()
        });
        if any_aliased {
            pooled.push(RubinPooledEstimate {
                term: name.clone(),
                m,
                estimate: f64::NAN,
                within_variance: f64::NAN,
                between_variance: f64::NAN,
                total_variance: f64::NAN,
                pooled_se: f64::NAN,
                t_statistic: f64::NAN,
                df: complete_df,
                p_value: f64::NAN,
                ci: (f64::NAN, f64::NAN),
                riv: f64::NAN,
                lambda: f64::NAN,
                fmi: f64::NAN,
            });
            continue;
        }
        let mut qs = Vec::with_capacity(m);
        let mut us = Vec::with_capacity(m);
        for fit in &fits {
            let term = &fit.terms[t_idx];
            qs.push(term.estimate);
            us.push(term.se * term.se);
        }
        pooled.push(rubin_pool_estimates(name.clone(), &qs, &us, Some(complete_df), config.confidence)?);
    }
    Ok(Some(pooled))
}
