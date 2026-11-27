//! Consensus optimal-*k* cluster validity suite (17 internal indices including Tibshirani et al.
//! 2001 Gap statistic), Hartigan–Wong (1979) and MacQueen (1967) *k*-means algorithms, and
//! feature-level one-way ANOVA summaries across clusters.
//!
//! # References
//!
//! - Hartigan, J. A., & Wong, M. A. (1979). Algorithm AS 136: A K-Means Clustering Algorithm.
//!   *Journal of the Royal Statistical Society, Series C (Applied Statistics)*, 28(1), 100–108.
//!   `doi:10.2307/2346830`.
//! - MacQueen, J. (1967). Some methods for classification and analysis of multivariate observations.
//!   *Proceedings of the Fifth Berkeley Symposium on Mathematical Statistics and Probability*, 1,
//!   281–297.
//! - Lloyd, S. P. (1982). Least squares quantization in PCM. *IEEE Transactions on Information
//!   Theory*, 28(2), 129–137. `doi:10.1109/tit.1982.1056489`.
//! - Charrad, M., Ghazzali, N., Boiteau, V., & Niknafs, A. (2014). NbClust: An R package for
//!   determining the relevant number of clusters in a data set. *Journal of Statistical Software*,
//!   61(6), 1–36. `doi:10.18637/jss.v061.i06`.
//! - Tibshirani, R., Walther, G., & Hastie, T. (2001). Estimating the number of clusters in a data
//!   set via the gap statistic. *Journal of the Royal Statistical Society, Series B*, 63(2),
//!   411–423. `doi:10.1111/1467-9868.00293`.
//! - Caliński, T., & Harabasz, J. (1974). A dendrite method for cluster analysis. *Communications
//!   in Statistics*, 3(1), 1–27. `doi:10.1080/03610927408827101`.
//! - Davies, D. L., & Bouldin, D. W. (1979). A cluster separation measure. *IEEE Transactions on
//!   Pattern Analysis and Machine Intelligence*, PAMI-1(2), 224–227.
//!   `doi:10.1109/TPAMI.1979.4766909`.
//! - Dunn, J. C. (1974). Well-separated clusters and optimal fuzzy partitions. *Journal of
//!   Cybernetics*, 4(1), 95–104. `doi:10.1080/01969727408546059`.
//! - Friedman, H. P., & Rubin, J. (1967). On some invariant criteria for grouping data. *Journal of
//!   the American Statistical Association*, 62(320), 1159–1178. `doi:10.1080/01621459.1967.10500923`.
//! - Krzanowski, W. J., & Lai, Y. T. (1988). A criterion for determining the number of groups in a
//!   data set using sum-of-squares clustering. *Biometrics*, 44(1), 23–34. `doi:10.2307/2531893`.
//! - Milligan, G. W. (1981). A Monte Carlo study of thirty internal criterion measures for cluster
//!   analysis. *Psychometrika*, 46(2), 187–199. `doi:10.1007/BF02293899`.
//! - Hubert, L. J., & Levin, J. R. (1976). A general statistical framework for assessing categorical
//!   clustering in free recall. *Psychological Bulletin*, 83(6), 1072–1080.
//!   `doi:10.1037/0033-2909.83.6.1072`.
//! - Ratkowsky, D. A., & Lance, G. N. (1978). A criterion for determining the number of groups in a
//!   classification. *Australian Computer Journal*, 10, 115–117.
//! - Ball, G. H., & Hall, D. J. (1965). *ISODATA, a novel method of data analysis and pattern
//!   classification*. Stanford Research Institute.
//! - Frey, T., & van Groenewoud, H. (1972). A cluster analysis of the D2 matrix of white spruce
//!   stands in Saskatchewan based on the maximum-minimum principle. *Journal of Ecology*, 60(3),
//!   873–886. `doi:10.2307/2258571`.
//! - Marriott, F. H. C. (1971). Practical problems in a method of cluster analysis. *Biometrics*,
//!   27(3), 501–514. `doi:10.2307/2528592`.
//! - McClain, J. O., & Rao, V. R. (1975). CLUSTISZ: A program to test for the quality of clustering
//!   of a set of objects. *Journal of Marketing Research*, 12(4), 456–460.
//! - Hartigan, J. A. (1975). *Clustering Algorithms*. Wiley.

use std::collections::BTreeMap;

use curvebook_doc::{ClusterDistance, KMeansAlgorithm};
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::{DenseMatrix, sym_eigen};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::{SummaryMoments, pearson_r};
use serde::{Deserialize, Serialize};

use super::clustering::{
    MAX_CLUSTERING_N, compute_mean_silhouette_from_dist, count_distinct_rows, kmeans_pp_from_dist, pairwise_distance_matrix, repair_empty_clusters,
    seed_kmeans_pp_centroids,
};
use super::types::KMeansResult;
use crate::common::validate_finite_slice;
use crate::error::StatsError;

#[inline]
fn sq_euclidean(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| (x - y) * (x - y)).sum()
}

fn validate_kmeans_matrix(rows: &[Vec<f64>], k: usize) -> Result<(usize, usize), StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
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
    if k > count_distinct_rows(rows) {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    Ok((n, d))
}

fn recompute_euclidean_centroids(rows: &[Vec<f64>], assignments: &[usize], k: usize, d: usize) -> (Vec<Vec<f64>>, Vec<usize>) {
    let mut sums = vec![vec![0.0_f64; d]; k];
    let mut counts = vec![0_usize; k];
    for (row, &c) in rows.iter().zip(assignments.iter()) {
        counts[c] += 1;
        for (dst, &v) in sums[c].iter_mut().zip(row.iter()) {
            *dst += v;
        }
    }
    for (c, cent) in sums.iter_mut().enumerate() {
        if counts[c] > 0 {
            let inv = 1.0 / (counts[c] as f64);
            for slot in cent.iter_mut() {
                *slot *= inv;
            }
        }
    }
    (sums, counts)
}

