//! One-way Multivariate Analysis of Variance (MANOVA).
//!
//! Computes Hypothesis (`H`) and Error (`E`) sums-of-squares and cross-products (SSCP) matrices,
//! the eigenvalues of $E^{-1} H$ via symmetric Cholesky reduction, all four classical multivariate
//! test statistics (**Pillai's trace**, **Wilks' $\Lambda$**, **Hotelling–Lawley trace**, and
//! **Roy's largest root**) with exact and Rao/Pillai *F*-approximations, and per-response
//! univariate one-way ANOVA summaries.
//!
//! References:
//! - Hotelling, H. (1951). A generalized *T* test and measure of multivariate dispersion.
//!   *Proceedings of the Second Berkeley Symposium on Mathematical Statistics and Probability*,
//!   23-41.
//! - Lawley, D. N. (1938). A generalization of Fisher's *z* test. *Biometrika*, 30(1/2), 180-187.
//!   `doi:10.1093/biomet/30.1-2.180`.
//! - Pillai, K. C. S. (1955). Some new test criteria in multivariate analysis. *Annals of
//!   Mathematical Statistics*, 26(1), 117-121. `doi:10.1214/aoms/1177728599`.
//! - Rao, C. R. (1951). An asymptotic expansion of the distribution of Wilks' criterion. *Bulletin
//!   of the International Statistical Institute*, 33(2), 177-180.
//! - Rencher, A. C. (2002). *Methods of Multivariate Analysis* (2nd ed.). New York: Wiley.
//! - Roy, S. N. (1953). On a heuristic method of test construction and its use in multivariate
//!   analysis. *Annals of Mathematical Statistics*, 24(2), 220-238. `doi:10.1214/aoms/1177729029`.
//! - Wilks, S. S. (1932). Certain generalizations in the analysis of variance. *Biometrika*,
//!   24(3/4), 471-494. `doi:10.1093/biomet/24.3-4.471`.

use std::collections::BTreeMap;

use curvebook_doc::{AnalysisSpec, ChecklistStatus, DataSheet, ManovaSpec, ResultSheet, ResultTabKind, TableArchetype, VariableColumn, VariableRole};
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::{DenseMatrix, cholesky, sym_eigen};
use curvebook_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::common::{
    TabBuilder, append_listwise_filtered_checklist_and_footnote, checklist_item, init_result_sheet, num_cell, opt_num_cell, p_cell_at_alpha,
    push_scalar, sanitize_result_sheet_floats, text_cell,
};
use crate::error::StatsError;

/// One of the four classical MANOVA test statistics and its *F*-approximation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManovaTestResult {
    /// Classical test name (`"Pillai's trace"`, `"Wilks' Lambda"`, `"Hotelling-Lawley trace"`, or `"Roy's largest root"`).
    pub name: String,
    /// Multivariate test statistic value ($V$, $\Lambda$, $U$, or $\lambda_1$).
    pub statistic: f64,
    /// Approximate (or exact when $s \le 2$ for Wilks, or $s = 1$) *F*-ratio (`f64::NAN` if denominator df is non-positive).
    pub approx_f: f64,
    /// Numerator degrees of freedom of the *F*-approximation.
    pub num_df: f64,
    /// Denominator degrees of freedom of the *F*-approximation.
    pub den_df: f64,
    /// Upper-tail *P* value from $F(\text{num\_df}, \text{den\_df})$ (`f64::NAN` if non-estimable).
    pub p_value: f64,
    /// `true` when the *F*-distribution is mathematically exact rather than approximate (or upper bound for Roy when $s > 1$).
    pub is_exact: bool,
}

/// Per-response univariate one-way ANOVA summary emitted alongside MANOVA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnivariateAnovaRow {
    /// Response variable display name.
    pub variable_name: String,
    /// Between-group sum of squares (`H[j][j]`).
    pub ss_between: f64,
    /// Within-group residual sum of squares (`E[j][j]`).
    pub ss_within: f64,
    /// Between-group degrees of freedom (`g - 1`).
    pub df_between: f64,
    /// Within-group degrees of freedom (`N - g`).
    pub df_within: f64,
    /// Between-group mean square (`ss_between / df_between`).
    pub ms_between: f64,
    /// Within-group residual mean square (`ss_within / df_within`).
    pub ms_within: f64,
    /// Univariate one-way *F*-ratio (`ms_between / ms_within`).
    pub f_statistic: f64,
    /// Upper-tail $F(\text{df\_between}, \text{df\_within})$ *P* value.
    pub p_value: f64,
    /// Classical proportion of variance explained ($\eta^2 = \text{SS}_{\text{between}} / (\text{SS}_{\text{between}} + \text{SS}_{\text{within}})$).
    pub eta_squared: f64,
}

