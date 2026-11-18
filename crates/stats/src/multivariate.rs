//! Multivariate statistics: Pearson *r* (Fisher-*z* CI), Spearman *rho*, Kendall *tau_b*,
//! correlation matrices with Benjamini–Hochberg multiplicity, Principal Component Analysis (SVD,
//! scree, Kaiser, Horn's parallel analysis, scores, loadings, biplot contributions), *k*-means++,
//! Hierarchical Clustering (Single, Complete, Average/UPGMA, Ward.D2 dendrogram linkage), and
//! Multiple Imputation by Chained Equations (MICE PMM) with Rubin (1987) / Barnard–Rubin (1999) pooling.
//!
//! References (D1 citation pass 2026-11-18; MICE and parallel-analysis sources are cited inline):
//! - Pearson, K. (1896). Mathematical contributions to the theory of evolution. III. Regression,
//!   heredity, and panmixia. *Philosophical Transactions of the Royal Society A*, 187, 253-318.
//!   `doi:10.1098/rsta.1896.0007`.
//! - Fisher, R. A. (1921). On the "probable error" of a coefficient of correlation deduced from a
//!   small sample. *Metron*, 1, 3-32.
//! - Spearman, C. (1904). The proof and measurement of association between two things. *American
//!   Journal of Psychology*, 15(1), 72-101. `doi:10.2307/1412159`.
//! - Kendall, M. G. (1938). A new measure of rank correlation. *Biometrika*, 30(1-2), 81-93.
//!   `doi:10.1093/biomet/30.1-2.81`.
//! - Kendall, M. G. (1945). The treatment of ties in ranking problems. *Biometrika*, 33(3), 239-251.
//!   `doi:10.1093/biomet/33.3.239`.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). New York: Springer.
//! - Lloyd, S. P. (1982). Least squares quantization in PCM. *IEEE Transactions on Information
//!   Theory*, 28(2), 129-137. `doi:10.1109/tit.1982.1056489`.
//! - Arthur, D., & Vassilvitskii, S. (2007). k-means++: The advantages of careful seeding.
//!   *Proceedings of the 18th Annual ACM-SIAM Symposium on Discrete Algorithms (SODA)*, 1027-1035.
//! - Rousseeuw, P. J. (1987). Silhouettes: A graphical aid to the interpretation and validation of
//!   cluster analysis. *Journal of Computational and Applied Mathematics*, 20, 53-65.
//!   `doi:10.1016/0377-0427(87)90125-7`.
//! - Lance, G. N., & Williams, W. T. (1967). A general theory of classificatory sorting strategies:
//!   1. Hierarchical systems. *The Computer Journal*, 9(4), 373-380. `doi:10.1093/comjnl/9.4.373`.
//! - Ward, J. H. (1963). Hierarchical grouping to optimize an objective function. *Journal of the
//!   American Statistical Association*, 58(301), 236-244. `doi:10.1080/01621459.1963.10500845`.
//! - Murtagh, F., & Legendre, P. (2014). Ward's hierarchical agglomerative clustering method: Which
//!   algorithms implement Ward's criterion? *Journal of Classification*, 31(3), 274-295.
//!   `doi:10.1007/s00357-014-9161-z`.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, DataSheet, MultivariateSpec, ResultCell, ResultSheet, ResultTabKind, TableArchetype, VariableRole,
};
pub use curvebook_doc::{ClusterDistance, ClusterLinkage};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_two_sided_isf, norm_two_sided_p, student_t_two_sided_p};
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, svd, sym_eigen};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::{NeumaierSum, SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use crate::common::{
    build_tab, checklist_item, ci_cell, extract_numeric_columns, init_result_sheet, num_cell, opt_num_cell, p_cell, push_scalar, quantile_sorted,
    rank_with_ties, resolve_selected_indices, t_crit_two_sided, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::adjust_benjamini_hochberg;
use crate::regression::multiple_linear_regression;

// ============================================================================
// Typed Outputs
// ============================================================================

/// Single pairwise correlation summary (Pearson, Spearman, or Kendall tau-b).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationPairResult {
    pub r: f64,
    pub r_squared: f64,
    pub ci: Option<(f64, f64)>,
    pub statistic: f64,
    pub df: Option<f64>,
    pub p_value: f64,
    pub n: usize,
}

/// Correlation matrix output across `p` variables with BH-FDR *q*-values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationMatrixResult {
    pub labels: Vec<String>,
    pub r_matrix: Vec<Vec<f64>>,
    pub p_matrix: Vec<Vec<f64>>,
    pub fdr_q_matrix: Vec<Vec<f64>>,
    pub n_matrix: Vec<Vec<usize>>,
    pub spearman: bool,
}

/// Complete Principal Component Analysis result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcaResult {
    pub n: usize,
    pub p: usize,
    pub standardized: bool,
    pub variable_labels: Vec<String>,
    pub eigenvalues: Vec<f64>,
    pub variance_proportion: Vec<f64>,
    pub cumulative_proportion: Vec<f64>,
    pub kaiser_components: usize,
    pub parallel_analysis_thresholds: Option<Vec<f64>>,
    pub parallel_analysis_components: Option<usize>,
    /// `p x k_comp` loadings (`V` columns).
    pub loadings: Vec<Vec<f64>>,
    /// `n x k_comp` PC scores (`X_scaled * V`).
    pub scores: Vec<Vec<f64>>,
    /// Variable percent contributions `(p x k_comp)` = `100 * loading^2`.
    pub variable_contributions_pct: Vec<Vec<f64>>,
}

/// Linkage rule for agglomerative hierarchical clustering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinkageMethod {
    Single,
    Complete,
    Average,
    WardD2,
}

/// Single merge step in a hierarchical clustering dendrogram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DendrogramMerge {
    /// Negative IDs (`-(i + 1)`) denote original observations `0..n`; non-negative denote prior merge step `0..step`.
    pub left: isize,
    pub right: isize,
    pub distance: f64,
    pub cluster_size: usize,
}

/// Complete *k*-means++ clustering result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMeansResult {
    pub k: usize,
    pub assignments: Vec<usize>,
    pub centroids: Vec<Vec<f64>>,
    pub wcss: f64,
    pub mean_silhouette: f64,
}

/// Pooled parameter estimate across `m` multiply imputed datasets using Rubin's (1987) rules
/// with Barnard–Rubin (1999) small-sample degrees-of-freedom adjustment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubinPooledEstimate {
    pub term: String,
    pub m: usize,
    pub estimate: f64,
    pub within_variance: f64,
    pub between_variance: f64,
    pub total_variance: f64,
    pub pooled_se: f64,
    pub t_statistic: f64,
    pub df: f64,
    pub p_value: f64,
    pub ci: (f64, f64),
    pub riv: f64,
    pub lambda: f64,
    pub fmi: f64,
}

/// Configuration options for seeded MICE Predictive Mean Matching multiple imputation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleImputationConfig {
    pub imputations_m: usize,
    pub max_iters: usize,
    pub seed: u64,
    pub response_col: Option<usize>,
    pub predictor_cols: Vec<usize>,
    pub confidence: ConfidenceLevel,
}

impl Default for MultipleImputationConfig {
    fn default() -> Self {
        Self { imputations_m: 5, max_iters: 10, seed: 42, response_col: None, predictor_cols: Vec::new(), confidence: ConfidenceLevel::default() }
    }
}

/// Complete Multiple Imputation (MICE PMM + Rubin pooling) result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleImputationResult {
    pub m: usize,
    pub max_iters: usize,
    pub seed: u64,
    pub n_rows: usize,
    pub variable_labels: Vec<String>,
    pub missing_counts: Vec<usize>,
    pub total_missing: usize,
    pub pooled_means: Vec<RubinPooledEstimate>,
    pub pooled_regression: Option<Vec<RubinPooledEstimate>>,
    /// Completed datasets indexed as `imputed_datasets[imp_idx][row_idx][col_idx]`.
    pub imputed_datasets: Vec<Vec<Vec<f64>>>,
}

// ============================================================================
// Correlations (Pearson, Spearman, Kendall tau-b, Correlation Matrix)
// ============================================================================

/// Pearson product-moment correlation with Fisher-*z* confidence interval and exact *t*-test *p*.
///
/// Pearson (1896) *r*; *t* = `r sqrt((n - 2) / (1 - r^2))` on `n - 2` df; CI from Fisher's (1921)
/// `z = atanh(r)` with standard error `1 / sqrt(n - 3)`.
pub fn pearson_correlation(x: &[f64], y: &[f64], confidence: ConfidenceLevel) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: n });
    }
    let r = pearson_r(x, y)?;
    let df = (n - 2) as f64;
    let (t_stat, p_val) = if (1.0 - r * r).abs() <= 1e-15 {
        (if r >= 0.0 { f64::INFINITY } else { f64::NEG_INFINITY }, 0.0)
    } else {
        let t = r * (df / (1.0 - r * r)).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };

    let ci = if n >= 4 && r.abs() < 1.0 - 1e-12 {
        let z_r = r.atanh();
        let se_z = 1.0 / ((n - 3) as f64).sqrt();
        let z_crit = norm_two_sided_isf(confidence.alpha())?;
        Some(((z_r - z_crit * se_z).tanh(), (z_r + z_crit * se_z).tanh()))
    } else {
        None
    };

    Ok(CorrelationPairResult { r, r_squared: r * r, ci, statistic: t_stat, df: Some(df), p_value: p_val, n })
}

/// Spearman rank-order correlation (`O(n log n)` with exact average tie handling).
///
/// Spearman (1904) *rho* computed as the Pearson correlation of mid-ranks, with the *t*
/// approximation on `n - 2` df for the two-sided *P* (no exact permutation *P*).
pub fn spearman_correlation(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: n });
    }
    let rx = rank_with_ties(x)?.ranks;
    let ry = rank_with_ties(y)?.ranks;
    let rho = pearson_r(&rx, &ry)?;
    let df = (n - 2) as f64;
    let (t_stat, p_val) = if (1.0 - rho * rho).abs() <= 1e-15 {
        (if rho >= 0.0 { f64::INFINITY } else { f64::NEG_INFINITY }, 0.0)
    } else {
        let t = rho * (df / (1.0 - rho * rho)).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };
    Ok(CorrelationPairResult { r: rho, r_squared: rho * rho, ci: None, statistic: t_stat, df: Some(df), p_value: p_val, n })
}