fn run_hartigan_wong(rows: &[Vec<f64>], init_centers: &[Vec<f64>], max_iters: usize) -> (Vec<usize>, Vec<Vec<f64>>) {
    let n = rows.len();
    let d = rows[0].len();
    let k = init_centers.len();
    let mut centroids = init_centers.to_vec();
    let mut ic1 = vec![0_usize; n];
    let mut ic2 = vec![0_usize; n];

    // Step 1: initial closest and second-closest assignment.
    for (i, r) in rows.iter().enumerate() {
        let mut best_c = 0_usize;
        let mut best_d = f64::INFINITY;
        let mut sec_c = 1_usize.min(k - 1);
        let mut sec_d = f64::INFINITY;
        for (c_idx, cent) in centroids.iter().enumerate() {
            let d2 = sq_euclidean(r, cent);
            if d2 < best_d {
                sec_d = best_d;
                sec_c = best_c;
                best_d = d2;
                best_c = c_idx;
            } else if d2 < sec_d {
                sec_d = d2;
                sec_c = c_idx;
            }
        }
        ic1[i] = best_c;
        ic2[i] = sec_c;
    }

    let _ = repair_empty_clusters(rows, &mut ic1, &mut centroids, ClusterDistance::Euclidean);
    let (mut centroids, mut counts) = recompute_euclidean_centroids(rows, &ic1, k, d);
    if k <= 1 {
        return (ic1, centroids);
    }

    // Refresh ic2 after centroid update.
    for (i, r) in rows.iter().enumerate() {
        let l1 = ic1[i];
        let mut sec_c = if l1 == 0 { 1 } else { 0 };
        let mut sec_d = f64::INFINITY;
        for (c_idx, cent) in centroids.iter().enumerate() {
            if c_idx == l1 {
                continue;
            }
            let d2 = sq_euclidean(r, cent);
            if d2 < sec_d {
                sec_d = d2;
                sec_c = c_idx;
            }
        }
        ic2[i] = sec_c;
    }

    let transfer_point = |i: usize, from: usize, to: usize, centroids: &mut [Vec<f64>], counts: &mut [usize]| {
        let nf = counts[from] as f64;
        let nt = counts[to] as f64;
        let row = &rows[i];
        for j in 0..d {
            centroids[from][j] = (nf * centroids[from][j] - row[j]) / (nf - 1.0);
            centroids[to][j] = (nt * centroids[to][j] + row[j]) / (nt + 1.0);
        }
        counts[from] -= 1;
        counts[to] += 1;
    };

    let mut live = vec![true; k];
    for _iter in 0..max_iters.max(1) {
        let mut next_live = vec![false; k];
        let mut optr_moved = false;

        // Optimal-transfer (OPTR) stage (Hartigan & Wong 1979 AS 136).
        for (i, r) in rows.iter().enumerate() {
            let l1 = ic1[i];
            if counts[l1] <= 1 {
                continue;
            }
            let n1 = counts[l1] as f64;
            let r1 = (n1 / (n1 - 1.0)) * sq_euclidean(r, &centroids[l1]);
            let mut best_l2 = l1;
            let mut best_r2 = r1;
            for (l, cent) in centroids.iter().enumerate() {
                if l == l1 {
                    continue;
                }
                if !live[l1] && !live[l] {
                    continue;
                }
                let nl = counts[l] as f64;
                let r2 = (nl / (nl + 1.0)) * sq_euclidean(r, cent);
                if r2 < best_r2 - 1e-14 * r1.max(1.0) {
                    best_r2 = r2;
                    best_l2 = l;
                }
            }
            if best_l2 != l1 {
                transfer_point(i, l1, best_l2, &mut centroids, &mut counts);
                ic1[i] = best_l2;
                ic2[i] = l1;
                next_live[l1] = true;
                next_live[best_l2] = true;
                optr_moved = true;
            }
        }
        if !optr_moved {
            break;
        }
        live = next_live;

        // Quick-transfer (QTRAN) stage between ic1[i] and ic2[i].
        for _qpass in 0..50 {
            let mut q_moved = false;
            for (i, r) in rows.iter().enumerate() {
                let l1 = ic1[i];
                let l2 = ic2[i];
                if l1 == l2 || counts[l1] <= 1 {
                    continue;
                }
                let n1 = counts[l1] as f64;
                let n2 = counts[l2] as f64;
                let r1 = (n1 / (n1 - 1.0)) * sq_euclidean(r, &centroids[l1]);
                let r2 = (n2 / (n2 + 1.0)) * sq_euclidean(r, &centroids[l2]);
                if r2 < r1 - 1e-14 * r1.max(1.0) {
                    transfer_point(i, l1, l2, &mut centroids, &mut counts);
                    ic1[i] = l2;
                    ic2[i] = l1;
                    live[l1] = true;
                    live[l2] = true;
                    q_moved = true;
                }
            }
            if !q_moved {
                break;
            }
        }
    }

    // Recompute exact centroids from final assignments to eliminate incremental drift.
    let (final_centroids, _) = recompute_euclidean_centroids(rows, &ic1, k, d);
    (ic1, final_centroids)
}