/// Complete one-way MANOVA computation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManovaResult {
    /// Total number of complete multivariate observations ($N$).
    pub n_observations: usize,
    /// Number of distinct grouping factor levels ($g \ge 2$).
    pub n_groups: usize,
    /// Number of numeric response variables ($p \ge 2$).
    pub n_responses: usize,
    /// Hypothesis degrees of freedom ($\nu_h = g - 1$).
    pub df_hypothesis: f64,
    /// Error degrees of freedom ($\nu_e = N - g$).
    pub df_error: f64,
    /// Ordered distinct group level labels.
    pub group_labels: Vec<String>,
    /// Per-group sample sizes $n_i$ aligned with `group_labels`.
    pub group_counts: Vec<usize>,
    /// Per-group response mean vectors (`n_groups x n_responses`).
    pub group_means: Vec<Vec<f64>>,
    /// Ordered numeric response variable labels.
    pub response_names: Vec<String>,
    /// Hypothesis (between-group) SSCP matrix $H$ (`p x p`).
    pub h_sscp: Vec<Vec<f64>>,
    /// Error (within-group) SSCP matrix $E$ (`p x p`).
    pub e_sscp: Vec<Vec<f64>>,
    /// Non-negative eigenvalues $\lambda_1 \ge \dots \ge \lambda_p \ge 0$ of $E^{-1} H$.
    pub eigenvalues: Vec<f64>,
    /// Pillai–Bartlett trace $V = \sum_{i=1}^s \lambda_i / (1 + \lambda_i)$ (Pillai 1955).
    pub pillai: ManovaTestResult,
    /// Wilks' likelihood-ratio criterion $\Lambda = \prod_{i=1}^s 1 / (1 + \lambda_i)$ with Rao's (1951) *F* (Wilks 1932).
    pub wilks: ManovaTestResult,
    /// Hotelling–Lawley trace $U = \sum_{i=1}^s \lambda_i$ with Pillai's (1955) *F* (Hotelling 1951; Lawley 1938).
    pub hotelling_lawley: ManovaTestResult,
    /// Roy's largest root $\lambda_1$ with upper-bound *F* (Roy 1953; Rencher 2002).
    pub roy: ManovaTestResult,
    /// Univariate one-way ANOVA breakdown for each response variable.
    pub univariate: Vec<UnivariateAnovaRow>,
}

/// Compute one-way MANOVA across complete observations `(group_labels[i], observations[i])`.
///
/// `observations` has `N` rows, each containing `p >= 2` finite response values matching
/// `response_names`. Groups are ordered by `group_order` when supplied, otherwise by first
/// appearance in `group_labels`.
///
/// Forms $H$ and $E$ SSCP matrices, reduces $E^{-1} H$ to the symmetric eigenproblem
/// $S = L^{-1} H L^{-\top}$ via Cholesky factorization $E = L L^\top$ (Rencher 2002 §6.1), and
/// evaluates Pillai's trace (Pillai 1955), Wilks' $\Lambda$ (Wilks 1932; Rao 1951),
/// Hotelling–Lawley trace (Hotelling 1951; Lawley 1938; Pillai 1955), and Roy's largest root
/// (Roy 1953).
///
type ManovaSscpBundle = (Vec<usize>, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<f64>>);