/// Kendall's *tau_b* rank correlation with tie-adjusted asymptotic normal *p*-value.
///
/// Kendall (1938) *tau*, with the *tau_b* tie adjustment of Kendall (1945).
pub fn kendall_tau_b(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
    }
    validate_finite_slice(x)?;
    validate_finite_slice(y)?;
    let n = x.len();
    if n < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "Pearson correlation", required: 3, actual: n });
    }
    let mut conc = 0.0_f64;
    let mut disc = 0.0_f64;
    let mut ties_x = 0.0_f64;
    let mut ties_y = 0.0_f64;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = x[i] - x[j];
            let dy = y[i] - y[j];
            if dx == 0.0 && dy == 0.0 {
                ties_x += 1.0;
                ties_y += 1.0;
            } else if dx == 0.0 {
                ties_x += 1.0;
            } else if dy == 0.0 {
                ties_y += 1.0;
            } else if dx * dy > 0.0 {
                conc += 1.0;
            } else {
                disc += 1.0;
            }
        }
    }
    let nf = n as f64;
    let n0 = 0.5 * nf * (nf - 1.0);
    let denom = ((n0 - ties_x) * (n0 - ties_y)).sqrt();
    let tau = if denom > 0.0 { ((conc - disc) / denom).clamp(-1.0, 1.0) } else { 0.0 };

    // Tie-corrected variance for S = C - D (Kendall 1970).
    let rx = rank_with_ties(x)?;
    let ry = rank_with_ties(y)?;
    let sum_t = |counts: &[usize]| -> (f64, f64, f64) {
        let mut v = 0.0;
        let mut s2 = 0.0;
        let mut s1 = 0.0;
        for &c in counts {
            let cf = c as f64;
            v += cf * (cf - 1.0) * (2.0 * cf + 5.0);
            s2 += cf * (cf - 1.0) * (cf - 2.0);
            s1 += cf * (cf - 1.0);
        }
        (v, s2, s1)
    };
    let (vt, t2, t1) = sum_t(&rx.tie_counts);
    let (vu, u2, u1) = sum_t(&ry.tie_counts);
    let v0 = nf * (nf - 1.0) * (2.0 * nf + 5.0);
    let term1 = (v0 - vt - vu) / 18.0;
    let term2 = (t2 * u2) / (9.0 * nf * (nf - 1.0) * (nf - 2.0));
    let term3 = (t1 * u1) / (2.0 * nf * (nf - 1.0));
    let var_s = (term1 + term2 + term3).max(1e-12);
    let z = (conc - disc) / var_s.sqrt();
    let p_val = norm_two_sided_p(z);

    Ok(CorrelationPairResult { r: tau, r_squared: tau * tau, ci: None, statistic: z, df: None, p_value: p_val, n })
}

/// Pairwise correlation matrix across columns (using pairwise complete finite observations)
/// with Benjamini–Hochberg FDR *q*-values across the upper triangle.
pub fn correlation_matrix(columns: &[(&str, &[Option<f64>])], spearman: bool) -> Result<CorrelationMatrixResult, StatsError> {
    let p = columns.len();
    if p < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "correlation matrix variables", required: 2, actual: p });
    }
    let mut r_mat = vec![vec![1.0_f64; p]; p];
    let mut p_mat = vec![vec![0.0_f64; p]; p];
    let mut q_mat = vec![vec![0.0_f64; p]; p];
    let mut n_mat = vec![vec![0_usize; p]; p];

    let mut upper_pairs = Vec::new();
    let mut raw_p = Vec::new();

    for i in 0..p {
        n_mat[i][i] = columns[i].1.iter().filter(|v| v.is_some_and(f64::is_finite)).count();
        for j in (i + 1)..p {
            let mut xs = Vec::new();
            let mut ys = Vec::new();
            for (&a, &b) in columns[i].1.iter().zip(columns[j].1.iter()) {
                if let (Some(av), Some(bv)) = (a, b)
                    && av.is_finite()
                    && bv.is_finite()
                {
                    xs.push(av);
                    ys.push(bv);
                }
            }
            n_mat[i][j] = xs.len();
            n_mat[j][i] = xs.len();
            let res = if xs.len() >= 3 {
                if spearman { spearman_correlation(&xs, &ys) } else { pearson_correlation(&xs, &ys, ConfidenceLevel::DEFAULT_95) }
            } else {
                Err(StatsError::InsufficientSampleSize { context: "pairwise correlation", required: 3, actual: xs.len() })
            };
            let (r_val, p_val) = match res {
                Ok(c) => (c.r, c.p_value),
                Err(_) => (0.0, 1.0),
            };
            r_mat[i][j] = r_val;
            r_mat[j][i] = r_val;
            p_mat[i][j] = p_val;
            p_mat[j][i] = p_val;
            upper_pairs.push((i, j));
            raw_p.push(p_val);
        }
    }

    let q_vals = adjust_benjamini_hochberg(&raw_p);
    for (&(i, j), q) in upper_pairs.iter().zip(q_vals) {
        q_mat[i][j] = q;
        q_mat[j][i] = q;
    }

    Ok(CorrelationMatrixResult {
        labels: columns.iter().map(|(l, _)| (*l).to_owned()).collect(),
        r_matrix: r_mat,
        p_matrix: p_mat,
        fdr_q_matrix: q_mat,
        n_matrix: n_mat,
        spearman,
    })
}

// ============================================================================
// Principal Component Analysis (SVD + Kaiser + Horn Parallel Analysis)
// ============================================================================

/// Principal Component Analysis via SVD of centered (and optionally standardized) `n x p` matrix
/// (Jolliffe 2002).
///
/// Guarantees `sum_k eigenvalue_k == trace(Cov)` (or `trace(Corr) = p` when `standardize = true`).
pub fn pca_analysis(
    data: &DenseMatrix,
    variable_labels: &[String],
    standardize: bool,
    parallel_analysis_seed: Option<u64>,
) -> Result<PcaResult, StatsError> {
    let n = data.nrows();
    let p = data.ncols();
    if n < 3 || p < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "PCA", required: 3, actual: n });
    }
    if variable_labels.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: p, actual: variable_labels.len() }));
    }

    let mut col_sds = Vec::with_capacity(p);
    let mut scaled_cols = Vec::with_capacity(p);
    for c in 0..p {
        let mut col = Vec::with_capacity(n);
        for r in 0..n {
            let v = data.get(r, c).unwrap_or(f64::NAN);
            if !v.is_finite() {
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "pca_x", value: v }));
            }
            col.push(v);
        }
        let m = SummaryMoments::from_slice(&col)?;
        let sd = m.sample_sd().unwrap_or(0.0);
        if standardize && (!sd.is_finite() || sd <= 0.0) {
            return Err(StatsError::ZeroVariance { context: "standardized PCA constant column" });
        }
        col_sds.push(sd);
        let denom = if standardize { sd } else { 1.0 };
        scaled_cols.push(col.into_iter().map(|v| (v - m.mean) / denom).collect::<Vec<_>>());
    }
    if !standardize && col_sds.iter().all(|&sd| sd <= 0.0) {
        return Err(StatsError::ZeroVariance { context: "PCA covariance matrix" });
    }

    // Build X_norm = X_centered / sqrt(n - 1) so singular_values^2 == eigenvalues of Cov/Corr.
    let scale_n = ((n - 1) as f64).sqrt();
    let mut x_scaled_data = vec![0.0_f64; n * p];
    let mut x_norm_data = vec![0.0_f64; n * p];
    for r in 0..n {
        for c in 0..p {
            let v = scaled_cols[c][r];
            x_scaled_data[r * p + c] = v;
            x_norm_data[r * p + c] = v / scale_n;
        }
    }
    let x_scaled = DenseMatrix::from_row_major(n, p, x_scaled_data)?;
    let x_norm = DenseMatrix::from_row_major(n, p, x_norm_data)?;
    let dec = svd(&x_norm)?;

    let k_comp = n.min(p);
    let mut eigenvalues = vec![0.0_f64; k_comp];
    for (i, &s) in dec.singular_values.iter().take(k_comp).enumerate() {
        eigenvalues[i] = s * s;
    }
    let total_var: f64 = eigenvalues.iter().sum::<f64>().max(1e-30);
    let variance_proportion: Vec<f64> = eigenvalues.iter().map(|&ev| ev / total_var).collect();
    let mut cumulative_proportion = Vec::with_capacity(k_comp);
    let mut run = 0.0_f64;
    for &vp in &variance_proportion {
        run += vp;
        cumulative_proportion.push(run.min(1.0));
    }

    let kaiser_thresh = if standardize { 1.0 } else { total_var / (p as f64) };
    let kaiser_components = eigenvalues.iter().filter(|&&ev| ev > kaiser_thresh).count();

    // Loadings V (p x k_comp) with deterministic sign convention (Jolliffe 2002): orient each
    // component j so its largest-magnitude loading (breaking near-ties within relative 1e-12 by
    // smallest variable index) is non-negative, then compute Scores = X_scaled * V.
    let mut loadings = vec![vec![0.0_f64; k_comp]; p];
    let mut contributions = vec![vec![0.0_f64; k_comp]; p];
    for j in 0..k_comp {
        let mut max_abs = 0.0_f64;
        for i in 0..p {
            max_abs = max_abs.max(dec.v.get(i, j).unwrap_or(0.0).abs());
        }
        let mut sign = 1.0_f64;
        if max_abs > 0.0 {
            let tie_thresh = max_abs * (1.0 - 1e-12);
            for i in 0..p {
                let vij = dec.v.get(i, j).unwrap_or(0.0);
                if vij.abs() >= tie_thresh {
                    if vij < 0.0 {
                        sign = -1.0;
                    }
                    break;
                }
            }
        }
        for i in 0..p {
            let v_ij = sign * dec.v.get(i, j).unwrap_or(0.0);
            loadings[i][j] = v_ij;
            contributions[i][j] = 100.0 * v_ij * v_ij;
        }
    }
    let mut scores = vec![vec![0.0_f64; k_comp]; n];
    for (r, row_scores) in scores.iter_mut().enumerate() {
        for (j, slot) in row_scores.iter_mut().enumerate() {
            let mut acc = 0.0_f64;
            for (c, load_row) in loadings.iter().enumerate() {
                acc += x_scaled.get(r, c).unwrap_or(0.0) * load_row[j];
            }
            *slot = acc;
        }
    }

    let (pa_thresholds, pa_count) = if let Some(seed) = parallel_analysis_seed {
        let thresh = horn_parallel_analysis(n, p, k_comp, standardize, &col_sds, seed)?;
        let cnt = eigenvalues.iter().zip(thresh.iter()).take_while(|(ev, th)| **ev > **th).count();
        (Some(thresh), Some(cnt))
    } else {
        (None, None)
    };

    Ok(PcaResult {
        n,
        p,
        standardized: standardize,
        variable_labels: variable_labels.to_vec(),
        eigenvalues,
        variance_proportion,
        cumulative_proportion,
        kaiser_components,
        parallel_analysis_thresholds: pa_thresholds,
        parallel_analysis_components: pa_count,
        loadings,
        scores,
        variable_contributions_pct: contributions,
    })
}