fn run_macqueen(rows: &[Vec<f64>], init_centers: &[Vec<f64>], max_iters: usize) -> (Vec<usize>, Vec<Vec<f64>>) {
    let n = rows.len();
    let d = rows[0].len();
    let k = init_centers.len();
    let mut centroids = init_centers.to_vec();
    let mut assignments = vec![0_usize; n];
    let mut counts = vec![0_usize; k];

    // Pass 1 (MacQueen 1967): online assignment with immediate centroid update.
    for (i, r) in rows.iter().enumerate() {
        let mut best_c = 0_usize;
        let mut best_d = f64::INFINITY;
        for (c_idx, cent) in centroids.iter().enumerate() {
            let d2 = sq_euclidean(r, cent);
            if d2 < best_d {
                best_d = d2;
                best_c = c_idx;
            }
        }
        assignments[i] = best_c;
        let nc = counts[best_c] as f64;
        if counts[best_c] == 0 {
            centroids[best_c] = r.clone();
        } else {
            for j in 0..d {
                centroids[best_c][j] = (nc * centroids[best_c][j] + r[j]) / (nc + 1.0);
            }
        }
        counts[best_c] += 1;
    }

    let _ = repair_empty_clusters(rows, &mut assignments, &mut centroids, ClusterDistance::Euclidean);
    let (mut centroids, mut counts) = recompute_euclidean_centroids(rows, &assignments, k, d);

    // Subsequent online sweeps until no point changes cluster.
    for _iter in 1..max_iters.max(1) {
        let mut moved = false;
        for (i, r) in rows.iter().enumerate() {
            let old_c = assignments[i];
            if counts[old_c] <= 1 {
                continue;
            }
            let mut best_c = old_c;
            let mut best_d = sq_euclidean(r, &centroids[old_c]);
            for (c_idx, cent) in centroids.iter().enumerate() {
                if c_idx == old_c {
                    continue;
                }
                let d2 = sq_euclidean(r, cent);
                if d2 < best_d - 1e-14 * best_d.max(1.0) {
                    best_d = d2;
                    best_c = c_idx;
                }
            }
            if best_c != old_c {
                let n_old = counts[old_c] as f64;
                let n_new = counts[best_c] as f64;
                for j in 0..d {
                    centroids[old_c][j] = (n_old * centroids[old_c][j] - r[j]) / (n_old - 1.0);
                    centroids[best_c][j] = (n_new * centroids[best_c][j] + r[j]) / (n_new + 1.0);
                }
                counts[old_c] -= 1;
                counts[best_c] += 1;
                assignments[i] = best_c;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let (final_centroids, _) = recompute_euclidean_centroids(rows, &assignments, k, d);
    (assignments, final_centroids)
}

fn run_lloyd(rows: &[Vec<f64>], init_centers: &[Vec<f64>], max_iters: usize) -> (Vec<usize>, Vec<Vec<f64>>) {
    let n = rows.len();
    let d = rows[0].len();
    let k = init_centers.len();
    let mut centroids = init_centers.to_vec();
    let mut assignments = vec![0_usize; n];
    for _iter in 0..max_iters.max(1) {
        let mut changed = false;
        for (i, r) in rows.iter().enumerate() {
            let mut best_c = 0_usize;
            let mut best_d = f64::INFINITY;
            for (c_idx, cent) in centroids.iter().enumerate() {
                let d2 = sq_euclidean(r, cent);
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
        if repair_empty_clusters(rows, &mut assignments, &mut centroids, ClusterDistance::Euclidean) {
            changed = true;
        }
        let (next_c, _) = recompute_euclidean_centroids(rows, &assignments, k, d);
        centroids = next_c;
        if !changed {
            break;
        }
    }
    (assignments, centroids)
}

/// Run Euclidean *k*-means starting from explicit initial centers using `Lloyd`, `HartiganWong`
/// (Hartigan & Wong 1979 AS 136), or `MacQueen` (MacQueen 1967).
pub fn kmeans_with_centers(
    rows: &[Vec<f64>],
    init_centers: &[Vec<f64>],
    algorithm: KMeansAlgorithm,
    max_iters: usize,
) -> Result<KMeansResult, StatsError> {
    let k = init_centers.len();
    let (n, d) = validate_kmeans_matrix(rows, k)?;
    for c in init_centers {
        if c.len() != d {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: d, actual: c.len() }));
        }
        validate_finite_slice(c)?;
    }
    if k == 1 {
        let assignments = vec![0_usize; n];
        let (centroids, _) = recompute_euclidean_centroids(rows, &assignments, 1, d);
        let wcss: f64 = rows.iter().map(|r| sq_euclidean(r, &centroids[0])).sum();
        return Ok(KMeansResult { k: 1, assignments, centroids, wcss, mean_silhouette: 0.0 });
    }
    let (assignments, centroids) = match algorithm {
        KMeansAlgorithm::Lloyd => run_lloyd(rows, init_centers, max_iters),
        KMeansAlgorithm::HartiganWong => run_hartigan_wong(rows, init_centers, max_iters),
        KMeansAlgorithm::MacQueen => run_macqueen(rows, init_centers, max_iters),
    };
    let wcss: f64 = rows.iter().zip(assignments.iter()).map(|(r, &c)| sq_euclidean(r, &centroids[c])).sum();
    let dist_mat = pairwise_distance_matrix(rows, ClusterDistance::Euclidean)?;
    let mean_silhouette = compute_mean_silhouette_from_dist(&dist_mat, &assignments, k);
    Ok(KMeansResult { k, assignments, centroids, wcss, mean_silhouette })
}

/// Run *k*-means++ clustering with configurable [`ClusterDistance`] and [`KMeansAlgorithm`]
/// (`Lloyd`, `HartiganWong`, or `MacQueen`).
pub fn kmeans_with_algorithm(
    rows: &[Vec<f64>],
    k: usize,
    distance: ClusterDistance,
    algorithm: KMeansAlgorithm,
    seed: u64,
) -> Result<KMeansResult, StatsError> {
    if distance != ClusterDistance::Euclidean || algorithm == KMeansAlgorithm::Lloyd {
        let dist_mat = pairwise_distance_matrix(rows, distance)?;
        return kmeans_pp_from_dist(rows, k, distance, &dist_mat, seed);
    }
    let _ = validate_kmeans_matrix(rows, k)?;
    let init = seed_kmeans_pp_centroids(rows, k, ClusterDistance::Euclidean, seed);
    kmeans_with_centers(rows, &init, algorithm, 100)
}

pub(super) fn kmeans_multistart(
    rows: &[Vec<f64>],
    k: usize,
    distance: ClusterDistance,
    algorithm: KMeansAlgorithm,
    n_starts: usize,
    seed: u64,
) -> Result<KMeansResult, StatsError> {
    let mut best: Option<KMeansResult> = None;
    let starts = n_starts.max(1);
    for s in 0..starts {
        let sub_seed = seed.wrapping_add((s as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let cand = kmeans_with_algorithm(rows, k, distance, algorithm, sub_seed)?;
        if best.as_ref().is_none_or(|b| cand.wcss < b.wcss) {
            best = Some(cand);
        }
    }
    best.ok_or(StatsError::EmptyInput)
}

/// Complete set of internal cluster validity indices evaluated for one partition at cluster count `k`
/// (Charrad et al. 2014). Each index returns `None` (`SkipIndex`) whenever degenerate geometry
/// (singular scatter matrix, zero diameter, single cluster, or missing neighbor `k+1`) prevents a
/// finite well-defined value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMeansValidityIndices {
    /// Number of clusters `k`.
    pub k: usize,
    /// Mean pairwise Euclidean distance among within-cluster observation pairs.
    pub mean_within_dist: Option<f64>,
    /// Mean pairwise Euclidean distance among between-cluster observation pairs.
    pub mean_between_dist: Option<f64>,
    /// TraceW: total within-cluster sum of squares `tr(W)`.
    pub trace_w: Option<f64>,
    /// Ball–Hall (1965): `W_k / k`.
    pub ball_hall: Option<f64>,
    /// Caliński–Harabasz (1974): `(B_k / (k - 1)) / (W_k / (n - k))`.
    pub calinski_harabasz: Option<f64>,
    /// Hubert–Levin (1976) C-Index: `(S_W - S_min) / (S_max - S_min)`.
    pub c_index: Option<f64>,
    /// Davies–Bouldin (1979): average worst-case within-to-between scatter ratio.
    pub davies_bouldin: Option<f64>,
    /// Dunn (1974): minimum inter-cluster separation divided by maximum cluster diameter.
    pub dunn: Option<f64>,
    /// Frey–van Groenewoud (1972): `(s_bar_b(k+1) - s_bar_b(k)) / (s_bar_w(k+1) - s_bar_w(k))`.
    pub frey: Option<f64>,
    /// Friedman–Rubin (1967): `tr(W^{-1} B)`.
    pub friedman: Option<f64>,
    /// Hartigan (1975): `(W_k / W_{k+1} - 1) * (n - k - 1)`.
    pub hartigan: Option<f64>,
    /// Krzanowski–Lai (1988): `|DIFF(k) / DIFF(k+1)|`.
    pub krzanowski_lai: Option<f64>,
    /// Marriott (1971): `k^2 * det(W)`.
    pub marriot: Option<f64>,
    /// McClain–Rao (1975): `(S_W / N_W) / (S_B / N_B)`.
    pub mcclain: Option<f64>,
    /// Milligan (1981) Point-Biserial correlation between pairwise distances and between-cluster indicator.
    pub ptbiserial: Option<f64>,
    /// Ratkowsky–Lance (1978): `sqrt(mean_j(BGSS_j / TSS_j)) / sqrt(k)`.
    pub ratkowsky: Option<f64>,
    /// Friedman–Rubin (1967) determinant ratio: `det(T) / det(W)`.
    pub rubin: Option<f64>,
    /// Rousseeuw (1987) mean silhouette width.
    pub silhouette: Option<f64>,
}

fn finite_opt(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

fn sym_det_and_inv(mat: &DenseMatrix) -> Option<(f64, DenseMatrix)> {
    let p = mat.nrows();
    if p == 0 || mat.ncols() != p {
        return None;
    }
    let eig = sym_eigen(mat).ok()?;
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if !max_ev.is_finite() || max_ev <= 0.0 {
        return None;
    }
    let tol = 1e-14 * max_ev;
    let mut det = 1.0_f64;
    for &ev in &eig.eigenvalues {
        if !ev.is_finite() || ev <= tol {
            return None;
        }
        det *= ev;
    }
    if !det.is_finite() || det <= 1e-300 {
        return None;
    }
    let mut inv_data = vec![0.0_f64; p * p];
    for (k_i, &ev) in eig.eigenvalues.iter().enumerate() {
        let inv_ev = 1.0 / ev;
        for i in 0..p {
            let qi = eig.eigenvectors.get(i, k_i).unwrap_or(0.0);
            for j in i..p {
                let qj = eig.eigenvectors.get(j, k_i).unwrap_or(0.0);
                let delta = qi * qj * inv_ev;
                inv_data[i * p + j] += delta;
                if i != j {
                    inv_data[j * p + i] += delta;
                }
            }
        }
    }
    let inv = DenseMatrix::from_row_major(p, p, inv_data).ok()?;
    Some((det, inv))
}

fn sym_det_only(mat: &DenseMatrix) -> Option<f64> {
    let eig = sym_eigen(mat).ok()?;
    let mut det = 1.0_f64;
    for &ev in &eig.eigenvalues {
        if !ev.is_finite() || ev < 0.0 {
            return None;
        }
        det *= ev;
    }
    det.is_finite().then_some(det)
}

fn compute_davies_bouldin(centroids: &[Vec<f64>], scatters: &[f64], k: usize) -> Option<f64> {
    let mut sum_r = 0.0_f64;
    for i in 0..k {
        let mut max_rij = f64::NEG_INFINITY;
        for j in 0..k {
            if i == j {
                continue;
            }
            let dij = sq_euclidean(&centroids[i], &centroids[j]).sqrt();
            if dij <= 1e-30 {
                return None;
            }
            max_rij = max_rij.max((scatters[i] + scatters[j]) / dij);
        }
        if !max_rij.is_finite() {
            return None;
        }
        sum_r += max_rij;
    }
    finite_opt(sum_r / (k as f64))
}

fn compute_dunn_index(dist_mat: &[Vec<f64>], members: &[Vec<usize>], diams: &[f64]) -> Option<f64> {
    let max_diam = diams.iter().copied().fold(0.0_f64, f64::max);
    if max_diam <= 1e-30 {
        return None;
    }
    let k = members.len();
    let mut min_inter = f64::INFINITY;
    for i in 0..k {
        for j in (i + 1)..k {
            for &u in &members[i] {
                for &v in &members[j] {
                    min_inter = min_inter.min(dist_mat[u][v]);
                }
            }
        }
    }
    if min_inter.is_finite() { finite_opt(min_inter / max_diam) } else { None }
}

struct PairwiseDistanceIndices {
    mean_within: Option<f64>,
    mean_between: Option<f64>,
    c_index: Option<f64>,
    mcclain: Option<f64>,
    ptbiserial: Option<f64>,
}

fn compute_pairwise_distance_indices(dist_mat: &[Vec<f64>], assignments: &[usize]) -> PairwiseDistanceIndices {
    let n = assignments.len();
    let m_pairs = n * (n - 1) / 2;
    let mut all_d = Vec::with_capacity(m_pairs);
    let mut is_between = Vec::with_capacity(m_pairs);
    let mut nw = 0_usize;
    let mut nb = 0_usize;
    let mut sw = 0.0_f64;
    let mut sb = 0.0_f64;
    for i in 0..n {
        for j in (i + 1)..n {
            let d_ij = dist_mat[i][j];
            all_d.push(d_ij);
            if assignments[i] == assignments[j] {
                nw += 1;
                sw += d_ij;
                is_between.push(0.0_f64);
            } else {
                nb += 1;
                sb += d_ij;
                is_between.push(1.0_f64);
            }
        }
    }
    let mean_within = if nw > 0 { finite_opt(sw / (nw as f64)) } else { None };
    let mean_between = if nb > 0 { finite_opt(sb / (nb as f64)) } else { None };
    let c_index = if nw > 0 && nw < m_pairs {
        let mut s_sorted = all_d.clone();
        s_sorted.sort_by(f64::total_cmp);
        let s_min: f64 = s_sorted.iter().take(nw).sum();
        let s_max: f64 = s_sorted.iter().rev().take(nw).sum();
        if s_max - s_min > 1e-30 { finite_opt((sw - s_min) / (s_max - s_min)) } else { None }
    } else {
        None
    };
    let mcclain = match (mean_within, mean_between) {
        (Some(mw), Some(mb)) if mb > 1e-30 => finite_opt(mw / mb),
        _ => None,
    };
    let ptbiserial = if nw > 0 && nb > 0 { pearson_r(&all_d, &is_between).ok().and_then(finite_opt) } else { None };
    PairwiseDistanceIndices { mean_within, mean_between, c_index, mcclain, ptbiserial }
}

fn compute_ratkowsky_index(members: &[Vec<usize>], centroids: &[Vec<f64>], grand_mean: &[f64], tss_j: &[f64], k: usize) -> Option<f64> {
    let mut sum_ratio = 0.0_f64;
    let mut valid_dims = 0_usize;
    for j in 0..grand_mean.len() {
        if tss_j[j] <= 1e-30 {
            continue;
        }
        let bg_j: f64 = members.iter().enumerate().map(|(c, idxs)| (idxs.len() as f64) * (centroids[c][j] - grand_mean[j]).powi(2)).sum();
        sum_ratio += (bg_j / tss_j[j]).clamp(0.0, 1.0);
        valid_dims += 1;
    }
    if valid_dims > 0 { finite_opt((sum_ratio / (valid_dims as f64)).sqrt() / (k as f64).sqrt()) } else { None }
}

struct ScatterDetIndices {
    rubin: Option<f64>,
    friedman: Option<f64>,
    marriot: Option<f64>,
}

fn compute_scatter_det_indices(p: usize, kf: f64, w_data: &[f64], t_data: &[f64]) -> Result<ScatterDetIndices, StatsError> {
    let w_mat = DenseMatrix::from_row_major(p, p, w_data.to_vec())?;
    let t_mat = DenseMatrix::from_row_major(p, p, t_data.to_vec())?;
    let b_data: Vec<f64> = t_data.iter().zip(w_data.iter()).map(|(&tv, &wv)| tv - wv).collect();
    let b_mat = DenseMatrix::from_row_major(p, p, b_data)?;
    Ok(if let Some((det_w, w_inv)) = sym_det_and_inv(&w_mat) {
        let rubin = sym_det_only(&t_mat).and_then(|det_t| finite_opt(det_t / det_w));
        let wb = w_inv.matmul(&b_mat).ok();
        let friedman = wb.and_then(|m| {
            let tr: f64 = (0..p).map(|i| m.get(i, i).unwrap_or(0.0)).sum();
            finite_opt(tr)
        });
        let marriot = finite_opt(kf * kf * det_w);
        ScatterDetIndices { rubin, friedman, marriot }
    } else {
        ScatterDetIndices { rubin: None, friedman: None, marriot: None }
    })
}

/// Evaluate all internal cluster validity indices (Charrad et al. 2014) for a partition at cluster
/// count `k >= 2`, guarding singular or degenerate metrics cleanly via `None` (`SkipIndex`).
///
/// `wcss_by_k` supplies `(k_val, wcss)` entries including neighbors `k - 1` and `k + 1` when
/// available so `Hartigan` and `Krzanowski–Lai` can be computed. `Frey` is populated once all
/// candidate `k` partitions are evaluated (see [`consensus_optimal_k`]).
pub fn compute_kmeans_validity_indices(
    rows: &[Vec<f64>],
    dist_mat: &[Vec<f64>],
    assignments: &[usize],
    k: usize,
    wcss_by_k: &[(usize, f64)],
) -> Result<KMeansValidityIndices, StatsError> {
    let n = rows.len();
    let p = rows.first().map_or(0, Vec::len);
    if n < 3 || p == 0 || k < 2 || k >= n {
        return Err(StatsError::InvalidParameter { name: "validity_k", value: k as f64 });
    }
    if assignments.len() != n || dist_mat.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: assignments.len() });
    }

    let nf = n as f64;
    let kf = k as f64;
    let pf = p as f64;

    let mut grand_mean = vec![0.0_f64; p];
    for r in rows {
        for (dst, &v) in grand_mean.iter_mut().zip(r.iter()) {
            *dst += v / nf;
        }
    }

    let mut t_data = vec![0.0_f64; p * p];
    let mut tss = 0.0_f64;
    let mut tss_j = vec![0.0_f64; p];
    for r in rows {
        for a in 0..p {
            let da = r[a] - grand_mean[a];
            tss += da * da;
            tss_j[a] += da * da;
            for b in a..p {
                let db = r[b] - grand_mean[b];
                t_data[a * p + b] += da * db;
                if a != b {
                    t_data[b * p + a] += da * db;
                }
            }
        }
    }

    let mut members: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (i, &c) in assignments.iter().enumerate() {
        if c >= k {
            return Err(StatsError::InvalidParameter { name: "cluster_assignment", value: c as f64 });
        }
        members[c].push(i);
    }

    let mut centroids = vec![vec![0.0_f64; p]; k];
    let mut w_data = vec![0.0_f64; p * p];
    let mut w_k = 0.0_f64;
    let mut scatters = vec![0.0_f64; k];
    let mut diams = vec![0.0_f64; k];
    let mut any_empty = false;
    for (c, idxs) in members.iter().enumerate() {
        if idxs.is_empty() {
            any_empty = true;
            continue;
        }
        let nc = idxs.len() as f64;
        for &i in idxs {
            for j in 0..p {
                centroids[c][j] += rows[i][j] / nc;
            }
        }
        let mut sum_dist = 0.0_f64;
        for &i in idxs {
            let mut d2 = 0.0_f64;
            for a in 0..p {
                let da = rows[i][a] - centroids[c][a];
                d2 += da * da;
                for b in a..p {
                    let db = rows[i][b] - centroids[c][b];
                    w_data[a * p + b] += da * db;
                    if a != b {
                        w_data[b * p + a] += da * db;
                    }
                }
            }
            w_k += d2;
            sum_dist += d2.sqrt();
        }
        scatters[c] = sum_dist / nc;
        let mut max_d = 0.0_f64;
        for (ii, &u) in idxs.iter().enumerate() {
            for &v in idxs.iter().skip(ii + 1) {
                max_d = max_d.max(dist_mat[u][v]);
            }
        }
        diams[c] = max_d;
    }

    let b_k = (tss - w_k).max(0.0);
    let trace_w = finite_opt(w_k);
    let ball_hall = finite_opt(w_k / kf);
    let calinski_harabasz = if !any_empty && w_k > 1e-30 && n > k { finite_opt((b_k / (kf - 1.0)) / (w_k / ((n - k) as f64))) } else { None };
    let davies_bouldin = if !any_empty { compute_davies_bouldin(&centroids, &scatters, k) } else { None };
    let dunn = if !any_empty { compute_dunn_index(dist_mat, &members, &diams) } else { None };
    let silhouette = if !any_empty { finite_opt(compute_mean_silhouette_from_dist(dist_mat, assignments, k)) } else { None };
    let pw = compute_pairwise_distance_indices(dist_mat, assignments);
    let ratkowsky = if !any_empty { compute_ratkowsky_index(&members, &centroids, &grand_mean, &tss_j, k) } else { None };
    let det_idx = compute_scatter_det_indices(p, kf, &w_data, &t_data)?;
    let (rubin, friedman, marriot) = (det_idx.rubin, det_idx.friedman, det_idx.marriot);

    // Neighbor-WCSS indices: Hartigan and Krzanowski-Lai.
    let lookup_w = |kv: usize| -> Option<f64> { wcss_by_k.iter().find(|&&(kk, _)| kk == kv).map(|&(_, w)| w) };
    let hartigan =
        lookup_w(k + 1).and_then(|w_next| if w_next > 1e-30 && n > k + 1 { finite_opt((w_k / w_next - 1.0) * ((n - k - 1) as f64)) } else { None });
    let krzanowski_lai = match (lookup_w(k - 1), lookup_w(k + 1)) {
        (Some(w_prev), Some(w_next)) => {
            let exp_p = 2.0 / pf;
            let diff_k = ((k - 1) as f64).powf(exp_p) * w_prev - kf.powf(exp_p) * w_k;
            let diff_k1 = kf.powf(exp_p) * w_k - ((k + 1) as f64).powf(exp_p) * w_next;
            if diff_k1.abs() > 1e-30 { finite_opt((diff_k / diff_k1).abs()) } else { None }
        }
        _ => None,
    };

    Ok(KMeansValidityIndices {
        k,
        mean_within_dist: pw.mean_within,
        mean_between_dist: pw.mean_between,
        trace_w,
        ball_hall,
        calinski_harabasz,
        c_index: pw.c_index,
        davies_bouldin,
        dunn,
        frey: None,
        friedman,
        hartigan,
        krzanowski_lai,
        marriot,
        mcclain: pw.mcclain,
        ptbiserial: pw.ptbiserial,
        ratkowsky,
        rubin,
        silhouette,
    })
}

/// Single point on the Tibshirani, Walther & Hastie (2001) Gap statistic curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GapCurvePoint {
    /// Candidate cluster count `k >= 1`.
    pub k: usize,
    /// Empirical `ln(W_k)` where `W_k` is the within-cluster dispersion (`0.5 * WCSS` / `WCSS`, constant shift cancels in Gap).
    pub log_w: f64,
    /// Monte Carlo reference expectation `E^*[ln(W_k^*)]` over `B` uniform bounding-box draws.
    pub expected_log_w: f64,
    /// Gap statistic `Gap(k) = E^*[ln(W_k^*)] - ln(W_k)`.
    pub gap: f64,
    /// Simulation standard error `s_k = sd(ln W_k^*) * sqrt(1 + 1/B)`.
    pub se_sim: f64,
}