fn compute_manova_sscp(observations: &[Vec<f64>], grouped: &BTreeMap<&str, Vec<usize>>, ordered_labels: &[String], p: usize) -> ManovaSscpBundle {
    let n_obs = observations.len();
    let g = ordered_labels.len();

    let mut grand_mean = vec![0.0_f64; p];
    for j in 0..p {
        let mut sum = NeumaierSum::new();
        for row in observations {
            sum.add(row[j]);
        }
        grand_mean[j] = sum.total() / (n_obs as f64);
    }

    let mut group_counts = Vec::with_capacity(g);
    let mut group_means = Vec::with_capacity(g);
    let mut h_acc = vec![vec![NeumaierSum::new(); p]; p];
    let mut e_acc = vec![vec![NeumaierSum::new(); p]; p];

    for lbl in ordered_labels {
        let rows = &grouped[lbl.as_str()];
        let n_i = rows.len();
        group_counts.push(n_i);
        let mut mean_i = vec![0.0_f64; p];
        for j in 0..p {
            let mut sum = NeumaierSum::new();
            for &r in rows {
                sum.add(observations[r][j]);
            }
            mean_i[j] = sum.total() / (n_i as f64);
        }
        let n_if = n_i as f64;
        for a in 0..p {
            let da = mean_i[a] - grand_mean[a];
            for b in a..p {
                let db = mean_i[b] - grand_mean[b];
                h_acc[a][b].add(n_if * da * db);
            }
        }
        for &r in rows {
            let row = &observations[r];
            for a in 0..p {
                let da = row[a] - mean_i[a];
                for b in a..p {
                    let db = row[b] - mean_i[b];
                    e_acc[a][b].add(da * db);
                }
            }
        }
        group_means.push(mean_i);
    }

    let mut h_sscp = vec![vec![0.0_f64; p]; p];
    let mut e_sscp = vec![vec![0.0_f64; p]; p];
    for a in 0..p {
        for b in a..p {
            let hv = h_acc[a][b].total();
            let ev = e_acc[a][b].total();
            h_sscp[a][b] = hv;
            h_sscp[b][a] = hv;
            e_sscp[a][b] = ev;
            e_sscp[b][a] = ev;
        }
    }
    (group_counts, group_means, h_sscp, e_sscp)
}

fn manova_generalized_eigenvalues(h_sscp: &[Vec<f64>], e_sscp: &[Vec<f64>], p: usize) -> Result<Vec<f64>, StatsError> {
    let e_mat = DenseMatrix::from_fn(p, p, |r, c| e_sscp[r][c])?;
    let chol = cholesky(&e_mat).map_err(|_| StatsError::ZeroVariance { context: "MANOVA error SSCP matrix E" })?;
    let l_mat = chol.l();
    let diag_max = (0..p).map(|i| l_mat.get(i, i).unwrap_or(0.0)).fold(0.0_f64, f64::max);
    let diag_min = (0..p).map(|i| l_mat.get(i, i).unwrap_or(0.0)).fold(f64::INFINITY, f64::min);
    if !diag_max.is_finite() || diag_max <= 0.0 || !diag_min.is_finite() || diag_min <= 1e-12 * diag_max {
        return Err(StatsError::ZeroVariance { context: "MANOVA error SSCP matrix E" });
    }

    let mut z_cols = vec![vec![0.0_f64; p]; p];
    for col in 0..p {
        let h_col: Vec<f64> = (0..p).map(|row| h_sscp[row][col]).collect();
        z_cols[col] = forward_solve_lower(l_mat, &h_col)?;
    }
    let mut s_rows = vec![vec![0.0_f64; p]; p];
    for row in 0..p {
        let z_row: Vec<f64> = (0..p).map(|col| z_cols[col][row]).collect();
        let s_row = forward_solve_lower(l_mat, &z_row)?;
        s_rows[row][..p].copy_from_slice(&s_row[..p]);
    }
    for a in 0..p {
        for b in (a + 1)..p {
            let avg = 0.5 * (s_rows[a][b] + s_rows[b][a]);
            s_rows[a][b] = avg;
            s_rows[b][a] = avg;
        }
    }
    let s_mat = DenseMatrix::from_fn(p, p, |r, c| s_rows[r][c])?;
    let eig = sym_eigen(&s_mat)?;
    let mut eigenvalues: Vec<f64> = eig.eigenvalues.iter().map(|&v| if v.is_finite() && v > 0.0 { v } else { 0.0 }).collect();
    eigenvalues.sort_by(|a, b| b.total_cmp(a));
    Ok(eigenvalues)
}