/// Horn (1965, *Psychometrika* 30:179-185) parallel analysis with Glorfeld (1995) 95th-percentile
/// threshold: simulate independent Gaussian columns matched to the empirical variance scale
/// (standardized to unit variance when `standardize = true`; centered and scaled by each observed
/// variable's sample SD `col_sds[c]` when `standardize = false`).
fn horn_parallel_analysis(n: usize, p: usize, k_comp: usize, standardize: bool, col_sds: &[f64], seed: u64) -> Result<Vec<f64>, StatsError> {
    let n_iter = 40_usize;
    let mut rng = DeterministicRng::from_seed(seed);
    let mut sim_eigs = vec![Vec::with_capacity(n_iter); k_comp];
    let scale_n = ((n - 1) as f64).sqrt();

    for _ in 0..n_iter {
        let mut mat_data = vec![0.0_f64; n * p];
        for c in 0..p {
            let mut col: Vec<f64> = (0..n).map(|_| rng.standard_normal()).collect();
            if let Ok(m) = SummaryMoments::from_slice(&col) {
                if standardize {
                    let sd = m.sample_sd().unwrap_or(1.0).max(1e-12);
                    for v in &mut col {
                        *v = (*v - m.mean) / sd;
                    }
                } else {
                    let target_sd = col_sds.get(c).copied().unwrap_or(0.0).max(0.0);
                    for v in &mut col {
                        *v = (*v - m.mean) * target_sd;
                    }
                }
            }
            for (r, &val) in col.iter().enumerate() {
                mat_data[r * p + c] = val / scale_n;
            }
        }
        let m = DenseMatrix::from_row_major(n, p, mat_data)?;
        let dec = svd(&m)?;
        for (j, &s) in dec.singular_values.iter().take(k_comp).enumerate() {
            sim_eigs[j].push(s * s);
        }
    }
    let fallback = if standardize { 1.0 } else { 0.0 };
    let mut out = Vec::with_capacity(k_comp);
    for mut v in sim_eigs {
        v.sort_by(f64::total_cmp);
        out.push(quantile_sorted(&v, 0.95).unwrap_or(fallback));
    }
    Ok(out)
}

// ============================================================================
// k-Means++ & Hierarchical Agglomerative Clustering
// ============================================================================

/// *k*-means++ clustering (`DeterministicRng::from_seed`) returning cluster assignments,
/// centroids, total within-cluster sum of squares (WCSS), and mean silhouette score.
///
/// `D^2` seeding of Arthur & Vassilvitskii (2007), Lloyd (1982) assignment/update iterations, and
/// the mean silhouette of Rousseeuw (1987).
pub fn kmeans_pp(rows: &[Vec<f64>], k: usize, seed: u64) -> Result<KMeansResult, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    for r in rows {
        if r.len() != d {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: d, actual: r.len() }));
        }
        validate_finite_slice(r)?;
    }

    let sq_dist = |a: &[f64], b: &[f64]| -> f64 { a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum() };

    let mut rng = DeterministicRng::from_seed(seed);
    let mut centroids = Vec::with_capacity(k);
    centroids.push(rows[rng.gen_index(n).unwrap_or(0)].clone());

    let mut min_d2 = vec![f64::INFINITY; n];
    for _ in 1..k {
        let last = centroids.last().map_or(&rows[0], |c| c);
        let mut total_d2 = 0.0_f64;
        for (i, r) in rows.iter().enumerate() {
            let d2 = sq_dist(r, last);
            if d2 < min_d2[i] {
                min_d2[i] = d2;
            }
            total_d2 += min_d2[i];
        }
        let chosen = if total_d2 <= 0.0 {
            rng.gen_index(n).unwrap_or(0)
        } else {
            let target = rng.next_open_f64() * total_d2;
            let mut acc = 0.0_f64;
            let mut pick = n - 1;
            for (i, &w) in min_d2.iter().enumerate() {
                acc += w;
                if acc >= target {
                    pick = i;
                    break;
                }
            }
            pick
        };
        centroids.push(rows[chosen].clone());
    }

    let mut assignments = vec![0_usize; n];
    for _iter in 0..100 {
        let mut changed = false;
        for (i, r) in rows.iter().enumerate() {
            let mut best_c = 0;
            let mut best_d = f64::INFINITY;
            for (c_idx, cent) in centroids.iter().enumerate() {
                let d2 = sq_dist(r, cent);
                if d2 < best_d {
                    best_d = d2;
                    best_c = c_idx;
                }
            }
            if assignments[i] != best_c {
                assignments[i] = best_c;
                changed = true;
            }
        }
        let mut sums = vec![vec![0.0_f64; d]; k];
        let mut counts = vec![0_usize; k];
        for (r, &c_idx) in rows.iter().zip(assignments.iter()) {
            counts[c_idx] += 1;
            for j in 0..d {
                sums[c_idx][j] += r[j];
            }
        }
        for c_idx in 0..k {
            if counts[c_idx] > 0 {
                let cf = counts[c_idx] as f64;
                for j in 0..d {
                    centroids[c_idx][j] = sums[c_idx][j] / cf;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut wcss = 0.0_f64;
    for (r, &c_idx) in rows.iter().zip(assignments.iter()) {
        wcss += sq_dist(r, &centroids[c_idx]);
    }
    let mean_silhouette = compute_mean_silhouette(rows, &assignments, k);

    Ok(KMeansResult { k, assignments, centroids, wcss, mean_silhouette })
}

/// Pairwise dissimilarity between two equal-length finite feature vectors under [`ClusterDistance`].
pub fn pairwise_distance_value(a: &[f64], b: &[f64], metric: ClusterDistance) -> Result<f64, StatsError> {
    if a.len() != b.len() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: a.len(), actual: b.len() }));
    }
    if a.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    validate_finite_slice(a)?;
    validate_finite_slice(b)?;
    Ok(pairwise_distance_unchecked(a, b, metric))
}

#[inline]
fn pairwise_distance_unchecked(a: &[f64], b: &[f64], metric: ClusterDistance) -> f64 {
    match metric {
        ClusterDistance::Euclidean => a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum::<f64>().sqrt(),
        ClusterDistance::Manhattan => a.iter().zip(b.iter()).map(|(x, y)| (x - y).abs()).sum(),
        ClusterDistance::Pearson => {
            let d = a.len() as f64;
            let ma = a.iter().sum::<f64>() / d;
            let mb = b.iter().sum::<f64>() / d;
            let mut sxx = 0.0_f64;
            let mut syy = 0.0_f64;
            let mut sxy = 0.0_f64;
            for (x, y) in a.iter().zip(b.iter()) {
                let dx = *x - ma;
                let dy = *y - mb;
                sxx += dx * dx;
                syy += dy * dy;
                sxy += dx * dy;
            }
            let denom = (sxx * syy).sqrt();
            if denom <= 1e-15 {
                if a == b { 0.0 } else { 1.0 }
            } else {
                let r = (sxy / denom).clamp(-1.0, 1.0);
                (1.0 - r).clamp(0.0, 2.0)
            }
        }
        ClusterDistance::Cosine => {
            let mut na2 = 0.0_f64;
            let mut nb2 = 0.0_f64;
            let mut dot = 0.0_f64;
            for (x, y) in a.iter().zip(b.iter()) {
                na2 += x * x;
                nb2 += y * y;
                dot += x * y;
            }
            let denom = (na2 * nb2).sqrt();
            if denom <= 1e-15 {
                if a == b || (na2 <= 1e-30 && nb2 <= 1e-30) { 0.0 } else { 1.0 }
            } else {
                let cos = (dot / denom).clamp(-1.0, 1.0);
                (1.0 - cos).clamp(0.0, 2.0)
            }
        }
    }
}

/// Maximum observation count accepted by hierarchical clustering and pairwise distance matrices.
pub const MAX_CLUSTERING_N: usize = 2_000;
const MAX_CLUSTER_MATRIX_TAB_N: usize = 500;
const MAX_CLUSTER_K_SWEEP: usize = 50;

/// Compute the symmetric `n x n` pairwise dissimilarity matrix for `rows` under `metric`.
pub fn pairwise_distance_matrix(rows: &[Vec<f64>], metric: ClusterDistance) -> Result<Vec<Vec<f64>>, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "pairwise distance matrix", required: 2, actual: n });
    }
    if n > MAX_CLUSTERING_N {
        return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
    }
    for r in rows {
        if r.len() != d {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: d, actual: r.len() }));
        }
        validate_finite_slice(r)?;
    }
    let mut mat = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let val = pairwise_distance_unchecked(&rows[i], &rows[j], metric);
            mat[i][j] = val;
            mat[j][i] = val;
        }
    }
    Ok(mat)
}

fn transform_row_for_centroid(row: &[f64], metric: ClusterDistance) -> Vec<f64> {
    let d = row.len();
    match metric {
        ClusterDistance::Euclidean | ClusterDistance::Manhattan => row.to_vec(),
        ClusterDistance::Cosine => {
            let norm = row.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm > 1e-15 { row.iter().map(|x| x / norm).collect() } else { vec![0.0; d] }
        }
        ClusterDistance::Pearson => {
            let mean = row.iter().sum::<f64>() / (d as f64);
            let centered: Vec<f64> = row.iter().map(|x| x - mean).collect();
            let norm = centered.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm > 1e-15 { centered.into_iter().map(|x| x / norm).collect() } else { vec![0.0; d] }
        }
    }
}

/// *k*-means++ clustering parameterized by [`ClusterDistance`] (`Euclidean`, `Manhattan`, `Pearson`, or `Cosine`).
pub fn kmeans_pp_with_distance(rows: &[Vec<f64>], k: usize, distance: ClusterDistance, seed: u64) -> Result<KMeansResult, StatsError> {
    if distance == ClusterDistance::Euclidean {
        return kmeans_pp(rows, k, seed);
    }
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    let dist_mat = pairwise_distance_matrix(rows, distance)?;
    kmeans_pp_from_dist(rows, k, distance, &dist_mat, seed)
}