/// Within-cluster Euclidean dispersion `W_k = sum_{c=1}^k (1 / (2 n_c)) sum_{i < j in C_c} ||x_i - x_j||_2`
/// matching Tibshirani, Walther & Hastie (2001) and R `cluster::clusGap`.
fn gap_within_dispersion(rows: &[Vec<f64>], assignments: &[usize], k: usize) -> f64 {
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (i, &c) in assignments.iter().enumerate() {
        if c < k {
            members[c].push(i);
        }
    }
    let mut total = 0.0_f64;
    for idxs in &members {
        let nc = idxs.len();
        if nc < 2 {
            continue;
        }
        let mut sum_d = 0.0_f64;
        for (a_pos, &i) in idxs.iter().enumerate() {
            for &j in idxs.iter().skip(a_pos + 1) {
                sum_d += sq_euclidean(&rows[i], &rows[j]).sqrt();
            }
        }
        total += sum_d / (2.0 * (nc as f64));
    }
    total
}

/// Select optimal `k` from a Gap curve using the `firstSEmax` rule (Tibshirani et al. 2001;
/// Maechler et al. 2012): find the first local maximum `k_f` where `Gap(k_f) >= Gap(k_f + 1)`,
/// then return the smallest `k <= k_f` satisfying `Gap(k) >= Gap(k_f) - s_{k_f}`.
fn gap_select_first_se_max(curve: &[GapCurvePoint], start_idx: usize) -> usize {
    if curve.is_empty() {
        return 1;
    }
    let last_idx = curve.len() - 1;
    let lo = start_idx.min(last_idx);
    let mut first_local_max = last_idx;
    for i in lo..last_idx {
        if curve[i].gap >= curve[i + 1].gap {
            first_local_max = i;
            break;
        }
    }
    let threshold = curve[first_local_max].gap - curve[first_local_max].se_sim;
    for pt in &curve[lo..=first_local_max] {
        if pt.gap >= threshold {
            return pt.k;
        }
    }
    curve[first_local_max].k
}

