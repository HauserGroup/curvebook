//! Multivariate statistics: Pearson *r* (Fisher-*z* CI), Spearman *rho*, Kendall *tau_b*,
//! correlation matrices with Benjamini–Hochberg multiplicity, Principal Component Analysis (SVD,
//! scree, Kaiser, Horn's parallel analysis, scores, loadings, biplot contributions), *k*-means++,
//! and Hierarchical Clustering (Single, Complete, Average/UPGMA, Ward.D2 dendrogram linkage).

use lumencraft_doc::{AnalysisSpec, ChecklistStatus, DataSheet, MultivariateSpec, ResultCell, ResultSheet, ResultTabKind, TableArchetype};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{norm_two_sided_isf, norm_two_sided_p, student_t_two_sided_p};
use lumencraft_math::linalg::{DenseMatrix, svd};
use lumencraft_math::rng::DeterministicRng;
use lumencraft_math::sum::{SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use crate::common::{
    build_tab, checklist_item, ci_cell, extract_numeric_columns, init_result_sheet, num_cell, opt_num_cell, p_cell, push_scalar, quantile_sorted,
    rank_with_ties, resolve_selected_indices, validate_finite_slice,
};
use crate::error::StatsError;
use crate::multiplicity::adjust_benjamini_hochberg;

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

// ============================================================================
// Correlations (Pearson, Spearman, Kendall tau-b, Correlation Matrix)
// ============================================================================

/// Pearson product-moment correlation with Fisher-*z* confidence interval and exact *t*-test *p*.
pub fn pearson_correlation(x: &[f64], y: &[f64], confidence: ConfidenceLevel) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
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
pub fn spearman_correlation(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
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
pub fn kendall_tau_b(x: &[f64], y: &[f64]) -> Result<CorrelationPairResult, StatsError> {
    if x.len() != y.len() {
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: x.len(), actual: y.len() }));
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

/// Principal Component Analysis via SVD of centered (and optionally standardized) `n x p` matrix.
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
        return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: p, actual: variable_labels.len() }));
    }

    let mut scaled_cols = Vec::with_capacity(p);
    for c in 0..p {
        let mut col = Vec::with_capacity(n);
        for r in 0..n {
            let v = data.get(r, c).unwrap_or(f64::NAN);
            if !v.is_finite() {
                return Err(StatsError::Math(lumencraft_math::MathError::DomainError { name: "pca_x", value: v }));
            }
            col.push(v);
        }
        let m = SummaryMoments::from_slice(&col)?;
        let sd = m.sample_sd().unwrap_or(1.0);
        let denom = if standardize && sd > 0.0 { sd } else { 1.0 };
        scaled_cols.push(col.into_iter().map(|v| (v - m.mean) / denom).collect::<Vec<_>>());
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

    // Loadings V (p x k_comp) and Scores = X_scaled * V (n x k_comp).
    let mut loadings = vec![vec![0.0_f64; k_comp]; p];
    let mut contributions = vec![vec![0.0_f64; k_comp]; p];
    for i in 0..p {
        for j in 0..k_comp {
            let v_ij = dec.v.get(i, j).unwrap_or(0.0);
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
        let thresh = horn_parallel_analysis(n, p, k_comp, standardize, seed)?;
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

fn horn_parallel_analysis(n: usize, p: usize, k_comp: usize, standardize: bool, seed: u64) -> Result<Vec<f64>, StatsError> {
    let n_iter = 40_usize;
    let mut rng = DeterministicRng::from_seed(seed);
    let mut sim_eigs = vec![Vec::with_capacity(n_iter); k_comp];
    let scale_n = ((n - 1) as f64).sqrt();

    for _ in 0..n_iter {
        let mut mat_data = vec![0.0_f64; n * p];
        for c in 0..p {
            let mut col: Vec<f64> = (0..n).map(|_| rng.standard_normal()).collect();
            if standardize && let Ok(m) = SummaryMoments::from_slice(&col) {
                let sd = m.sample_sd().unwrap_or(1.0).max(1e-12);
                for v in &mut col {
                    *v = (*v - m.mean) / sd;
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
    let mut out = Vec::with_capacity(k_comp);
    for mut v in sim_eigs {
        v.sort_by(f64::total_cmp);
        out.push(quantile_sorted(&v, 0.95).unwrap_or(1.0));
    }
    Ok(out)
}

// ============================================================================
// k-Means++ & Hierarchical Agglomerative Clustering
// ============================================================================

/// *k*-means++ clustering (`DeterministicRng::from_seed`) returning cluster assignments,
/// centroids, total within-cluster sum of squares (WCSS), and mean silhouette score.
pub fn kmeans_pp(rows: &[Vec<f64>], k: usize, seed: u64) -> Result<KMeansResult, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    for r in rows {
        if r.len() != d {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: d, actual: r.len() }));
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

fn compute_mean_silhouette(rows: &[Vec<f64>], assignments: &[usize], k: usize) -> f64 {
    let n = rows.len();
    if k < 2 || n <= k {
        return 0.0;
    }
    let dist = |a: &[f64], b: &[f64]| -> f64 { a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum::<f64>().sqrt() };
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
            let d = dist(&rows[i], &rows[j]);
            let cj = assignments[j];
            if cj == ci {
                intra_sum += d;
                intra_cnt += 1;
            } else {
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

/// Agglomerative hierarchical clustering supporting Single, Complete, Average (UPGMA), and Ward.D2
/// linkage via Lance–Williams dissimilarity updates.
pub fn hierarchical_clustering(rows: &[Vec<f64>], linkage: LinkageMethod) -> Result<Vec<DendrogramMerge>, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "hierarchical clustering", required: 2, actual: n });
    }
    for r in rows {
        if r.len() != d {
            return Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: d, actual: r.len() }));
        }
        validate_finite_slice(r)?;
    }

    let mut active = vec![true; n];
    let mut ids: Vec<isize> = (0..n).map(|i| -((i as isize) + 1)).collect();
    let mut sizes = vec![1_usize; n];
    let mut dist_mat = vec![vec![0.0_f64; n]; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let dist = rows[i].iter().zip(rows[j].iter()).map(|(x, y)| (x - y).powi(2)).sum::<f64>().sqrt();
            dist_mat[i][j] = dist;
            dist_mat[j][i] = dist;
        }
    }

    let mut merges = Vec::with_capacity(n - 1);
    for step in 0..(n - 1) {
        let mut best_i = 0_usize;
        let mut best_j = 1_usize;
        let mut best_d = f64::INFINITY;
        for i in 0..n {
            if !active[i] {
                continue;
            }
            for j in (i + 1)..n {
                if active[j] && dist_mat[i][j] < best_d {
                    best_d = dist_mat[i][j];
                    best_i = i;
                    best_j = j;
                }
            }
        }

        let ni = sizes[best_i] as f64;
        let nj = sizes[best_j] as f64;
        let new_size = sizes[best_i] + sizes[best_j];
        merges.push(DendrogramMerge { left: ids[best_i], right: ids[best_j], distance: best_d, cluster_size: new_size });

        // Lance-Williams update into slot `best_i`, deactivate `best_j`.
        for k in 0..n {
            if !active[k] || k == best_i || k == best_j {
                continue;
            }
            let dik = dist_mat[best_i][k];
            let djk = dist_mat[best_j][k];
            let nk = sizes[k] as f64;
            let updated = match linkage {
                LinkageMethod::Single => dik.min(djk),
                LinkageMethod::Complete => dik.max(djk),
                LinkageMethod::Average => (ni * dik + nj * djk) / (ni + nj),
                LinkageMethod::WardD2 => {
                    let denom = ni + nj + nk;
                    let sq = ((ni + nk) * dik * dik + (nj + nk) * djk * djk - nk * best_d * best_d) / denom;
                    sq.max(0.0).sqrt()
                }
            };
            dist_mat[best_i][k] = updated;
            dist_mat[k][best_i] = updated;
        }
        active[best_j] = false;
        ids[best_i] = step as isize;
        sizes[best_i] = new_size;
    }

    Ok(merges)
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
        MultivariateSpec::Clustering { k_means_k } => {
            let (labels, rows) = extract_complete_matrix_rows(sheet, &spec.selected_data_sets)?;
            let n = rows.len();
            let k_chosen = match k_means_k {
                Some(k) => *k,
                None => {
                    let max_k = 6.min(n - 1).max(2);
                    let mut best_k = 2;
                    let mut best_sil = f64::NEG_INFINITY;
                    for k_try in 2..=max_k {
                        if let Ok(km) = kmeans_pp(&rows, k_try, 0xC105_7E57)
                            && km.mean_silhouette > best_sil
                        {
                            best_sil = km.mean_silhouette;
                            best_k = k_try;
                        }
                    }
                    best_k
                }
            };
            let km = kmeans_pp(&rows, k_chosen, 0xC105_7E57)?;
            let merges = hierarchical_clustering(&rows, LinkageMethod::WardD2)?;

            push_scalar(&mut out, "Clusters (k)", None, km.k as f64);
            push_scalar(&mut out, "Total WCSS", None, km.wcss);
            push_scalar(&mut out, "Mean silhouette", None, km.mean_silhouette);

            let mut c_headers = vec![String::from("Cluster"), String::from("Size")];
            c_headers.extend(labels.iter().map(|l| format!("Centroid ({l})")));
            let mut c_labels = Vec::with_capacity(km.k);
            let mut c_rows = Vec::with_capacity(km.k);
            for (c_idx, cent) in km.centroids.iter().enumerate() {
                c_labels.push(format!("Cluster {}", c_idx + 1));
                let cnt = km.assignments.iter().filter(|&&a| a == c_idx).count();
                let mut r = vec![num_cell((c_idx + 1) as f64), num_cell(cnt as f64)];
                r.extend(cent.iter().map(|&v| num_cell(v)));
                c_rows.push(r);
            }
            out.tabs.push(build_tab("kmeans-clusters", "k-means++ cluster centroids", ResultTabKind::MainSummary, c_headers, c_labels, c_rows));

            let d_headers = vec![String::from("Left"), String::from("Right"), String::from("Ward.D2 distance"), String::from("Cluster size")];
            let d_labels: Vec<String> = (1..=merges.len()).map(|s| format!("Merge {s}")).collect();
            let d_rows: Vec<Vec<ResultCell>> = merges
                .iter()
                .map(|m| vec![num_cell(m.left as f64), num_cell(m.right as f64), num_cell(m.distance), num_cell(m.cluster_size as f64)])
                .collect();
            out.tabs.push(build_tab(
                "hierarchical-dendrogram",
                "Ward.D2 dendrogram linkage",
                ResultTabKind::ModelComparison,
                d_headers,
                d_labels,
                d_rows,
            ));

            out.checklist.push(checklist_item(
                "Cluster separation quality (Silhouette)",
                if km.mean_silhouette >= 0.25 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
                Some(format!("k = {}, silhouette = {:.3}, WCSS = {:.2}", km.k, km.mean_silhouette, km.wcss)),
                "Mean silhouette near +1 indicates tight, well-separated clusters; Ward.D2 dendrogram captures nested agglomerative hierarchy.",
            ));
        }
    }

    Ok(out)
}

type OptionalColumnsPair = (Vec<String>, Vec<Vec<Option<f64>>>);

fn extract_optional_columns(sheet: &DataSheet, selected: &[usize]) -> Result<OptionalColumnsPair, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let indices = resolve_selected_indices(selected, mv.variables.len());
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

fn extract_complete_matrix_rows(sheet: &DataSheet, selected: &[usize]) -> Result<(Vec<String>, Vec<Vec<f64>>), StatsError> {
    let (labels, opt_cols) = extract_optional_columns(sheet, selected)?;
    if labels.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate variables", required: 2, actual: labels.len() });
    }
    let n_rows = opt_cols[0].len();
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
            rows.push(row);
        }
    }
    if rows.len() < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate complete rows", required: 3, actual: rows.len() });
    }
    let _ = extract_numeric_columns;
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