fn kmeans_pp_from_dist(rows: &[Vec<f64>], k: usize, distance: ClusterDistance, dist_mat: &[Vec<f64>], seed: u64) -> Result<KMeansResult, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    if dist_mat.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: dist_mat.len() }));
    }
    let sq_dist = |a: &[f64], b: &[f64]| -> f64 {
        let v = pairwise_distance_unchecked(a, b, distance);
        v * v
    };

    let mut rng = DeterministicRng::from_seed(seed);
    let mut centroids = Vec::with_capacity(k);
    centroids.push(rows[rng.gen_index(n).unwrap_or(0)].clone());

    let mut min_d2 = vec![f64::INFINITY; n];
    for _ in 1..k {
        let last = centroids.last().map_or(&rows[0], |c| c);
        let mut total_d2 = 0.0_f64;
        for (i, r) in rows.iter().enumerate() {
            let d2 = sq_dist(r, last);
            if d2 < min_d2[i] {
                min_d2[i] = d2;
            }
            total_d2 += min_d2[i];
        }
        let chosen = if total_d2 <= 0.0 {
            rng.gen_index(n).unwrap_or(0)
        } else {
            let target = rng.next_open_f64() * total_d2;
            let mut acc = 0.0_f64;
            let mut pick = n - 1;
            for (i, &w) in min_d2.iter().enumerate() {
                acc += w;
                if acc >= target {
                    pick = i;
                    break;
                }
            }
            pick
        };
        centroids.push(rows[chosen].clone());
    }

    let mut assignments = vec![0_usize; n];
    for _iter in 0..100 {
        let mut changed = false;
        for (i, r) in rows.iter().enumerate() {
            let mut best_c = 0;
            let mut best_d = f64::INFINITY;
            for (c_idx, cent) in centroids.iter().enumerate() {
                let dv = pairwise_distance_unchecked(r, cent, distance);
                if dv < best_d {
                    best_d = dv;
                    best_c = c_idx;
                }
            }
            if assignments[i] != best_c {
                assignments[i] = best_c;
                changed = true;
            }
        }
        for (c_idx, cent) in centroids.iter_mut().enumerate() {
            let members: Vec<&Vec<f64>> = rows.iter().zip(assignments.iter()).filter(|(_, a)| **a == c_idx).map(|(r, _)| r).collect();
            if members.is_empty() {
                continue;
            }
            match distance {
                ClusterDistance::Manhattan => {
                    for (j, slot) in cent.iter_mut().enumerate() {
                        let mut col: Vec<f64> = members.iter().map(|r| r[j]).collect();
                        col.sort_by(f64::total_cmp);
                        *slot = quantile_sorted(&col, 0.5).unwrap_or(0.0);
                    }
                }
                ClusterDistance::Cosine | ClusterDistance::Pearson | ClusterDistance::Euclidean => {
                    let mut sum_vec = vec![0.0_f64; d];
                    for r in &members {
                        let t = transform_row_for_centroid(r, distance);
                        for j in 0..d {
                            sum_vec[j] += t[j];
                        }
                    }
                    let count_f = members.len() as f64;
                    for slot in &mut sum_vec {
                        *slot /= count_f;
                    }
                    *cent = sum_vec;
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut wcss = 0.0_f64;
    for (r, &c_idx) in rows.iter().zip(assignments.iter()) {
        wcss += if distance == ClusterDistance::Manhattan {
            pairwise_distance_unchecked(r, &centroids[c_idx], distance)
        } else {
            sq_dist(r, &centroids[c_idx])
        };
    }
    let mean_silhouette = compute_mean_silhouette_from_dist(dist_mat, &assignments, k);
    Ok(KMeansResult { k, assignments, centroids, wcss, mean_silhouette })
}

fn compute_mean_silhouette(rows: &[Vec<f64>], assignments: &[usize], k: usize) -> f64 {
    let Ok(dist_mat) = pairwise_distance_matrix(rows, ClusterDistance::Euclidean) else {
        return 0.0;
    };
    compute_mean_silhouette_from_dist(&dist_mat, assignments, k)
}

/// Compute mean silhouette width in `[-1, 1]` directly from a precomputed `n x n` distance matrix.
#[must_use]
pub fn compute_mean_silhouette_from_dist(dist_mat: &[Vec<f64>], assignments: &[usize], k: usize) -> f64 {
    let n = dist_mat.len();
    if k < 2 || n <= k || assignments.len() != n {
        return 0.0;
    }
    let mut sum_s = 0.0_f64;
    for i in 0..n {
        let ci = assignments[i];
        let mut intra_sum = 0.0_f64;
        let mut intra_cnt = 0_usize;
        let mut cluster_sums = vec![0.0_f64; k];
        let mut cluster_cnts = vec![0_usize; k];
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = dist_mat[i][j];
            let cj = assignments[j];
            if cj == ci {
                intra_sum += d;
                intra_cnt += 1;
            } else if cj < k {
                cluster_sums[cj] += d;
                cluster_cnts[cj] += 1;
            }
        }
        if intra_cnt == 0 {
            continue;
        }
        let a_i = intra_sum / (intra_cnt as f64);
        let b_i =
            (0..k).filter(|&c| c != ci && cluster_cnts[c] > 0).map(|c| cluster_sums[c] / (cluster_cnts[c] as f64)).fold(f64::INFINITY, f64::min);
        if b_i.is_finite() {
            let denom = a_i.max(b_i);
            if denom > 0.0 {
                sum_s += (b_i - a_i) / denom;
            }
        }
    }
    sum_s / (n as f64)
}

#[inline]
const fn map_legacy_linkage(linkage: LinkageMethod) -> ClusterLinkage {
    match linkage {
        LinkageMethod::Single => ClusterLinkage::Single,
        LinkageMethod::Complete => ClusterLinkage::Complete,
        LinkageMethod::Average => ClusterLinkage::Upgma,
        LinkageMethod::WardD2 => ClusterLinkage::Ward,
    }
}

/// Agglomerative hierarchical clustering supporting Euclidean Single, Complete, Average (UPGMA),
/// and Ward.D2 linkage via Lance–Williams dissimilarity updates.
///
/// Lance & Williams (1967) recurrence; Ward (1963) minimum-variance criterion in the Ward.D2 form
/// of Murtagh & Legendre (2014).
pub fn hierarchical_clustering(rows: &[Vec<f64>], linkage: LinkageMethod) -> Result<Vec<DendrogramMerge>, StatsError> {
    hierarchical_clustering_with_metrics(rows, ClusterDistance::Euclidean, map_legacy_linkage(linkage))
}

#[inline]
fn lance_williams_distance(linkage: ClusterLinkage, dik: f64, djk: f64, dij: f64, ni: f64, nj: f64, nk: f64) -> f64 {
    let nij = ni + nj;
    match linkage {
        ClusterLinkage::Single => dik.min(djk),
        ClusterLinkage::Complete => dik.max(djk),
        ClusterLinkage::Upgma => (ni * dik + nj * djk) / nij,
        ClusterLinkage::Wpgma => 0.5 * (dik + djk),
        ClusterLinkage::Centroid => ((ni * dik * dik + nj * djk * djk) / nij - (ni * nj * dij * dij) / (nij * nij)).max(0.0).sqrt(),
        ClusterLinkage::Median => (0.5 * dik * dik + 0.5 * djk * djk - 0.25 * dij * dij).max(0.0).sqrt(),
        ClusterLinkage::Ward => (((ni + nk) * dik * dik + (nj + nk) * djk * djk - nk * dij * dij) / (nij + nk)).max(0.0).sqrt(),
    }
}

/// O(n^2) nearest-neighbor chain agglomerative clustering (Murtagh 1983) for reducible linkages.
fn nn_chain_linkage(dist_mat: &[Vec<f64>], linkage: ClusterLinkage) -> Vec<DendrogramMerge> {
    let n = dist_mat.len();
    let mut active = vec![true; n];
    let mut dmat = vec![0.0_f64; n * n];
    for (i, row) in dist_mat.iter().enumerate() {
        dmat[i * n..(i + 1) * n].copy_from_slice(row);
    }
    let mut ids: Vec<isize> = (0..n).map(|i| -((i as isize) + 1)).collect();
    let mut sizes = vec![1_usize; n];
    let mut raw_merges: Vec<DendrogramMerge> = Vec::with_capacity(n - 1);
    let mut chain: Vec<usize> = Vec::with_capacity(n);

    while raw_merges.len() < n - 1 {
        if chain.is_empty() {
            if let Some(first) = active.iter().position(|&a| a) {
                chain.push(first);
            } else {
                break;
            }
        }
        let curr = chain[chain.len() - 1];
        let prev = if chain.len() >= 2 { Some(chain[chain.len() - 2]) } else { None };
        let row_off = curr * n;
        let mut best_j = usize::MAX;
        let mut best_d = f64::INFINITY;
        for (j, &is_act) in active.iter().enumerate() {
            if !is_act || j == curr {
                continue;
            }
            let d = dmat[row_off + j];
            if d < best_d || (d == best_d && j < best_j) {
                best_d = d;
                best_j = j;
            }
        }
        // Break ties in favor of the chain predecessor to avoid zero-distance cycles.
        if let Some(p) = prev
            && active[p]
            && dmat[row_off + p] <= best_d
        {
            best_j = p;
            best_d = dmat[row_off + p];
        }
        if best_j == usize::MAX {
            break;
        }

        if Some(best_j) == prev {
            chain.pop();
            chain.pop();
            let keep = curr.min(best_j);
            let drop_slot = curr.max(best_j);
            let step = raw_merges.len();
            let ni = sizes[keep] as f64;
            let nj = sizes[drop_slot] as f64;
            let new_size = sizes[keep] + sizes[drop_slot];
            raw_merges.push(DendrogramMerge { left: ids[keep], right: ids[drop_slot], distance: best_d, cluster_size: new_size });
            for (k, &is_act) in active.iter().enumerate() {
                if !is_act || k == keep || k == drop_slot {
                    continue;
                }
                let updated = lance_williams_distance(linkage, dmat[keep * n + k], dmat[drop_slot * n + k], best_d, ni, nj, sizes[k] as f64);
                dmat[keep * n + k] = updated;
                dmat[k * n + keep] = updated;
            }
            active[drop_slot] = false;
            ids[keep] = step as isize;
            sizes[keep] = new_size;
        } else {
            chain.push(best_j);
        }
    }

    // Reorder subtrees into ascending merge height while preserving topological parent-after-child order.
    let m = raw_merges.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| raw_merges[a].distance.total_cmp(&raw_merges[b].distance).then_with(|| a.cmp(&b)));
    let mut remap = vec![0_isize; m];
    for (new_idx, &orig_idx) in order.iter().enumerate() {
        remap[orig_idx] = new_idx as isize;
    }
    let mut sorted = Vec::with_capacity(m);
    for orig_idx in order {
        let mut entry = raw_merges[orig_idx].clone();
        if entry.left >= 0 {
            entry.left = remap[entry.left as usize];
        }
        if entry.right >= 0 {
            entry.right = remap[entry.right as usize];
        }
        sorted.push(entry);
    }
    sorted
}

#[derive(Clone, Copy)]
struct LinkageHeapEntry {
    dist: f64,
    i: usize,
    j: usize,
    gen_i: usize,
    gen_j: usize,
}

impl PartialEq for LinkageHeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.dist.to_bits() == other.dist.to_bits()
            && self.i == other.i
            && self.j == other.j
            && self.gen_i == other.gen_i
            && self.gen_j == other.gen_j
    }
}

impl Eq for LinkageHeapEntry {}

impl Ord for LinkageHeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .dist
            .total_cmp(&self.dist)
            .then_with(|| other.i.cmp(&self.i))
            .then_with(|| other.j.cmp(&self.j))
            .then_with(|| other.gen_i.cmp(&self.gen_i))
            .then_with(|| other.gen_j.cmp(&self.gen_j))
    }
}