/// Compute the Tibshirani, Walther & Hastie (2001) Gap statistic across `k = 1..=k_max` using
/// seeded uniform bounding-box reference datasets (`DeterministicRng`), returning `(curve, k_opt)`
/// where `k_opt` applies the `firstSEmax` criterion.
pub fn compute_gap_statistic(
    rows: &[Vec<f64>],
    k_max: usize,
    b_refs: usize,
    algorithm: KMeansAlgorithm,
    seed: u64,
) -> Result<(Vec<GapCurvePoint>, usize), StatsError> {
    let n = rows.len();
    let p = rows.first().map_or(0, Vec::len);
    if n < 3 || p == 0 || k_max < 2 || k_max >= n {
        return Err(StatsError::InvalidParameter { name: "gap_k_max", value: k_max as f64 });
    }
    let b_count = b_refs.max(10);
    let mut mins = vec![f64::INFINITY; p];
    let mut maxs = vec![f64::NEG_INFINITY; p];
    for r in rows {
        if r.len() != p {
            return Err(StatsError::DimensionMismatch { expected: p, actual: r.len() });
        }
        for (j, &v) in r.iter().enumerate() {
            if !v.is_finite() {
                return Err(StatsError::InvalidParameter { name: "gap_input", value: v });
            }
            mins[j] = mins[j].min(v);
            maxs[j] = maxs[j].max(v);
        }
    }

    // Observed log(W_k) for k = 1..=k_max.
    let mut obs_log_w = Vec::with_capacity(k_max);
    let all_zero = vec![0_usize; n];
    let w1 = gap_within_dispersion(rows, &all_zero, 1);
    if !w1.is_finite() {
        return Err(StatsError::InvalidParameter { name: "gap_dispersion", value: w1 });
    }
    obs_log_w.push(w1.max(1e-300).ln());
    for k in 2..=k_max {
        let fit = kmeans_multistart(rows, k, ClusterDistance::Euclidean, algorithm, 8, seed.wrapping_add(k as u64))?;
        let wk = gap_within_dispersion(rows, &fit.assignments, k);
        if !wk.is_finite() {
            return Err(StatsError::InvalidParameter { name: "gap_dispersion", value: wk });
        }
        obs_log_w.push(wk.max(1e-300).ln());
    }

    let mut rng = DeterministicRng::from_seed(seed ^ 0x6A09_E667_F3BC_C908);
    let mut ref_log_ws = vec![Vec::with_capacity(b_count); k_max];
    for b_idx in 0..b_count {
        let mut ref_rows = vec![vec![0.0_f64; p]; n];
        for row in &mut ref_rows {
            for j in 0..p {
                let lo = mins[j];
                let hi = maxs[j];
                row[j] = if hi > lo {
                    let u = rng.next_open_f64();
                    lo * (1.0 - u) + hi * u
                } else {
                    lo
                };
            }
        }
        let rw1 = gap_within_dispersion(&ref_rows, &all_zero, 1);
        ref_log_ws[0].push(rw1.max(1e-300).ln());
        for k in 2..=k_max {
            let sub_seed = seed.wrapping_add(((b_idx + 1) * 131 + k) as u64);
            let rw = kmeans_multistart(&ref_rows, k, ClusterDistance::Euclidean, algorithm, 4, sub_seed)
                .map_or(rw1, |f| gap_within_dispersion(&ref_rows, &f.assignments, k));
            ref_log_ws[k - 1].push(rw.max(1e-300).ln());
        }
    }

    let se_mult = (1.0 + 1.0 / (b_count as f64)).sqrt();
    let mut curve = Vec::with_capacity(k_max);
    for k in 1..=k_max {
        let m = SummaryMoments::from_slice(&ref_log_ws[k - 1])?;
        let exp_lw = m.mean;
        let sd_lw = m.sample_sd().unwrap_or(0.0);
        let se_sim = sd_lw * se_mult;
        let lw = obs_log_w[k - 1];
        curve.push(GapCurvePoint { k, log_w: lw, expected_log_w: exp_lw, gap: exp_lw - lw, se_sim });
    }

    let k_opt = gap_select_first_se_max(&curve, 0);
    Ok((curve, k_opt))
}