fn compute_univariate_anovas(
    response_names: &[String],
    h_sscp: &[Vec<f64>],
    e_sscp: &[Vec<f64>],
    df_h: f64,
    df_e: f64,
) -> Result<Vec<UnivariateAnovaRow>, StatsError> {
    let mut univariate = Vec::with_capacity(response_names.len());
    for (j, name) in response_names.iter().enumerate() {
        let ss_b = h_sscp[j][j].max(0.0);
        let ss_w = e_sscp[j][j].max(0.0);
        let ms_b = ss_b / df_h;
        let ms_w = ss_w / df_e;
        let (f_stat, p_val) = if ms_w > 0.0 {
            let f = ms_b / ms_w;
            (f, f_sf(f, df_h, df_e)?)
        } else if ms_b > 0.0 {
            (f64::INFINITY, 0.0)
        } else {
            (0.0, 1.0)
        };
        let ss_tot = ss_b + ss_w;
        let eta_squared = if ss_tot > 0.0 { (ss_b / ss_tot).clamp(0.0, 1.0) } else { 0.0 };
        univariate.push(UnivariateAnovaRow {
            variable_name: name.clone(),
            ss_between: ss_b,
            ss_within: ss_w,
            df_between: df_h,
            df_within: df_e,
            ms_between: ms_b,
            ms_within: ms_w,
            f_statistic: f_stat,
            p_value: p_val,
            eta_squared,
        });
    }
    Ok(univariate)
}

/// # Errors
/// Returns [`StatsError`] on empty input, fewer than 2 response variables, fewer than 2 groups,
/// non-finite values, insufficient residual degrees of freedom ($N - g < p$), or a singular /
/// non-positive-definite error SSCP matrix $E$.
pub fn one_way_manova(
    group_labels: &[String],
    observations: &[Vec<f64>],
    response_names: &[String],
    group_order: Option<&[String]>,
) -> Result<ManovaResult, StatsError> {
    let n_obs = observations.len();
    if n_obs == 0 {
        return Err(StatsError::EmptyInput);
    }
    if group_labels.len() != n_obs {
        return Err(StatsError::DimensionMismatch { expected: n_obs, actual: group_labels.len() });
    }
    let p = response_names.len();
    if p < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "MANOVA response variables", required: 2, actual: p });
    }
    for row in observations {
        if row.len() != p {
            return Err(StatsError::DimensionMismatch { expected: p, actual: row.len() });
        }
        if let Some(&bad) = row.iter().find(|v| !v.is_finite()) {
            return Err(StatsError::InvalidParameter { name: "response", value: bad });
        }
    }

    let mut grouped: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut first_seen: Vec<String> = Vec::new();
    for (idx, lbl) in group_labels.iter().enumerate() {
        grouped
            .entry(lbl.as_str())
            .or_insert_with(|| {
                first_seen.push(lbl.clone());
                Vec::new()
            })
            .push(idx);
    }

    let ordered_labels: Vec<String> = if let Some(order) = group_order {
        let mut out = Vec::with_capacity(first_seen.len());
        for lbl in order {
            if grouped.contains_key(lbl.as_str()) && !out.contains(lbl) {
                out.push(lbl.clone());
            }
        }
        for lbl in first_seen {
            if !out.contains(&lbl) {
                out.push(lbl);
            }
        }
        out
    } else {
        first_seen
    };

    let g = ordered_labels.len();
    if g < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "MANOVA groups", required: 2, actual: g });
    }
    let df_h_usize = g - 1;
    let df_e_usize = n_obs.saturating_sub(g);
    if df_e_usize < p {
        return Err(StatsError::InsufficientSampleSize {
            context: "MANOVA residual degrees of freedom (N - g >= p)",
            required: p,
            actual: df_e_usize,
        });
    }

    let df_h = df_h_usize as f64;
    let df_e = df_e_usize as f64;
    let (group_counts, group_means, h_sscp, e_sscp) = compute_manova_sscp(observations, &grouped, &ordered_labels, p);
    let eigenvalues = manova_generalized_eigenvalues(&h_sscp, &e_sscp, p)?;

    let s_usize = p.min(df_h_usize);
    let s = s_usize as f64;
    let active_eigs = &eigenvalues[..s_usize];
    let p_f = p as f64;
    let m = 0.5 * ((p_f - df_h).abs() - 1.0);
    let n = 0.5 * (df_e - p_f - 1.0);

    let pillai = compute_pillai(active_eigs, s, m, n)?;
    let wilks = compute_wilks(active_eigs, p_f, df_h, df_e)?;
    let hotelling_lawley = compute_hotelling_lawley(active_eigs, s, m, n)?;
    let roy = compute_roy(active_eigs, p_f, df_h, df_e)?;
    let univariate = compute_univariate_anovas(response_names, &h_sscp, &e_sscp, df_h, df_e)?;

    Ok(ManovaResult {
        n_observations: n_obs,
        n_groups: g,
        n_responses: p,
        df_hypothesis: df_h,
        df_error: df_e,
        group_labels: ordered_labels,
        group_counts,
        group_means,
        response_names: response_names.to_vec(),
        h_sscp,
        e_sscp,
        eigenvalues,
        pillai,
        wilks,
        hotelling_lawley,
        roy,
        univariate,
    })
}