impl PartialOrd for LinkageHeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// O(n^2 log n) lazy-deletion min-heap linkage for non-reducible Centroid and Median rules.
fn heap_linkage(dist_mat: &[Vec<f64>], linkage: ClusterLinkage) -> Vec<DendrogramMerge> {
    let n = dist_mat.len();
    let mut active = vec![true; n];
    let mut dmat = vec![0.0_f64; n * n];
    let mut initial = Vec::with_capacity(n * (n - 1) / 2);
    for (i, row) in dist_mat.iter().enumerate() {
        dmat[i * n..(i + 1) * n].copy_from_slice(row);
        for (j, &d) in row.iter().enumerate().skip(i + 1) {
            initial.push(LinkageHeapEntry { dist: d, i, j, gen_i: 0, gen_j: 0 });
        }
    }
    let mut heap = BinaryHeap::from(initial);
    let mut gens = vec![0_usize; n];
    let mut ids: Vec<isize> = (0..n).map(|i| -((i as isize) + 1)).collect();
    let mut sizes = vec![1_usize; n];
    let mut merges = Vec::with_capacity(n - 1);

    while merges.len() < n - 1 {
        let Some(top) = heap.pop() else {
            break;
        };
        if !active[top.i] || !active[top.j] || top.gen_i != gens[top.i] || top.gen_j != gens[top.j] {
            continue;
        }
        let keep = top.i.min(top.j);
        let drop_slot = top.i.max(top.j);
        let step = merges.len();
        let ni = sizes[keep] as f64;
        let nj = sizes[drop_slot] as f64;
        let new_size = sizes[keep] + sizes[drop_slot];
        merges.push(DendrogramMerge { left: ids[keep], right: ids[drop_slot], distance: top.dist, cluster_size: new_size });

        active[drop_slot] = false;
        gens[keep] += 1;
        ids[keep] = step as isize;
        sizes[keep] = new_size;

        for (k, &is_act) in active.iter().enumerate() {
            if !is_act || k == keep {
                continue;
            }
            let updated = lance_williams_distance(linkage, dmat[keep * n + k], dmat[drop_slot * n + k], top.dist, ni, nj, sizes[k] as f64);
            dmat[keep * n + k] = updated;
            dmat[k * n + keep] = updated;
            let (u, v) = if keep < k { (keep, k) } else { (k, keep) };
            heap.push(LinkageHeapEntry { dist: updated, i: u, j: v, gen_i: gens[u], gen_j: gens[v] });
        }
    }
    merges
}

fn hierarchical_clustering_from_dist(dist_mat: &[Vec<f64>], linkage: ClusterLinkage) -> Result<Vec<DendrogramMerge>, StatsError> {
    let n = dist_mat.len();
    if n < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "hierarchical clustering", required: 2, actual: n });
    }
    if n > MAX_CLUSTERING_N {
        return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
    }
    if let Some(bad) = dist_mat.iter().find(|r| r.len() != n) {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: bad.len() }));
    }
    Ok(match linkage {
        ClusterLinkage::Single | ClusterLinkage::Complete | ClusterLinkage::Upgma | ClusterLinkage::Wpgma | ClusterLinkage::Ward => {
            nn_chain_linkage(dist_mat, linkage)
        }
        ClusterLinkage::Centroid | ClusterLinkage::Median => heap_linkage(dist_mat, linkage),
    })
}

/// Agglomerative hierarchical clustering across all [`ClusterDistance`] metrics (`Euclidean`,
/// `Manhattan`, `Pearson`, `Cosine`) and all 7 [`ClusterLinkage`] criteria (`Ward`, `Complete`,
/// `Single`, `Upgma`, `Wpgma`, `Centroid`, `Median`) via Lance–Williams updates.
pub fn hierarchical_clustering_with_metrics(
    rows: &[Vec<f64>],
    distance: ClusterDistance,
    linkage: ClusterLinkage,
) -> Result<Vec<DendrogramMerge>, StatsError> {
    let dist_mat = pairwise_distance_matrix(rows, distance)?;
    hierarchical_clustering_from_dist(&dist_mat, linkage)
}

fn resolve_node_members(node_id: isize, leaf_n: usize, merged_members: &[Vec<usize>]) -> Result<&[usize], StatsError> {
    if node_id < 0 {
        let leaf = (-node_id - 1) as usize;
        if leaf >= leaf_n {
            return Err(StatsError::InvalidParameter { name: "dendrogram_node", value: node_id as f64 });
        }
        Ok(&merged_members[leaf])
    } else {
        let step_idx = node_id as usize;
        merged_members
            .get(leaf_n + step_idx)
            .map(Vec::as_slice)
            .ok_or(StatsError::InvalidParameter { name: "dendrogram_merge_step", value: node_id as f64 })
    }
}

/// Compute the symmetric `n x n` cophenetic distance matrix from a dendrogram merge schedule.
pub fn cophenetic_distance_matrix(n: usize, merges: &[DendrogramMerge]) -> Result<Vec<Vec<f64>>, StatsError> {
    if n < 2 || merges.len() != n - 1 {
        return Err(StatsError::DimensionMismatch { expected: n.saturating_sub(1), actual: merges.len() });
    }
    if n > MAX_CLUSTERING_N {
        return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
    }
    let mut coph = vec![vec![0.0_f64; n]; n];
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    members.reserve(merges.len());
    for merge in merges {
        let left_m = resolve_node_members(merge.left, n, &members)?.to_vec();
        let right_m = resolve_node_members(merge.right, n, &members)?.to_vec();
        for &u in &left_m {
            for &v in &right_m {
                coph[u][v] = merge.distance;
                coph[v][u] = merge.distance;
            }
        }
        let mut union_m = left_m;
        union_m.extend(right_m);
        members.push(union_m);
    }
    Ok(coph)
}

/// Compute the cophenetic correlation coefficient (`Pearson r` over upper-triangular pairs `i < j`)
/// comparing original pairwise dissimilarities `dist_mat` against cophenetic heights `coph_mat`.
#[must_use]
pub fn cophenetic_correlation(dist_mat: &[Vec<f64>], coph_mat: &[Vec<f64>]) -> Option<f64> {
    let n = dist_mat.len();
    if n < 3 || coph_mat.len() != n {
        return None;
    }
    let m = n * (n - 1) / 2;
    let mut orig = Vec::with_capacity(m);
    let mut coph = Vec::with_capacity(m);
    for i in 0..n {
        if dist_mat[i].len() != n || coph_mat[i].len() != n {
            return None;
        }
        for j in (i + 1)..n {
            orig.push(dist_mat[i][j]);
            coph.push(coph_mat[i][j]);
        }
    }
    pearson_r(&orig, &coph).ok()
}

/// Cut an `n`-leaf hierarchical merge schedule into `k` disjoint flat clusters (`1 <= k <= n`).
pub fn cut_dendrogram(n: usize, merges: &[DendrogramMerge], k: usize) -> Result<Vec<usize>, StatsError> {
    if n < 2 || merges.len() != n - 1 {
        return Err(StatsError::DimensionMismatch { expected: n.saturating_sub(1), actual: merges.len() });
    }
    if k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "cut_k", value: k as f64 });
    }
    let mut node_members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut active_roots: Vec<bool> = vec![true; n + merges.len()];
    let n_merges = n - k;
    for (step, merge) in merges.iter().take(n_merges).enumerate() {
        let left_slot = if merge.left < 0 { (-merge.left - 1) as usize } else { n + (merge.left as usize) };
        let right_slot = if merge.right < 0 { (-merge.right - 1) as usize } else { n + (merge.right as usize) };
        if left_slot >= node_members.len() || right_slot >= node_members.len() {
            return Err(StatsError::InvalidParameter { name: "dendrogram_merge_ref", value: step as f64 });
        }
        let mut combined = node_members[left_slot].clone();
        combined.extend_from_slice(&node_members[right_slot]);
        active_roots[left_slot] = false;
        active_roots[right_slot] = false;
        node_members.push(combined);
    }

    let mut assignments = vec![0_usize; n];
    let mut cluster_id = 0_usize;
    for (slot, members) in node_members.iter().enumerate() {
        if active_roots.get(slot).copied().unwrap_or(false) {
            for &leaf in members {
                assignments[leaf] = cluster_id;
            }
            cluster_id += 1;
        }
    }
    Ok(assignments)
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