/// Per-index vote record in the 17-index consensus optimal-*k* procedure (Charrad et al. 2014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusIndexVote {
    /// Name of the validity index (e.g. `"Calinski-Harabasz"`, `"Gap"`, `"Rubin"`).
    pub index_name: String,
    /// Cluster count `k` selected by this index (`None` when skipped due to degeneracy/insufficient range).
    pub voted_k: Option<usize>,
    /// Reason this index returned `SkipIndex` (`None` when `voted_k.is_some()`).
    pub skip_reason: Option<String>,
}

/// Complete output of the 17-index consensus optimal-*k* procedure (Charrad et al. 2014;
/// Tibshirani et al. 2001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusOptimalKResult {
    /// Minimum candidate `k` evaluated (`>= 2`).
    pub k_min: usize,
    /// Maximum candidate `k` evaluated.
    pub k_max: usize,
    /// Majority-vote consensus optimal cluster count `k` (ties broken by smallest `k`).
    pub optimal_k: usize,
    /// Sorted `(k, vote_count)` tally across all non-skipped indices.
    pub vote_counts: Vec<(usize, usize)>,
    /// Individual vote or `SkipIndex` record for each of the 17 validity indices.
    pub index_votes: Vec<ConsensusIndexVote>,
    /// Full validity-index profiles for each candidate `k` in `k_min..=k_max`.
    pub metrics_by_k: Vec<KMeansValidityIndices>,
    /// Gap statistic curve across `k = 1..=k_max` (Tibshirani et al. 2001).
    pub gap_curve: Vec<GapCurvePoint>,
    /// Fitted *k*-means partition at `optimal_k`.
    pub best_fit: KMeansResult,
}

fn vote_argmax(pairs: &[(usize, Option<f64>)], name: &'static str) -> ConsensusIndexVote {
    let mut best: Option<(usize, f64)> = None;
    for &(k, val) in pairs {
        if let Some(v) = val
            && v.is_finite()
            && best.is_none_or(|(_, bv)| v > bv + 1e-12)
        {
            best = Some((k, v));
        }
    }
    match best {
        Some((k, _)) => ConsensusIndexVote { index_name: String::from(name), voted_k: Some(k), skip_reason: None },
        None => ConsensusIndexVote { index_name: String::from(name), voted_k: None, skip_reason: Some(String::from("SkipIndex: no finite values")) },
    }
}