fn forward_solve_lower(l: &DenseMatrix, rhs: &[f64]) -> Result<Vec<f64>, StatsError> {
    let n = l.nrows();
    let mut x = vec![0.0_f64; n];
    for i in 0..n {
        let mut sum = NeumaierSum::new();
        for j in 0..i {
            sum.add(l.get(i, j).unwrap_or(0.0) * x[j]);
        }
        let diag = l.get(i, i).unwrap_or(0.0);
        if !diag.is_finite() || diag == 0.0 {
            return Err(StatsError::ZeroVariance { context: "MANOVA Cholesky factor" });
        }
        x[i] = (rhs[i] - sum.total()) / diag;
    }
    Ok(x)
}

fn compute_pillai(eigs: &[f64], s: f64, m: f64, n: f64) -> Result<ManovaTestResult, StatsError> {
    let v: f64 = eigs.iter().map(|&lam| lam / (1.0 + lam)).sum();
    let num_df = s * (2.0 * m + s + 1.0);
    let den_df = s * (2.0 * n + s + 1.0);
    let denom = s - v;
    let (approx_f, p_value) = if num_df > 0.0 && den_df > 0.0 {
        if v <= 0.0 {
            (0.0, 1.0)
        } else if denom <= 0.0 {
            (f64::INFINITY, 0.0)
        } else {
            let f = ((2.0 * n + s + 1.0) / (2.0 * m + s + 1.0)) * (v / denom);
            (f, f_sf(f, num_df, den_df)?)
        }
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(ManovaTestResult { name: String::from("Pillai's trace"), statistic: v, approx_f, num_df, den_df, p_value, is_exact: s <= 1.0 })
}

fn compute_wilks(eigs: &[f64], p: f64, df_h: f64, df_e: f64) -> Result<ManovaTestResult, StatsError> {
    let lambda: f64 = eigs.iter().map(|&lam| 1.0 / (1.0 + lam)).product();
    let num_df = p * df_h;
    let denom_t = p * p + df_h * df_h - 5.0;
    let t = if denom_t > 0.0 { ((p * p * df_h * df_h - 4.0) / denom_t).max(0.0).sqrt() } else { 1.0 };
    let w = df_e + df_h - 0.5 * (p + df_h + 1.0);
    let den_df = w * t - 0.5 * (p * df_h - 2.0);
    let (approx_f, p_value) = if num_df > 0.0 && den_df > 0.0 {
        if lambda >= 1.0 {
            (0.0, 1.0)
        } else if lambda <= 0.0 {
            (f64::INFINITY, 0.0)
        } else {
            let l_pow = lambda.powf(1.0 / t);
            let f = ((1.0 - l_pow) / l_pow) * (den_df / num_df);
            (f, f_sf(f, num_df, den_df)?)
        }
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(ManovaTestResult { name: String::from("Wilks' Lambda"), statistic: lambda, approx_f, num_df, den_df, p_value, is_exact: p.min(df_h) <= 2.0 })
}

fn compute_hotelling_lawley(eigs: &[f64], s: f64, m: f64, n: f64) -> Result<ManovaTestResult, StatsError> {
    let u: f64 = eigs.iter().copied().sum();
    let num_df = s * (2.0 * m + s + 1.0);
    let den_df = 2.0 * (s * n + 1.0);
    let (approx_f, p_value) = if num_df > 0.0 && den_df > 0.0 {
        if u <= 0.0 {
            (0.0, 1.0)
        } else {
            let f = (den_df * u) / (s * num_df);
            (f, f_sf(f, num_df, den_df)?)
        }
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(ManovaTestResult { name: String::from("Hotelling-Lawley trace"), statistic: u, approx_f, num_df, den_df, p_value, is_exact: s <= 1.0 })
}

fn compute_roy(eigs: &[f64], p: f64, df_h: f64, df_e: f64) -> Result<ManovaTestResult, StatsError> {
    let lam1 = eigs.first().copied().unwrap_or(0.0);
    let r = p.max(df_h);
    let num_df = r;
    let den_df = df_e - r + df_h;
    let (approx_f, p_value) = if num_df > 0.0 && den_df > 0.0 {
        if lam1 <= 0.0 {
            (0.0, 1.0)
        } else {
            let f = lam1 * (den_df / num_df);
            (f, f_sf(f, num_df, den_df)?)
        }
    } else {
        (f64::NAN, f64::NAN)
    };
    Ok(ManovaTestResult {
        name: String::from("Roy's largest root"),
        statistic: lam1,
        approx_f,
        num_df,
        den_df,
        p_value,
        is_exact: p.min(df_h) <= 1.0,
    })
}

fn column_is_numeric(col: &VariableColumn) -> bool {
    matches!(col.role, VariableRole::Continuous | VariableRole::Calculated)
        || (col.role != VariableRole::Label && col.cells.iter().any(|c| c.active_number().is_some()))
}

fn cell_group_token(col: &VariableColumn, row: usize) -> Option<String> {
    let cell = col.cells.get(row)?;
    if cell.excluded || cell.is_empty() {
        return None;
    }
    match &cell.value {
        curvebook_doc::CellValue::Text(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_owned())
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => {
            if (v.fract()).abs() < 1e-12 {
                Some(format!("{v:.0}"))
            } else {
                Some(format!("{v}"))
            }
        }
        _ => None,
    }
}

/// Execute one-way MANOVA against a `MultipleVariables` [`DataSheet`].
pub fn run_manova_analysis(sheet: &DataSheet, spec: &AnalysisSpec, manova_spec: &ManovaSpec) -> Result<ResultSheet, StatsError> {
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "one-way MANOVA", actual: sheet.archetype.kind() });
    };

    let n_vars = mv.variables.len();
    if n_vars < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "MANOVA columns (1 group + >= 2 responses)", required: 3, actual: n_vars });
    }

    // Resolve grouping column.
    let group_col_idx = match manova_spec.group_column {
        Some(idx) => {
            if idx >= n_vars {
                return Err(StatsError::InvalidParameter { name: "group_column", value: idx as f64 });
            }
            idx
        }
        None => mv
            .variables
            .iter()
            .position(|v| v.role == VariableRole::Categorical)
            .or_else(|| mv.variables.iter().position(|v| !column_is_numeric(v)))
            .ok_or(StatsError::IncompatibleTable { analysis: "MANOVA requires a categorical grouping column", actual: sheet.archetype.kind() })?,
    };

    // Resolve response columns (>= 2 numeric variables excluding group_col_idx).
    let mut response_cols: Vec<usize> = if !manova_spec.response_columns.is_empty() {
        manova_spec.response_columns.clone()
    } else if !spec.selected_data_sets.is_empty() {
        spec.selected_data_sets.iter().copied().filter(|&c| c != group_col_idx).collect()
    } else {
        (0..n_vars).filter(|&c| c != group_col_idx && column_is_numeric(&mv.variables[c])).collect()
    };

    // Validate indices & uniqueness.
    let mut deduped = Vec::with_capacity(response_cols.len());
    for c in response_cols.drain(..) {
        if c >= n_vars || c == group_col_idx {
            return Err(StatsError::InvalidParameter { name: "response_columns", value: c as f64 });
        }
        if !deduped.contains(&c) {
            deduped.push(c);
        }
    }
    response_cols = deduped;
    if response_cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "MANOVA response columns", required: 2, actual: response_cols.len() });
    }

    let group_col = &mv.variables[group_col_idx];
    let response_names: Vec<String> = response_cols
        .iter()
        .map(|&c| {
            let t = mv.variables[c].title.trim();
            if t.is_empty() { format!("Var {}", c + 1) } else { t.to_owned() }
        })
        .collect();

    let total_rows = mv.row_count();
    let mut group_labels = Vec::new();
    let mut observations = Vec::new();
    let mut omitted_rows = 0_usize;

    for row in 0..total_rows {
        let Some(g_lbl) = cell_group_token(group_col, row) else {
            // Check whether any response cell was populated to count as listwise omitted.
            let any_resp = response_cols.iter().any(|&c| mv.variables[c].cells.get(row).is_some_and(|cell| !cell.is_empty()));
            if any_resp {
                omitted_rows += 1;
            }
            continue;
        };
        let mut vals = Vec::with_capacity(response_cols.len());
        let mut complete = true;
        for &c in &response_cols {
            match mv.variables[c].cells.get(row).and_then(|cell| cell.active_number()) {
                Some(v) if v.is_finite() => vals.push(v),
                _ => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            group_labels.push(g_lbl);
            observations.push(vals);
        } else {
            omitted_rows += 1;
        }
    }

    let level_order: Vec<String> = group_col.ordered_levels().into_iter().map(|s| s.level).collect();
    let order_opt = (!level_order.is_empty()).then_some(level_order.as_slice());
    let res = one_way_manova(&group_labels, &observations, &response_names, order_opt)?;

    let mut out = init_result_sheet(spec)?;
    let alpha = spec.confidence_level.alpha();
    let p_fmt = spec.p_value_format;

    // Publish scalars for hooks, automation, and oracle checks.
    for (stat_label, prefix, test) in [
        ("Pillai's trace", "Pillai", &res.pillai),
        ("Wilks' Lambda", "Wilks", &res.wilks),
        ("Hotelling-Lawley trace", "Hotelling-Lawley", &res.hotelling_lawley),
        ("Roy's largest root", "Roy", &res.roy),
    ] {
        push_scalar(&mut out, stat_label, None, test.statistic);
        push_scalar(&mut out, format!("{prefix} num df"), None, test.num_df);
        if test.den_df.is_finite() && test.den_df > 0.0 && test.approx_f.is_finite() {
            push_scalar(&mut out, format!("{prefix} F"), None, test.approx_f);
            push_scalar(&mut out, format!("{prefix} den df"), None, test.den_df);
            if test.p_value.is_finite() {
                push_scalar(&mut out, format!("{prefix} P"), None, test.p_value);
            }
        }
    }

    push_scalar(&mut out, "Observations", None, res.n_observations as f64);
    push_scalar(&mut out, "Groups", None, res.n_groups as f64);
    push_scalar(&mut out, "Response variables", None, res.n_responses as f64);
    push_scalar(&mut out, "Hypothesis df", None, res.df_hypothesis);
    push_scalar(&mut out, "Error df", None, res.df_error);

    // Tab 1: Multivariate Tests table.
    let mut mv_tab = TabBuilder::new("manova-multivariate", "Multivariate tests (MANOVA)", ResultTabKind::AnovaTable).headers([
        "Statistic",
        "Approx. F",
        "Num df",
        "Den df",
        "P value",
        "Notes",
    ]);
    for test in [&res.pillai, &res.wilks, &res.hotelling_lawley, &res.roy] {
        let note = if test.name.starts_with("Roy") && !test.is_exact {
            "Upper bound on F"
        } else if test.is_exact {
            "Exact F"
        } else {
            "Approximate F"
        };
        mv_tab.push_row(
            test.name.clone(),
            [
                num_cell(test.statistic),
                opt_num_cell(test.approx_f.is_finite().then_some(test.approx_f)),
                num_cell(test.num_df),
                opt_num_cell((test.den_df.is_finite() && test.den_df > 0.0).then_some(test.den_df)),
                p_cell_at_alpha(test.p_value, p_fmt, alpha),
                text_cell(note),
            ],
        );
    }
    let mut mv_tab = mv_tab.build();
    mv_tab.footnotes.push(format!(
        "Grouping variable: {} ({} groups, N = {}, hypothesis df = {:.0}, error df = {:.0}).",
        group_col.title, res.n_groups, res.n_observations, res.df_hypothesis, res.df_error
    ));
    out.tabs.push(mv_tab);

    // Tab 2: Univariate ANOVA summaries.
    let mut uni_tab = TabBuilder::new("manova-univariate", "Univariate one-way ANOVA summary", ResultTabKind::AnovaTable).headers([
        "SS (Between)",
        "SS (Within)",
        "df (Between)",
        "df (Within)",
        "MS (Between)",
        "MS (Within)",
        "F",
        "P value",
        "Eta squared",
    ]);
    for (idx, row) in res.univariate.iter().enumerate() {
        push_scalar(&mut out, "Univariate F", Some(idx), row.f_statistic);
        push_scalar(&mut out, "Univariate P", Some(idx), row.p_value);
        uni_tab.push_row(
            row.variable_name.clone(),
            [
                num_cell(row.ss_between),
                num_cell(row.ss_within),
                num_cell(row.df_between),
                num_cell(row.df_within),
                num_cell(row.ms_between),
                num_cell(row.ms_within),
                num_cell(row.f_statistic),
                p_cell_at_alpha(row.p_value, p_fmt, alpha),
                num_cell(row.eta_squared),
            ],
        );
    }
    out.tabs.push(uni_tab.build());

    // Tab 3 & 4: Hypothesis (H) and Error (E) SSCP matrices.
    let mut h_tab = TabBuilder::new("manova-sscp-h", "Hypothesis SSCP matrix (H)", ResultTabKind::MainSummary).headers(res.response_names.clone());
    let mut e_tab = TabBuilder::new("manova-sscp-e", "Error SSCP matrix (E)", ResultTabKind::MainSummary).headers(res.response_names.clone());
    for (i, rname) in res.response_names.iter().enumerate() {
        h_tab.push_row(rname.clone(), res.h_sscp[i].iter().map(|&v| num_cell(v)));
        e_tab.push_row(rname.clone(), res.e_sscp[i].iter().map(|&v| num_cell(v)));
    }
    out.tabs.push(h_tab.build());
    out.tabs.push(e_tab.build());

    // Tab 5: Group means table.
    let mut mean_headers = vec![String::from("N")];
    mean_headers.extend(res.response_names.iter().cloned());
    let mut means_tab = TabBuilder::new("manova-group-means", "Group means", ResultTabKind::DescriptiveSummary).headers(mean_headers);
    for (gi, glbl) in res.group_labels.iter().enumerate() {
        let mut cells = Vec::with_capacity(res.n_responses + 1);
        cells.push(num_cell(res.group_counts[gi] as f64));
        cells.extend(res.group_means[gi].iter().map(|&v| num_cell(v)));
        means_tab.push_row(glbl.clone(), cells);
    }
    out.tabs.push(means_tab.build());

    // Checklist items.
    let pillai_sig = res.pillai.p_value.is_finite() && res.pillai.p_value < alpha;
    let p_str = if res.pillai.p_value < 1e-4 { format!("{:.3e}", res.pillai.p_value) } else { format!("{:.4}", res.pillai.p_value) };
    out.checklist.push(checklist_item(
        "Multivariate group effect (Pillai's trace)",
        ChecklistStatus::Info,
        Some(format!("V = {:.4}, F = {:.3}, P = {p_str}", res.pillai.statistic, res.pillai.approx_f)),
        if pillai_sig {
            "Pillai's trace rejects equality of group mean vectors across responses at the configured significance threshold; inspect univariate ANOVAs to see which responses drive separation."
        } else {
            "Pillai's trace does not reject equality of group mean vectors across responses at the configured significance threshold."
        },
    ));
    if res.n_observations < res.n_groups * res.n_responses * 2 {
        out.checklist.push(checklist_item(
            "Sample size relative to response dimensionality",
            ChecklistStatus::Warn,
            Some(format!("N = {}, g = {}, p = {}", res.n_observations, res.n_groups, res.n_responses)),
            "Small sample size relative to response dimension p reduces MANOVA power and sensitivity to covariance heterogeneity; Pillai's trace is the most robust of the four criteria.",
        ));
    }
    append_listwise_filtered_checklist_and_footnote(&mut out, "MANOVA", omitted_rows, res.n_observations);

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}