/// Execute a [`MultivariateSpec`] (`Pca`, `CorrelationMatrix`, or `Clustering`) on a `DataSheet`.
pub fn run_multivariate_analysis(sheet: &DataSheet, spec: &AnalysisSpec, mv_spec: &MultivariateSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;

    match mv_spec {
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            let (labels, opt_cols) = extract_optional_columns(sheet, &spec.selected_data_sets)?;
            let refs: Vec<(&str, &[Option<f64>])> = labels.iter().zip(opt_cols.iter()).map(|(l, v)| (l.as_str(), v.as_slice())).collect();
            let res = correlation_matrix(&refs, *spearman_rank)?;

            let tab_r = build_tab(
                "corr-matrix-r",
                if *spearman_rank { "Spearman correlation matrix (rho)" } else { "Pearson correlation matrix (r)" },
                ResultTabKind::MainSummary,
                res.labels.clone(),
                res.labels.clone(),
                res.r_matrix.iter().map(|row| row.iter().map(|&v| num_cell(v)).collect()).collect(),
            );
            let tab_p = build_tab(
                "corr-matrix-p",
                "Correlation P values",
                ResultTabKind::ParametersAndConfidenceIntervals,
                res.labels.clone(),
                res.labels.clone(),
                res.p_matrix.iter().map(|row| row.iter().map(|&v| p_cell(v, p_fmt)).collect()).collect(),
            );
            let tab_q = build_tab(
                "corr-matrix-fdr",
                "BH FDR-adjusted q values",
                ResultTabKind::MultipleComparisons,
                res.labels.clone(),
                res.labels.clone(),
                res.fdr_q_matrix.iter().map(|row| row.iter().map(|&v| p_cell(v, p_fmt)).collect()).collect(),
            );
            out.tabs.extend([tab_r, tab_p, tab_q]);

            let min_n = res.n_matrix.iter().flat_map(|r| r.iter()).copied().min().unwrap_or(0);
            out.checklist.push(checklist_item(
                "Pairwise sample sizes & multiplicity",
                if min_n >= 10 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
                Some(format!("min pairwise n = {min_n}, {} variables", res.labels.len())),
                "Upper-triangle P values are adjusted for false discovery rate via Benjamini-Hochberg.",
            ));
        }
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            let (labels, rows) = extract_complete_matrix_rows(sheet, &spec.selected_data_sets)?;
            let n = rows.len();
            let p = labels.len();
            let flat: Vec<f64> = rows.into_iter().flat_map(|r| r.into_iter()).collect();
            let mat = DenseMatrix::from_row_major(n, p, flat)?;
            let pca = pca_analysis(&mat, &labels, *standardize, *parallel_analysis_seed)?;

            if let Some(ev1) = pca.eigenvalues.first() {
                push_scalar(&mut out, "PC1 eigenvalue", None, *ev1);
            }
            if let Some(vp1) = pca.variance_proportion.first() {
                push_scalar(&mut out, "PC1 variance proportion", None, *vp1);
            }
            push_scalar(&mut out, "Kaiser retained PCs", None, pca.kaiser_components as f64);

            let mut headers = vec![String::from("Eigenvalue"), String::from("Variance %"), String::from("Cumulative %")];
            if pca.parallel_analysis_thresholds.is_some() {
                headers.push(String::from("Parallel Analysis (95th pct)"));
            }
            let mut pc_labels = Vec::with_capacity(pca.eigenvalues.len());
            let mut scree_rows = Vec::with_capacity(pca.eigenvalues.len());
            for (idx, &ev) in pca.eigenvalues.iter().enumerate() {
                pc_labels.push(format!("PC{}", idx + 1));
                let mut r = vec![num_cell(ev), num_cell(100.0 * pca.variance_proportion[idx]), num_cell(100.0 * pca.cumulative_proportion[idx])];
                if let Some(pa) = &pca.parallel_analysis_thresholds {
                    r.push(opt_num_cell(pa.get(idx).copied()));
                }
                scree_rows.push(r);
            }
            out.tabs.push(build_tab(
                "pca-eigenvalues",
                "PCA eigenvalues & scree",
                ResultTabKind::MainSummary,
                headers,
                pc_labels.clone(),
                scree_rows,
            ));

            let load_rows: Vec<Vec<ResultCell>> = pca.loadings.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
            out.tabs.push(build_tab(
                "pca-loadings",
                "PCA component loadings",
                ResultTabKind::ParametersAndConfidenceIntervals,
                pc_labels,
                labels,
                load_rows,
            ));

            out.checklist.push(checklist_item(
                "Component retention (Kaiser & Parallel Analysis)",
                ChecklistStatus::Pass,
                Some(format!(
                    "Kaiser = {} PCs{}",
                    pca.kaiser_components,
                    pca.parallel_analysis_components.map_or(String::new(), |c| format!(", Parallel Analysis = {c} PCs"))
                )),
                "Horn's parallel analysis compares empirical eigenvalues to 95th-percentile eigenvalues of uncorrelated Gaussian matrices.",
            ));
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            if let Some(k_max_req) = k_sweep_max
                && *k_max_req < 2
            {
                return Err(StatsError::InvalidParameter { name: "k_sweep_max", value: *k_max_req as f64 });
            }
            let (var_labels, obs_labels, rows) = extract_complete_matrix_rows_with_names(sheet, &spec.selected_data_sets)?;
            let n = rows.len();
            if n > MAX_CLUSTERING_N {
                return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
            }
            let p = var_labels.len();
            let dist_metric = *distance;
            let link_rule = *linkage;

            let dist_mat = pairwise_distance_matrix(&rows, dist_metric)?;
            let merges = hierarchical_clustering_from_dist(&dist_mat, link_rule)?;
            let coph_mat = cophenetic_distance_matrix(n, &merges)?;
            let cophenetic_r = cophenetic_correlation(&dist_mat, &coph_mat);

            let k_max = match k_sweep_max {
                Some(req) => (*req).min(n).min(MAX_CLUSTER_K_SWEEP).max(2.min(n)),
                None => 6.min(n.saturating_sub(1)).max(2).min(n),
            };
            let mut sweep_results: Vec<KMeansResult> = Vec::new();
            for k_try in 2..=k_max {
                if let Ok(res) = kmeans_pp_from_dist(&rows, k_try, dist_metric, &dist_mat, 0xC105_7E57) {
                    sweep_results.push(res);
                }
            }

            let km = if let Some(k) = k_means_k {
                if let Some(found) = sweep_results.iter().find(|r| r.k == *k) {
                    found.clone()
                } else {
                    kmeans_pp_from_dist(&rows, *k, dist_metric, &dist_mat, 0xC105_7E57)?
                }
            } else if let Some(best) = sweep_results
                .iter()
                .filter(|r| n < 3 || r.k < n)
                .max_by(|a, b| a.mean_silhouette.total_cmp(&b.mean_silhouette))
                .or_else(|| sweep_results.first())
            {
                best.clone()
            } else {
                kmeans_pp_from_dist(&rows, 2.min(n), dist_metric, &dist_mat, 0xC105_7E57)?
            };

            push_scalar(&mut out, "Clusters (k)", None, km.k as f64);
            push_scalar(&mut out, "Total WCSS", None, km.wcss);
            push_scalar(&mut out, "Mean silhouette", None, km.mean_silhouette);
            if let Some(r_coph) = cophenetic_r {
                push_scalar(&mut out, "cophenetic_r", None, r_coph);
            }

            let dist_label = cluster_distance_label(dist_metric);
            let link_label = cluster_linkage_label(link_rule);

            if k_means_k.is_some() || k_sweep_max.is_some() || !sweep_results.is_empty() {
                let mut c_headers = vec![String::from("Cluster"), String::from("Size")];
                c_headers.extend(var_labels.iter().map(|l| format!("Centroid ({l})")));
                let mut c_labels = Vec::with_capacity(km.k);
                let mut c_rows = Vec::with_capacity(km.k);
                for (c_idx, cent) in km.centroids.iter().enumerate() {
                    c_labels.push(format!("Cluster {}", c_idx + 1));
                    let cnt = km.assignments.iter().filter(|&&a| a == c_idx).count();
                    let mut r = vec![num_cell((c_idx + 1) as f64), num_cell(cnt as f64)];
                    r.extend(cent.iter().map(|&v| num_cell(v)));
                    c_rows.push(r);
                }
                out.tabs.push(build_tab(
                    "kmeans-clusters",
                    format!("k-means++ cluster centroids ({dist_label})"),
                    ResultTabKind::MainSummary,
                    c_headers,
                    c_labels,
                    c_rows,
                ));

                let mut assign_rows = Vec::with_capacity(n);
                for (i, row_vec) in rows.iter().enumerate() {
                    let c_idx = km.assignments[i];
                    let dist_c = pairwise_distance_unchecked(row_vec, &km.centroids[c_idx], dist_metric);
                    assign_rows.push(vec![num_cell((c_idx + 1) as f64), num_cell(dist_c)]);
                }
                out.tabs.push(build_tab(
                    "kmeans-assignments",
                    "k-means++ observation cluster assignments",
                    ResultTabKind::ParametersAndConfidenceIntervals,
                    vec![String::from("Assigned cluster"), format!("Distance to centroid ({dist_label})")],
                    obs_labels.clone(),
                    assign_rows,
                ));

                if !sweep_results.is_empty() {
                    let s_labels: Vec<String> = sweep_results.iter().map(|r| format!("k = {}", r.k)).collect();
                    let s_rows: Vec<Vec<ResultCell>> =
                        sweep_results.iter().map(|r| vec![num_cell(r.k as f64), num_cell(r.wcss), num_cell(r.mean_silhouette)]).collect();
                    out.tabs.push(build_tab(
                        "kmeans-sweep-curve",
                        "Candidate-k elbow (WCSS) & silhouette curve",
                        ResultTabKind::ModelComparison,
                        vec![String::from("Clusters (k)"), String::from("WCSS"), String::from("Mean silhouette")],
                        s_labels,
                        s_rows,
                    ));
                }
            }

            let d_headers = vec![String::from("Left"), String::from("Right"), format!("{link_label} distance"), String::from("Cluster size")];
            let d_labels: Vec<String> = (1..=merges.len()).map(|s| format!("Merge {s}")).collect();
            let d_rows: Vec<Vec<ResultCell>> = merges
                .iter()
                .map(|m| vec![num_cell(m.left as f64), num_cell(m.right as f64), num_cell(m.distance), num_cell(m.cluster_size as f64)])
                .collect();
            out.tabs.push(build_tab(
                "hierarchical-dendrogram",
                format!("{link_label} dendrogram linkage ({dist_label})"),
                ResultTabKind::ModelComparison,
                d_headers,
                d_labels,
                d_rows,
            ));

            let max_hier_k = n.saturating_sub(1).min(8);
            if max_hier_k >= 2 {
                let mut h_labels = Vec::with_capacity(max_hier_k - 1);
                let mut h_rows = Vec::with_capacity(max_hier_k - 1);
                for k_cut in 2..=max_hier_k {
                    let assigns = cut_dendrogram(n, &merges, k_cut)?;
                    let sil = compute_mean_silhouette_from_dist(&dist_mat, &assigns, k_cut);
                    let mut counts = vec![0_usize; k_cut];
                    for &c in &assigns {
                        counts[c] += 1;
                    }
                    let min_sz = counts.iter().copied().min().unwrap_or(0);
                    let max_sz = counts.iter().copied().max().unwrap_or(0);
                    let cut_height = merges.get(n - k_cut).map_or(0.0, |m| m.distance);
                    h_labels.push(format!("k = {k_cut}"));
                    h_rows.push(vec![num_cell(k_cut as f64), num_cell(cut_height), num_cell(sil), num_cell(min_sz as f64), num_cell(max_sz as f64)]);
                }
                out.tabs.push(build_tab(
                    "hierarchical-cut-silhouettes",
                    format!("Hierarchical cut silhouette widths ({link_label}, {dist_label})"),
                    ResultTabKind::Diagnostics,
                    vec![
                        String::from("Clusters (k)"),
                        String::from("Cut height"),
                        String::from("Mean silhouette"),
                        String::from("Min cluster size"),
                        String::from("Max cluster size"),
                    ],
                    h_labels,
                    h_rows,
                ));
            }

            if n <= MAX_CLUSTER_MATRIX_TAB_N {
                let dist_rows: Vec<Vec<ResultCell>> = dist_mat.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
                out.tabs.push(build_tab(
                    "clustering-distance-matrix",
                    format!("Pairwise dissimilarity matrix ({dist_label})"),
                    ResultTabKind::CovarianceMatrix,
                    obs_labels.clone(),
                    obs_labels.clone(),
                    dist_rows,
                ));

                let coph_rows: Vec<Vec<ResultCell>> = coph_mat.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
                out.tabs.push(build_tab(
                    "cophenetic-distance-matrix",
                    format!("Cophenetic distance matrix ({link_label})"),
                    ResultTabKind::CovarianceMatrix,
                    obs_labels.clone(),
                    obs_labels,
                    coph_rows,
                ));
            } else {
                out.checklist.push(checklist_item(
                    "Dense pairwise & cophenetic matrices omitted for large n",
                    ChecklistStatus::Info,
                    Some(format!("n = {n} exceeds dense table cap ({MAX_CLUSTER_MATRIX_TAB_N})")),
                    "Cophenetic correlation (r), dendrogram linkage schedule, and cluster assignments are still reported.",
                ));
            }

            out.tabs.push(build_tab(
                "clustering-diagnostics",
                "Clustering configuration & cophenetic diagnostics",
                ResultTabKind::Diagnostics,
                vec![String::from("Value")],
                vec![
                    String::from("Cophenetic correlation (r)"),
                    String::from("Distance metric"),
                    String::from("Linkage rule"),
                    String::from("Complete observations (n)"),
                    String::from("Variables (p)"),
                ],
                vec![
                    vec![opt_num_cell(cophenetic_r)],
                    vec![ResultCell::Text(dist_label.to_owned())],
                    vec![ResultCell::Text(link_label.to_owned())],
                    vec![num_cell(n as f64)],
                    vec![num_cell(p as f64)],
                ],
            ));

            out.checklist.push(checklist_item(
                "Cluster separation quality & cophenetic fit",
                if km.mean_silhouette >= 0.25 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
                Some(format!(
                    "k = {}, silhouette = {:.3}, WCSS = {:.2}, cophenetic r = {}",
                    km.k,
                    km.mean_silhouette,
                    km.wcss,
                    cophenetic_r.map_or_else(|| String::from("n/a"), |r| format!("{r:.3}"))
                )),
                format!(
                    "Mean silhouette near +1 indicates tight, well-separated clusters under {dist_label} dissimilarity; {link_label} dendrogram cophenetic r measures ultrametric tree fidelity."
                ),
            ));
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, response_column, predictor_columns } => {
            let extracted = extract_mi_columns(sheet, &spec.selected_data_sets, *response_column, predictor_columns)?;
            let config = MultipleImputationConfig {
                imputations_m: *imputations_m,
                max_iters: *max_iters,
                seed: *seed,
                response_col: extracted.model_response_col,
                predictor_cols: extracted.model_predictor_cols,
                confidence: spec.confidence_level,
            };
            let res = mice_predictive_mean_matching(&extracted.labels, &extracted.cols, &config)?;
            populate_mi_result_sheet(&mut out, &res, p_fmt, spec.confidence_level);
        }
    }

    Ok(out)
}