fn vote_argmin(pairs: &[(usize, Option<f64>)], name: &'static str) -> ConsensusIndexVote {
    let mut best: Option<(usize, f64)> = None;
    for &(k, val) in pairs {
        if let Some(v) = val
            && v.is_finite()
            && best.is_none_or(|(_, bv)| v < bv - 1e-12)
        {
            best = Some((k, v));
        }
    }
    match best {
        Some((k, _)) => ConsensusIndexVote { index_name: String::from(name), voted_k: Some(k), skip_reason: None },
        None => ConsensusIndexVote { index_name: String::from(name), voted_k: None, skip_reason: Some(String::from("SkipIndex: no finite values")) },
    }
}

fn vote_max_first_drop(pairs: &[(usize, Option<f64>)], name: &'static str) -> ConsensusIndexVote {
    // Maximize val(k-1) - val(k) over consecutive entries, assigning vote to k (or k_prev for Hartigan).
    let mut best: Option<(usize, f64)> = None;
    for w in pairs.windows(2) {
        if let (Some(v_prev), Some(v_cur)) = (w[0].1, w[1].1) {
            let drop = v_prev - v_cur;
            if drop.is_finite() && best.is_none_or(|(_, bd)| drop > bd + 1e-12) {
                best = Some((w[1].0, drop));
            }
        }
    }
    match best {
        Some((k, _)) => ConsensusIndexVote { index_name: String::from(name), voted_k: Some(k), skip_reason: None },
        None => ConsensusIndexVote {
            index_name: String::from(name),
            voted_k: None,
            skip_reason: Some(String::from("SkipIndex: insufficient finite range")),
        },
    }
}

fn vote_max_first_rise(pairs: &[(usize, Option<f64>)], name: &'static str) -> ConsensusIndexVote {
    let mut best: Option<(usize, f64)> = None;
    for w in pairs.windows(2) {
        if let (Some(v_prev), Some(v_cur)) = (w[0].1, w[1].1) {
            let rise = v_cur - v_prev;
            if rise.is_finite() && best.is_none_or(|(_, br)| rise > br + 1e-12) {
                best = Some((w[1].0, rise));
            }
        }
    }
    match best {
        Some((k, _)) => ConsensusIndexVote { index_name: String::from(name), voted_k: Some(k), skip_reason: None },
        None => ConsensusIndexVote {
            index_name: String::from(name),
            voted_k: None,
            skip_reason: Some(String::from("SkipIndex: insufficient finite range")),
        },
    }
}

fn vote_second_diff(pairs: &[(usize, Option<f64>)], name: &'static str, maximize: bool) -> ConsensusIndexVote {
    let mut best: Option<(usize, f64)> = None;
    for w in pairs.windows(3) {
        if let (Some(v_prev), Some(v_cur), Some(v_next)) = (w[0].1, w[1].1, w[2].1) {
            let d2 = (v_next - v_cur) - (v_cur - v_prev);
            if !d2.is_finite() {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, bd)) => {
                    if maximize {
                        d2 > bd + 1e-12
                    } else {
                        d2 < bd - 1e-12
                    }
                }
            };
            if better {
                best = Some((w[1].0, d2));
            }
        }
    }
    match best {
        Some((k, _)) => ConsensusIndexVote { index_name: String::from(name), voted_k: Some(k), skip_reason: None },
        None => ConsensusIndexVote {
            index_name: String::from(name),
            voted_k: None,
            skip_reason: Some(String::from("SkipIndex: requires >= 3 finite k levels")),
        },
    }
}

/// Evaluate all 17 consensus optimal-*k* indices (Charrad et al. 2014; Tibshirani et al. 2001)
/// over `k_min..=k_max`, returning per-index votes with `SkipIndex` finite guards, majority-vote
/// tie-breaking (smallest `k` on ties), per-`k` metrics, Gap curve, and the optimal partition.
pub fn consensus_optimal_k(
    rows: &[Vec<f64>],
    k_min: usize,
    k_max: usize,
    distance: ClusterDistance,
    algorithm: KMeansAlgorithm,
    seed: u64,
) -> Result<ConsensusOptimalKResult, StatsError> {
    let n = rows.len();
    let p = rows.first().map_or(0, Vec::len);
    if n < 4 || p == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "consensus optimal-k clustering", required: 4, actual: n });
    }
    let n_distinct = count_distinct_rows(rows);
    let eff_min = k_min.max(2);
    let eff_max = k_max.min(n - 1).min(n_distinct);
    if eff_min > eff_max {
        return Err(StatsError::InvalidParameter { name: "k_sweep_max", value: k_max as f64 });
    }

    let dist_mat = pairwise_distance_matrix(rows, ClusterDistance::Euclidean)?;
    let grand_mean: Vec<f64> = (0..p).map(|j| rows.iter().map(|r| r[j]).sum::<f64>() / (n as f64)).collect();
    let w1: f64 = rows.iter().map(|r| sq_euclidean(r, &grand_mean)).sum();

    // Fit partitions for k in eff_min..=eff_max (plus eff_max + 1 if possible for Hartigan/KL).
    let upper_fit_k = (eff_max + 1).min(n - 1).min(n_distinct);
    let mut fits: BTreeMap<usize, KMeansResult> = BTreeMap::new();
    let mut wcss_by_k: Vec<(usize, f64)> = vec![(1, w1)];
    for k in eff_min..=upper_fit_k {
        let fit = kmeans_multistart(rows, k, distance, algorithm, 8, seed.wrapping_add((k as u64) * 31))?;
        let w_euclid: f64 = rows.iter().zip(fit.assignments.iter()).map(|(r, &c)| sq_euclidean(r, &fit.centroids[c])).sum();
        wcss_by_k.push((k, w_euclid));
        fits.insert(k, fit);
    }

    let mut metrics_by_k = Vec::with_capacity(eff_max - eff_min + 1);
    for k in eff_min..=eff_max {
        let Some(fit) = fits.get(&k) else {
            continue;
        };
        metrics_by_k.push(compute_kmeans_validity_indices(rows, &dist_mat, &fit.assignments, k, &wcss_by_k)?);
    }

    // Populate Frey & van Groenewoud (1972) index using adjacent mean between/within distances.
    for i in 0..metrics_by_k.len().saturating_sub(1) {
        let cur_b = metrics_by_k[i].mean_between_dist;
        let cur_w = metrics_by_k[i].mean_within_dist;
        let nxt_b = metrics_by_k[i + 1].mean_between_dist;
        let nxt_w = metrics_by_k[i + 1].mean_within_dist;
        if let (Some(cb), Some(cw), Some(nb), Some(nw)) = (cur_b, cur_w, nxt_b, nxt_w) {
            let denom = nw - cw;
            if denom.abs() > 1e-30 {
                metrics_by_k[i].frey = finite_opt((nb - cb) / denom);
            }
        }
    }

    // Gap statistic (Tibshirani et al. 2001) with deterministic reference draws.
    let (gap_curve, gap_k_raw) = compute_gap_statistic(rows, eff_max, 25, algorithm, seed)?;
    let gap_vote = if gap_k_raw >= eff_min && gap_k_raw <= eff_max {
        ConsensusIndexVote { index_name: String::from("Gap"), voted_k: Some(gap_k_raw), skip_reason: None }
    } else {
        let chosen = gap_select_first_se_max(&gap_curve, eff_min - 1);
        ConsensusIndexVote { index_name: String::from("Gap"), voted_k: Some(chosen), skip_reason: None }
    };

    // Frey & van Groenewoud (1972) / Charrad et al. (2014, NbClust Table 2):
    // Moving downward from k_max toward k_min, choose the cluster level immediately before Frey
    // drops below 1.0 (i.e. k_last_lt_one + 1, where k_last_lt_one is the largest k with Frey(k) < 1.0).
    let frey_vote = {
        let mut last_lt_one: Option<usize> = None;
        for m in &metrics_by_k {
            if let Some(fv) = m.frey
                && fv < 1.0
            {
                last_lt_one = Some(m.k);
            }
        }
        match last_lt_one.map(|k| k + 1).filter(|&k| k <= eff_max) {
            Some(k) => ConsensusIndexVote { index_name: String::from("Frey"), voted_k: Some(k), skip_reason: None },
            None => ConsensusIndexVote {
                index_name: String::from("Frey"),
                voted_k: None,
                skip_reason: Some(String::from("SkipIndex: Frey >= 1.0 for all evaluated k (suggests k = 1)")),
            },
        }
    };

    let extract = |f: fn(&KMeansValidityIndices) -> Option<f64>| -> Vec<(usize, Option<f64>)> { metrics_by_k.iter().map(|m| (m.k, f(m))).collect() };
    let index_votes = vec![
        vote_max_first_drop(&extract(|m| m.ball_hall), "Ball-Hall"),
        vote_argmax(&extract(|m| m.calinski_harabasz), "Calinski-Harabasz"),
        vote_argmin(&extract(|m| m.c_index), "C-Index"),
        vote_argmin(&extract(|m| m.davies_bouldin), "Davies-Bouldin"),
        vote_argmax(&extract(|m| m.dunn), "Dunn"),
        frey_vote,
        vote_max_first_rise(&extract(|m| m.friedman), "Friedman"),
        gap_vote,
        vote_max_first_drop(&extract(|m| m.hartigan), "Hartigan"),
        vote_argmax(&extract(|m| m.krzanowski_lai), "Krzanowski-Lai"),
        vote_second_diff(&extract(|m| m.marriot), "Marriot", true),
        vote_argmin(&extract(|m| m.mcclain), "McClain"),
        vote_argmax(&extract(|m| m.ptbiserial), "Ptbiserial"),
        vote_argmax(&extract(|m| m.ratkowsky), "Ratkowsky"),
        vote_second_diff(&extract(|m| m.rubin), "Rubin", false),
        vote_argmax(&extract(|m| m.silhouette), "Silhouette"),
        vote_second_diff(&extract(|m| m.trace_w), "TraceW", true),
    ];

    let mut tally: BTreeMap<usize, usize> = BTreeMap::new();
    for v in &index_votes {
        if let Some(k) = v.voted_k {
            *tally.entry(k).or_insert(0) += 1;
        }
    }
    // Majority vote; break ties in favor of smallest k (parsimony).
    let optimal_k = tally.iter().max_by(|(ka, va), (kb, vb)| va.cmp(vb).then_with(|| kb.cmp(ka))).map(|(&k, _)| k).unwrap_or(eff_min);
    let vote_counts: Vec<(usize, usize)> = tally.into_iter().collect();
    let best_fit = fits.remove(&optimal_k).ok_or(StatsError::EmptyInput)?;

    Ok(ConsensusOptimalKResult { k_min: eff_min, k_max: eff_max, optimal_k, vote_counts, index_votes, metrics_by_k, gap_curve, best_fit })
}