// ============================================================================
// Multiple Imputation (MICE PMM) & Rubin's Pooling Rules
// ============================================================================

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
            t_statistic: 0.0,
            df,
            p_value: 1.0,
            ci: (q_bar, q_bar),
            riv: 0.0,
            lambda: 0.0,
            fmi: 0.0,
        });
    }

    let pooled_se = t_total.sqrt();
    let lambda = (b_inflated / t_total).clamp(0.0, 1.0);
    let riv = if u_bar > 0.0 { b_inflated / u_bar } else { 1.0e12 };

    let (df, fmi) = if b <= 0.0 || lambda <= 1.0e-15 {
        (complete_data_df.unwrap_or(1.0e6), 0.0)
    } else {
        let nu_old = (m_f - 1.0) / (lambda * lambda);
        let df_val = if let Some(vc) = complete_data_df {
            let nu_obs = ((vc + 1.0) / (vc + 3.0)) * vc * (1.0 - lambda).max(1.0e-12);
            ((nu_old * nu_obs) / (nu_old + nu_obs)).max(1.0e-6)
        } else {
            nu_old.max(1.0e-6)
        };
        let fmi_val = (lambda + (1.0 - lambda) * (2.0 / (df_val + 3.0))).clamp(0.0, 1.0);
        (df_val, fmi_val)
    };

    let t_stat = q_bar / pooled_se;
    let p_val = student_t_two_sided_p(t_stat, df)?;
    let t_crit = t_crit_two_sided(confidence, df)?;

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

    let m = config.imputations_m.clamp(2, 50);
    let max_iters = config.max_iters.clamp(1, 100);
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
    let y_obs: Vec<f64> = obs_rows.iter().map(|&r| rows[r][0]).collect();
    let Ok(m) = SummaryMoments::from_slice(&y_obs) else {
        return;
    };
    let df = n_obs.saturating_sub(1).max(1);
    let g = chi2_draw(df, rng);
    let sigma_star = if m.css > 0.0 { (m.css / g).sqrt() } else { 0.0 };
    let mu_star = m.mean + (sigma_star / (n_obs as f64).sqrt()) * rng.standard_normal();

    let donors_k = if n_obs <= 3 { n_obs } else { (n_obs / 2).clamp(2, 5) };
    let mut ranked: Vec<(usize, f64)> = y_obs.iter().enumerate().map(|(i, &yv)| (i, (yv - mu_star).abs())).collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    for &r_mis in mis_rows {
        let pick = rng.gen_index(donors_k).unwrap_or(0);
        rows[r_mis][0] = y_obs[ranked[pick].0];
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

struct ExtractedMiData {
    labels: Vec<String>,
    cols: Vec<Vec<Option<f64>>>,
    model_response_col: Option<usize>,
    model_predictor_cols: Vec<usize>,
}

struct RawOptionalColumns {
    labels: Vec<String>,
    cols: Vec<Vec<Option<f64>>>,
    is_label: Vec<bool>,
}

fn extract_raw_optional_columns(sheet: &DataSheet) -> Result<RawOptionalColumns, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let mut labels = Vec::with_capacity(mv.variables.len());
            let mut cols = Vec::with_capacity(mv.variables.len());
            let mut is_label = Vec::with_capacity(mv.variables.len());
            for v in &mv.variables {
                labels.push(v.title.clone());
                cols.push(v.cells.iter().map(|c| c.active_number()).collect());
                is_label.push(matches!(v.role, VariableRole::Label));
            }
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        TableArchetype::Column(col_tab) => {
            let labels = col_tab.data_sets.iter().map(|ds| ds.title.clone()).collect();
            let cols =
                col_tab.data_sets.iter().map(|ds| ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()).collect();
            let is_label = vec![false; col_tab.data_sets.len()];
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        TableArchetype::Xy(xy) => {
            let mut labels = vec![xy.x_title.clone()];
            let mut cols = vec![xy.x_cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()];
            for ds in &xy.data_sets {
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
            let is_label = vec![false; labels.len()];
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        other => Err(StatsError::IncompatibleTable { analysis: "multiple imputation", actual: other.kind() }),
    }
}

fn push_unique(target: &mut Vec<usize>, val: usize) {
    if !target.contains(&val) {
        target.push(val);
    }
}

fn collect_mi_column_indices(
    raw_cols: &[Vec<Option<f64>>],
    is_label: &[bool],
    selected: &[usize],
    response_column: Option<usize>,
    predictor_columns: &[usize],
) -> Vec<usize> {
    let total_cols = raw_cols.len();
    let is_numeric_usable = |idx: usize| !is_label[idx] && raw_cols[idx].iter().any(|c| c.is_some_and(f64::is_finite));
    let mut chosen = Vec::new();

    if response_column.is_some() || !predictor_columns.is_empty() {
        if let Some(resp) = response_column {
            push_unique(&mut chosen, resp);
        }
        for &pred in predictor_columns {
            push_unique(&mut chosen, pred);
        }
        let extra = if !selected.is_empty() {
            resolve_selected_indices(selected, total_cols)
        } else if response_column.is_some() && predictor_columns.is_empty() {
            (0..total_cols).collect()
        } else {
            Vec::new()
        };
        for idx in extra {
            if is_numeric_usable(idx) {
                push_unique(&mut chosen, idx);
            }
        }
        return chosen;
    }

    if !selected.is_empty() {
        for idx in resolve_selected_indices(selected, total_cols) {
            push_unique(&mut chosen, idx);
        }
        return chosen;
    }

    for idx in 0..total_cols {
        if is_numeric_usable(idx) {
            chosen.push(idx);
        }
    }
    chosen
}

fn extract_mi_columns(
    sheet: &DataSheet,
    selected: &[usize],
    response_column: Option<usize>,
    predictor_columns: &[usize],
) -> Result<ExtractedMiData, StatsError> {
    let raw = extract_raw_optional_columns(sheet)?;
    let (raw_labels, raw_cols, is_label) = (raw.labels, raw.cols, raw.is_label);
    let total_cols = raw_cols.len();
    if total_cols == 0 {
        return Err(StatsError::EmptyInput);
    }

    if let Some(resp) = response_column
        && resp >= total_cols
    {
        return Err(StatsError::MissingColumn { index: resp });
    }
    for &pred in predictor_columns {
        if pred >= total_cols {
            return Err(StatsError::MissingColumn { index: pred });
        }
    }

    let chosen_orig = collect_mi_column_indices(&raw_cols, &is_label, selected, response_column, predictor_columns);
    if chosen_orig.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let max_len = chosen_orig.iter().map(|&idx| raw_cols[idx].len()).max().unwrap_or(0);
    let mut padded_cols: Vec<Vec<Option<f64>>> = chosen_orig
        .iter()
        .map(|&idx| {
            let mut v: Vec<Option<f64>> = raw_cols[idx].iter().map(|&c| c.filter(|x| x.is_finite())).collect();
            v.resize(max_len, None);
            v
        })
        .collect();

    // Trim trailing completely blank spreadsheet rows without dropping internal missingness.
    let mut effective_rows = max_len;
    while effective_rows > 0 && padded_cols.iter().all(|c| c[effective_rows - 1].is_none()) {
        effective_rows -= 1;
    }
    for col in &mut padded_cols {
        col.truncate(effective_rows);
    }

    let labels: Vec<String> = chosen_orig.iter().map(|&idx| raw_labels[idx].clone()).collect();
    let model_response_col = response_column.and_then(|r| chosen_orig.iter().position(|&orig| orig == r));
    let model_predictor_cols = if let Some(resp_pos) = model_response_col {
        if predictor_columns.is_empty() {
            (0..chosen_orig.len()).filter(|&pos| pos != resp_pos).collect()
        } else {
            predictor_columns
                .iter()
                .filter_map(|&p_orig| chosen_orig.iter().position(|&orig| orig == p_orig))
                .filter(|&pos| pos != resp_pos)
                .collect()
        }
    } else {
        Vec::new()
    };

    Ok(ExtractedMiData { labels, cols: padded_cols, model_response_col, model_predictor_cols })
}

fn populate_mi_result_sheet(out: &mut ResultSheet, res: &MultipleImputationResult, p_fmt: curvebook_doc::PValueFormat, confidence: ConfidenceLevel) {
    push_scalar(out, "Imputations (m)", None, res.m as f64);
    push_scalar(out, "Max iterations", None, res.max_iters as f64);
    push_scalar(out, "Total missing cells", None, res.total_missing as f64);
    for pm in &res.pooled_means {
        push_scalar(out, format!("{}: Pooled mean", pm.term), None, pm.estimate);
        push_scalar(out, format!("{}: Pooled SE", pm.term), None, pm.pooled_se);
        push_scalar(out, format!("{}: FMI", pm.term), None, pm.fmi);
    }

    let ci_header = format!("{}% CI", confidence.percent());
    let mean_headers = vec![
        String::from("Pooled mean"),
        String::from("Within var (U)"),
        String::from("Between var (B)"),
        String::from("Total var (T)"),
        String::from("Pooled SE"),
        ci_header.clone(),
        String::from("df (Barnard-Rubin)"),
        String::from("RIV"),
        String::from("FMI"),
        String::from("Missing count"),
    ];
    let mut mean_rows = Vec::with_capacity(res.pooled_means.len());
    for (pm, &mis_cnt) in res.pooled_means.iter().zip(res.missing_counts.iter()) {
        mean_rows.push(vec![
            num_cell(pm.estimate),
            num_cell(pm.within_variance),
            num_cell(pm.between_variance),
            num_cell(pm.total_variance),
            num_cell(pm.pooled_se),
            ci_cell(pm.ci.0, pm.ci.1),
            num_cell(pm.df),
            num_cell(pm.riv),
            num_cell(pm.fmi),
            num_cell(mis_cnt as f64),
        ]);
    }
    out.tabs.push(build_tab(
        "mi-pooled-means",
        "Pooled descriptive estimates (Rubin 1987)",
        ResultTabKind::MainSummary,
        mean_headers,
        res.variable_labels.clone(),
        mean_rows,
    ));

    if let Some(reg_terms) = &res.pooled_regression {
        let reg_headers = vec![
            String::from("Pooled estimate"),
            String::from("Pooled SE"),
            ci_header,
            String::from("t"),
            String::from("df"),
            String::from("P value"),
            String::from("Within var (U)"),
            String::from("Between var (B)"),
            String::from("Total var (T)"),
            String::from("RIV"),
            String::from("FMI"),
        ];
        let mut reg_labels = Vec::with_capacity(reg_terms.len());
        let mut reg_rows = Vec::with_capacity(reg_terms.len());
        for term in reg_terms {
            push_scalar(out, format!("Reg {}: Pooled estimate", term.term), None, term.estimate);
            push_scalar(out, format!("Reg {}: Pooled SE", term.term), None, term.pooled_se);
            push_scalar(out, format!("Reg {}: P value", term.term), None, term.p_value);
            push_scalar(out, format!("Reg {}: FMI", term.term), None, term.fmi);
            reg_labels.push(term.term.clone());
            reg_rows.push(vec![
                num_cell(term.estimate),
                num_cell(term.pooled_se),
                ci_cell(term.ci.0, term.ci.1),
                num_cell(term.t_statistic),
                num_cell(term.df),
                p_cell(term.p_value, p_fmt),
                num_cell(term.within_variance),
                num_cell(term.between_variance),
                num_cell(term.total_variance),
                num_cell(term.riv),
                num_cell(term.fmi),
            ]);
        }
        out.tabs.push(build_tab(
            "mi-pooled-regression",
            "Pooled regression parameters (Rubin 1987)",
            ResultTabKind::ParametersAndConfidenceIntervals,
            reg_headers,
            reg_labels,
            reg_rows,
        ));
    }

    let diag_headers = vec![
        String::from("Observed n"),
        String::from("Missing count"),
        String::from("Missing %"),
        String::from("Imputations (m)"),
        String::from("FMI"),
    ];
    let diag_rows: Vec<Vec<ResultCell>> = res
        .missing_counts
        .iter()
        .zip(res.pooled_means.iter())
        .map(|(&mis, pm)| {
            let obs = res.n_rows.saturating_sub(mis);
            let pct = if res.n_rows > 0 { 100.0 * (mis as f64) / (res.n_rows as f64) } else { 0.0 };
            vec![num_cell(obs as f64), num_cell(mis as f64), num_cell(pct), num_cell(res.m as f64), num_cell(pm.fmi)]
        })
        .collect();
    out.tabs.push(build_tab(
        "mi-missingness",
        "Missingness & imputation diagnostics",
        ResultTabKind::Diagnostics,
        diag_headers,
        res.variable_labels.clone(),
        diag_rows,
    ));

    let max_fmi = res.pooled_means.iter().map(|e| e.fmi).chain(res.pooled_regression.iter().flatten().map(|e| e.fmi)).fold(0.0_f64, f64::max);
    let adequacy_status = if res.total_missing == 0 {
        ChecklistStatus::Info
    } else if (res.m as f64) + 1.0e-9 >= (100.0 * max_fmi).ceil().min(50.0) || max_fmi <= 0.25 {
        ChecklistStatus::Pass
    } else {
        ChecklistStatus::Warn
    };
    out.checklist.push(checklist_item(
        "Multiple imputation pooling & FMI",
        adequacy_status,
        Some(format!("m = {}, {} missing cell(s), max FMI = {:.3}", res.m, res.total_missing, max_fmi)),
        "Chained equations use Type-1 predictive mean matching with posterior parameter draws and Barnard–Rubin small-sample df; higher FMI indicates greater between-imputation missingness uncertainty.",
    ));
}

#[inline]
const fn cluster_distance_label(distance: ClusterDistance) -> &'static str {
    match distance {
        ClusterDistance::Euclidean => "Euclidean",
        ClusterDistance::Manhattan => "Manhattan",
        ClusterDistance::Pearson => "Pearson (1 - r)",
        ClusterDistance::Cosine => "Cosine (1 - cos)",
    }
}

#[inline]
const fn cluster_linkage_label(linkage: ClusterLinkage) -> &'static str {
    match linkage {
        ClusterLinkage::Ward => "Ward.D2",
        ClusterLinkage::Complete => "Complete",
        ClusterLinkage::Single => "Single",
        ClusterLinkage::Upgma => "UPGMA",
        ClusterLinkage::Wpgma => "WPGMA",
        ClusterLinkage::Centroid => "Centroid",
        ClusterLinkage::Median => "Median",
    }
}