/// Per-feature one-way ANOVA summary across *k*-means clusters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMeansFeatureAnovaRow {
    /// Feature variable label.
    pub variable: String,
    /// Between-cluster sum of squares.
    pub ss_between: f64,
    /// Between-cluster degrees of freedom `k - 1`.
    pub df_between: f64,
    /// Between-cluster mean square.
    pub ms_between: f64,
    /// Within-cluster residual sum of squares.
    pub ss_within: f64,
    /// Within-cluster degrees of freedom `n - k`.
    pub df_within: f64,
    /// Within-cluster mean square.
    pub ms_within: f64,
    /// One-way ANOVA *F* statistic `ms_between / ms_within`.
    pub f_statistic: f64,
    /// Upper-tail *F* *P* value.
    pub p_value: f64,
    /// Variance explained `R^2 = ss_between / (ss_between + ss_within)`.
    pub r_squared: f64,
}

/// Compute a feature-by-feature one-way ANOVA summary table across *k*-means clusters.
pub fn kmeans_feature_anova(
    rows: &[Vec<f64>],
    var_labels: &[String],
    assignments: &[usize],
    k: usize,
) -> Result<Vec<KMeansFeatureAnovaRow>, StatsError> {
    let n = rows.len();
    let p = var_labels.len();
    if k < 2 {
        return Err(StatsError::InvalidParameter { name: "k", value: k as f64 });
    }
    if p == 0 {
        return Err(StatsError::EmptyInput);
    }
    if assignments.len() != n {
        return Err(StatsError::DimensionMismatch { expected: n, actual: assignments.len() });
    }
    if n <= k {
        return Err(StatsError::InsufficientSampleSize { context: "k-means cluster ANOVA", required: k + 1, actual: n });
    }
    for row in rows {
        if row.len() != p {
            return Err(StatsError::DimensionMismatch { expected: p, actual: row.len() });
        }
        if let Some(&bad) = row.iter().find(|v| !v.is_finite()) {
            return Err(StatsError::InvalidParameter { name: "kmeans_anova_value", value: bad });
        }
    }
    for &c in assignments {
        if c >= k {
            return Err(StatsError::InvalidParameter { name: "cluster_assignment", value: c as f64 });
        }
    }
    let df_b = (k - 1) as f64;
    let df_w = (n - k) as f64;
    let mut out = Vec::with_capacity(p);
    for (j, label) in var_labels.iter().enumerate() {
        let mut grand_sum = 0.0_f64;
        let mut c_sums = vec![0.0_f64; k];
        let mut c_counts = vec![0_usize; k];
        for (i, row) in rows.iter().enumerate() {
            let v = row[j];
            let c = assignments[i];
            grand_sum += v;
            c_sums[c] += v;
            c_counts[c] += 1;
        }
        let grand_mean = grand_sum / (n as f64);
        let c_means: Vec<f64> = c_sums.iter().zip(c_counts.iter()).map(|(&s, &cnt)| if cnt > 0 { s / (cnt as f64) } else { grand_mean }).collect();
        let ss_b: f64 = c_means.iter().zip(c_counts.iter()).map(|(&m, &cnt)| (cnt as f64) * (m - grand_mean).powi(2)).sum();
        let ss_w: f64 = rows.iter().zip(assignments.iter()).map(|(row, &c)| (row[j] - c_means[c]).powi(2)).sum();
        let ms_b = ss_b / df_b;
        let ms_w = ss_w / df_w;
        let (f_val, p_val) = if ms_w > 0.0 {
            let f = ms_b / ms_w;
            (f, f_sf(f, df_b, df_w)?)
        } else if ms_b > 0.0 {
            (f64::INFINITY, 0.0)
        } else {
            (0.0, 1.0)
        };
        let r2 = if ss_b + ss_w > 0.0 { ss_b / (ss_b + ss_w) } else { 0.0 };
        out.push(KMeansFeatureAnovaRow {
            variable: label.clone(),
            ss_between: ss_b,
            df_between: df_b,
            ms_between: ms_b,
            ss_within: ss_w,
            df_within: df_w,
            ms_within: ms_w,
            f_statistic: f_val,
            p_value: p_val,
            r_squared: r2,
        });
    }
    Ok(out)
}