type OptionalColumnsPair = (Vec<String>, Vec<Vec<Option<f64>>>);

fn extract_optional_columns(sheet: &DataSheet, selected: &[usize]) -> Result<OptionalColumnsPair, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let indices = if selected.is_empty() {
                (0..mv.variables.len()).filter(|&idx| !matches!(mv.variables[idx].role, VariableRole::Categorical | VariableRole::Label)).collect()
            } else {
                resolve_selected_indices(selected, mv.variables.len())
            };
            let mut labels = Vec::with_capacity(indices.len());
            let mut cols = Vec::with_capacity(indices.len());
            for idx in indices {
                let v = &mv.variables[idx];
                labels.push(v.title.clone());
                cols.push(v.cells.iter().map(|c| c.active_number()).collect());
            }
            Ok((labels, cols))
        }
        TableArchetype::Column(col_tab) => {
            let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
            let mut labels = Vec::with_capacity(indices.len());
            let mut cols = Vec::with_capacity(indices.len());
            for idx in indices {
                let ds = &col_tab.data_sets[idx];
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
            Ok((labels, cols))
        }
        TableArchetype::Xy(xy) => {
            let mut labels = vec![xy.x_title.clone()];
            let mut cols = vec![xy.x_cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()];
            for ds in &xy.data_sets {
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
            Ok((labels, cols))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "multivariate correlation", actual: other.kind() }),
    }
}

type CompleteMatrixWithNames = (Vec<String>, Vec<String>, Vec<Vec<f64>>);

fn extract_complete_matrix_rows_with_names(sheet: &DataSheet, selected: &[usize]) -> Result<CompleteMatrixWithNames, StatsError> {
    let (labels, opt_cols) = extract_optional_columns(sheet, selected)?;
    if labels.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate variables", required: 2, actual: labels.len() });
    }
    let sheet_row_titles: Option<&[String]> = match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => Some(&mv.row_titles),
        TableArchetype::Column(col) => Some(&col.row_titles),
        TableArchetype::Xy(xy) => Some(&xy.row_titles),
        _ => None,
    };
    let n_rows = opt_cols[0].len();
    let mut obs_labels = Vec::new();
    let mut rows = Vec::new();
    for r in 0..n_rows {
        let mut row = Vec::with_capacity(labels.len());
        let mut ok = true;
        for col in &opt_cols {
            if let Some(v) = col.get(r).and_then(|x| *x)
                && v.is_finite()
            {
                row.push(v);
            } else {
                ok = false;
                break;
            }
        }
        if ok {
            let r_label = sheet_row_titles
                .and_then(|t| t.get(r))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map_or_else(|| format!("Obs {}", r + 1), ToOwned::to_owned);
            obs_labels.push(r_label);
            rows.push(row);
        }
    }
    if rows.len() < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate complete rows", required: 3, actual: rows.len() });
    }
    let _ = extract_numeric_columns;
    Ok((labels, obs_labels, rows))
}

fn extract_complete_matrix_rows(sheet: &DataSheet, selected: &[usize]) -> Result<(Vec<String>, Vec<Vec<f64>>), StatsError> {
    let (labels, _obs_labels, rows) = extract_complete_matrix_rows_with_names(sheet, selected)?;
    Ok((labels, rows))
}

/// Expose Pearson `r` helper directly for pairwise convenience.
#[must_use]
pub fn ci_cell_from_option(ci: Option<(f64, f64)>) -> ResultCell {
    match ci {
        Some((lo, hi)) => ci_cell(lo, hi),
        None => ResultCell::Empty,
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, DataSheet, MultipleVariablesTable, MultivariateSpec, SheetId,
        TableArchetype, VariableRole,
    };

    use super::{extract_optional_columns, run_multivariate_analysis};

    #[test]
    fn empty_selection_excludes_categorical_and_label_columns_including_appended_cluster() -> Result<(), Box<dyn std::error::Error>> {
        let mut mv = MultipleVariablesTable::new(5, 6)?;
        mv.variables[0].title = String::from("SampleLabel");
        mv.variables[0].role = VariableRole::Label;
        mv.variables[1].title = String::from("F1");
        mv.variables[1].role = VariableRole::Continuous;
        mv.variables[2].title = String::from("F2");
        mv.variables[2].role = VariableRole::Continuous;
        mv.variables[3].title = String::from("F3Calc");
        mv.variables[3].role = VariableRole::Calculated;
        mv.variables[4].title = String::from("Cluster");
        mv.variables[4].role = VariableRole::Categorical;

        let coords = [[1.0, 1.0, 2.0], [1.1, 0.9, 2.1], [0.9, 1.1, 1.9], [10.0, 10.0, 8.0], [10.2, 9.8, 8.2], [9.9, 10.1, 7.9]];
        for (r, vals) in coords.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::text(format!("S{}", r + 1));
            mv.variables[1].cells[r] = Cell::number(vals[0]);
            mv.variables[2].cells[r] = Cell::number(vals[1]);
            mv.variables[3].cells[r] = Cell::number(vals[2]);
            mv.variables[4].cells[r] = Cell::text(if r < 3 { "Cluster 1" } else { "Cluster 2" });
        }

        let sheet = DataSheet::new(SheetId::new("mv-cluster"), "Cluster MV", TableArchetype::MultipleVariables(mv))?;
        let (labels, cols) = extract_optional_columns(&sheet, &[])?;
        assert_eq!(labels, vec![String::from("F1"), String::from("F2"), String::from("F3Calc")]);
        assert_eq!(cols.len(), 3);

        let explicit = extract_optional_columns(&sheet, &[1, 4])?;
        assert_eq!(explicit.0, vec![String::from("F1"), String::from("Cluster")]);

        let mv_spec = MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        };
        let spec = AnalysisSpec::new(SheetId::new("res-cl"), SheetId::new("mv-cluster"), AnalysisKind::Multivariate(mv_spec.clone()))?;
        let res = run_multivariate_analysis(&sheet, &spec, &mv_spec)?;
        assert!(res.tabs.iter().any(|t| t.id == "kmeans-assignments"));
        Ok(())
    }
}
