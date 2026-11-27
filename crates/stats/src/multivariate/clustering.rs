//! *k*-means++ clustering, pairwise dissimilarity matrices, silhouette widths, and agglomerative
//! hierarchical clustering (nearest-neighbor chain and heap Lance–Williams linkage, cophenetic
//! matrices, and dendrogram cutting).
//!
//! References:
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
//! - Murtagh, F. (1983). A survey of recent advances in hierarchical clustering algorithms. *The
//!   Computer Journal*, 26(4), 354-359.
//! - Murtagh, F., & Legendre, P. (2014). Ward's hierarchical agglomerative clustering method: Which
//!   algorithms implement Ward's criterion? *Journal of Classification*, 31(3), 274-295.
//!   `doi:10.1007/s00357-014-9161-z`.
//! - Dixon, J. K. (1979). Pattern recognition with partly missing data. *IEEE Transactions on
//!   Systems, Man, and Cybernetics*, 9(10), 617-621.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use curvebook_doc::{ClusterDistance, ClusterLinkage};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::pearson_r;

use super::types::{DendrogramMerge, KMeansResult, LinkageMethod};
use crate::common::{quantile_sorted, validate_finite_slice};
use crate::error::StatsError;

pub(super) fn count_distinct_rows(rows: &[Vec<f64>]) -> usize {
    let mut keys: Vec<Vec<u64>> = rows.iter().map(|row| row.iter().map(|&v| (if v == 0.0 { 0.0 } else { v }).to_bits()).collect()).collect();
    keys.sort();
    keys.dedup();
    keys.len()
}

pub(super) fn count_distinct_by_dist(dist_mat: &[Vec<f64>]) -> usize {
    let n = dist_mat.len();
    let mut visited = vec![false; n];
    let mut classes = 0_usize;
    for i in 0..n {
        if visited[i] {
            continue;
        }
        classes += 1;
        for (j, slot) in visited.iter_mut().enumerate().skip(i) {
            if !*slot && dist_mat[i][j] <= 0.0 {
                *slot = true;
            }
        }
    }
    classes
}

fn pick_unassigned_furthest_point(
    rows: &[Vec<f64>],
    assignments: &[usize],
    counts: &[usize],
    centroids: &[Vec<f64>],
    metric: ClusterDistance,
) -> Option<usize> {
    let mut best_idx = None;
    let mut best_dist = -1.0_f64;
    for (i, row) in rows.iter().enumerate() {
        let cur_c = assignments[i];
        if counts[cur_c] <= 1 {
            continue;
        }
        let d = pairwise_distance_unchecked(row, &centroids[cur_c], metric);
        if d > best_dist {
            best_dist = d;
            best_idx = Some(i);
        }
    }
    best_idx
}

pub(super) fn seed_kmeans_pp_centroids(rows: &[Vec<f64>], k: usize, distance: ClusterDistance, seed: u64) -> Vec<Vec<f64>> {
    let n = rows.len();
    let mut rng = DeterministicRng::from_seed(seed);
    let mut centroids = Vec::with_capacity(k);
    let mut chosen_idx = Vec::with_capacity(k);
    let first = rng.gen_index(n).unwrap_or(0);
    chosen_idx.push(first);
    centroids.push(rows[first].clone());

    let mut min_d2 = vec![f64::INFINITY; n];
    for _ in 1..k {
        let last = centroids.last().map_or(&rows[0], |c| c);
        let mut total_d2 = 0.0_f64;
        for (i, r) in rows.iter().enumerate() {
            let dv = pairwise_distance_unchecked(r, last, distance);
            let d2 = dv * dv;
            if d2 < min_d2[i] {
                min_d2[i] = d2;
            }
            if chosen_idx.contains(&i) {
                min_d2[i] = 0.0;
            }
            total_d2 += min_d2[i];
        }
        let chosen = if total_d2 <= 0.0 {
            (0..n).find(|i| !chosen_idx.contains(i)).unwrap_or_else(|| rng.gen_index(n).unwrap_or(0))
        } else {
            let target = rng.next_open_f64() * total_d2;
            let mut acc = 0.0_f64;
            let mut pick = None;
            for (i, &w) in min_d2.iter().enumerate() {
                acc += w;
                if w > 0.0 && acc >= target {
                    pick = Some(i);
                    break;
                }
            }
            pick.or_else(|| (0..n).find(|i| min_d2[*i] > 0.0)).unwrap_or(n - 1)
        };
        chosen_idx.push(chosen);
        centroids.push(rows[chosen].clone());
    }
    centroids
}

fn recompute_cluster_centroid(rows: &[Vec<f64>], assignments: &[usize], c_idx: usize, d: usize, distance: ClusterDistance) -> Vec<f64> {
    let members: Vec<&Vec<f64>> = rows.iter().zip(assignments.iter()).filter(|(_, a)| **a == c_idx).map(|(r, _)| r).collect();
    if members.is_empty() {
        return vec![0.0; d];
    }
    match distance {
        ClusterDistance::Manhattan => {
            let mut cent = vec![0.0_f64; d];
            for (j, slot) in cent.iter_mut().enumerate() {
                let mut col: Vec<f64> = members.iter().map(|r| r[j]).collect();
                col.sort_by(f64::total_cmp);
                *slot = quantile_sorted(&col, 0.5).unwrap_or(0.0);
            }
            cent
        }
        ClusterDistance::Euclidean => {
            let mut sum_vec = vec![0.0_f64; d];
            for r in &members {
                for j in 0..d {
                    sum_vec[j] += r[j];
                }
            }
            let count_f = members.len() as f64;
            for slot in &mut sum_vec {
                *slot /= count_f;
            }
            sum_vec
        }
        ClusterDistance::Cosine | ClusterDistance::Pearson => {
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
            sum_vec
        }
    }
}

pub(super) fn repair_empty_clusters(rows: &[Vec<f64>], assignments: &mut [usize], centroids: &mut [Vec<f64>], distance: ClusterDistance) -> bool {
    let k = centroids.len();
    let d = rows.first().map_or(0, Vec::len);
    let mut counts = vec![0_usize; k];
    for &c_idx in assignments.iter() {
        counts[c_idx] += 1;
    }
    let mut repaired = false;
    for c_idx in 0..k {
        if counts[c_idx] == 0
            && let Some(donor_obs) = pick_unassigned_furthest_point(rows, assignments, &counts, centroids, distance)
        {
            let old_c = assignments[donor_obs];
            counts[old_c] -= 1;
            counts[c_idx] += 1;
            assignments[donor_obs] = c_idx;
            centroids[c_idx] = match distance {
                ClusterDistance::Euclidean | ClusterDistance::Manhattan => rows[donor_obs].clone(),
                ClusterDistance::Cosine | ClusterDistance::Pearson => transform_row_for_centroid(&rows[donor_obs], distance),
            };
            centroids[old_c] = recompute_cluster_centroid(rows, assignments, old_c, d, distance);
            repaired = true;
        }
    }
    repaired
}

/// *k*-means++ clustering (`DeterministicRng::from_seed`) returning cluster assignments,
/// centroids, total within-cluster sum of squares (WCSS), and mean silhouette score.
///
/// `D^2` seeding of Arthur & Vassilvitskii (2007), Lloyd (1982) assignment/update iterations, and
/// the mean silhouette of Rousseeuw (1987). Enforces `rows.len() <= MAX_CLUSTERING_N` consistently
/// with [`kmeans_pp_with_distance`] and [`pairwise_distance_matrix`] so silhouette evaluation never
/// silently falls back to `0.0`.
pub fn kmeans_pp(rows: &[Vec<f64>], k: usize, seed: u64) -> Result<KMeansResult, StatsError> {
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
    let n_distinct = count_distinct_rows(rows);
    if k > n_distinct {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }

    let sq_dist = |a: &[f64], b: &[f64]| -> f64 { a.iter().zip(b.iter()).map(|(x, y)| (x - y).powi(2)).sum() };
    let mut centroids = seed_kmeans_pp_centroids(rows, k, ClusterDistance::Euclidean, seed);
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
        if repair_empty_clusters(rows, &mut assignments, &mut centroids, ClusterDistance::Euclidean) {
            changed = true;
        }
        for (c_idx, cent) in centroids.iter_mut().enumerate() {
            *cent = recompute_cluster_centroid(rows, &assignments, c_idx, d, ClusterDistance::Euclidean);
        }
        if !changed {
            break;
        }
    }

    let mut wcss = 0.0_f64;
    for (r, &c_idx) in rows.iter().zip(assignments.iter()) {
        wcss += sq_dist(r, &centroids[c_idx]);
    }
    let mean_silhouette = compute_mean_silhouette(rows, &assignments, k)?;

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
pub(super) fn pairwise_distance_unchecked(a: &[f64], b: &[f64], metric: ClusterDistance) -> f64 {
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

/// Maximum observation count accepted by *k*-means++, hierarchical clustering, and pairwise distance matrices.
pub const MAX_CLUSTERING_N: usize = 2_000;
pub(super) const MAX_CLUSTER_MATRIX_TAB_N: usize = 500;
pub(super) const MAX_CLUSTER_K_SWEEP: usize = 50;

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

pub(super) fn kmeans_pp_from_dist(
    rows: &[Vec<f64>],
    k: usize,
    distance: ClusterDistance,
    dist_mat: &[Vec<f64>],
    seed: u64,
) -> Result<KMeansResult, StatsError> {
    let n = rows.len();
    let d = rows.first().map_or(0, Vec::len);
    if n < 2 || d == 0 || k == 0 || k > n {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    if dist_mat.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: dist_mat.len() }));
    }
    let n_distinct = count_distinct_rows(rows).min(count_distinct_by_dist(dist_mat));
    if k > n_distinct {
        return Err(StatsError::InvalidParameter { name: "k_means_k", value: k as f64 });
    }
    let sq_dist = |a: &[f64], b: &[f64]| -> f64 {
        let v = pairwise_distance_unchecked(a, b, distance);
        v * v
    };

    let mut centroids = seed_kmeans_pp_centroids(rows, k, distance, seed);
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
        if repair_empty_clusters(rows, &mut assignments, &mut centroids, distance) {
            changed = true;
        }
        for (c_idx, cent) in centroids.iter_mut().enumerate() {
            *cent = recompute_cluster_centroid(rows, &assignments, c_idx, d, distance);
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

fn compute_mean_silhouette(rows: &[Vec<f64>], assignments: &[usize], k: usize) -> Result<f64, StatsError> {
    let dist_mat = pairwise_distance_matrix(rows, ClusterDistance::Euclidean)?;
    Ok(compute_mean_silhouette_from_dist(&dist_mat, assignments, k))
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

#[derive(Clone, Copy)]
struct TopoMergeKey {
    dist: f64,
    orig_idx: usize,
}

impl PartialEq for TopoMergeKey {
    fn eq(&self, other: &Self) -> bool {
        self.dist.to_bits() == other.dist.to_bits() && self.orig_idx == other.orig_idx
    }
}

impl Eq for TopoMergeKey {}

impl Ord for TopoMergeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        other.dist.total_cmp(&self.dist).then_with(|| other.orig_idx.cmp(&self.orig_idx))
    }
}

impl PartialOrd for TopoMergeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Reorder dendrogram merges in ascending height using Kahn's topological min-heap sort keyed by
/// `(distance, orig_idx)` so parent merges can never precede their children even under float ties.
fn reorder_merges_topologically(raw_merges: Vec<DendrogramMerge>) -> Vec<DendrogramMerge> {
    let m = raw_merges.len();
    let mut in_deg = vec![0_u8; m];
    let mut parent = vec![usize::MAX; m];
    for (idx, merge) in raw_merges.iter().enumerate() {
        if merge.left >= 0 {
            let child = merge.left as usize;
            in_deg[idx] += 1;
            parent[child] = idx;
        }
        if merge.right >= 0 {
            let child = merge.right as usize;
            in_deg[idx] += 1;
            parent[child] = idx;
        }
    }
    let mut ready = BinaryHeap::with_capacity(m);
    for (idx, &deg) in in_deg.iter().enumerate() {
        if deg == 0 {
            ready.push(TopoMergeKey { dist: raw_merges[idx].distance, orig_idx: idx });
        }
    }
    let mut order = Vec::with_capacity(m);
    while let Some(TopoMergeKey { orig_idx, .. }) = ready.pop() {
        order.push(orig_idx);
        let p = parent[orig_idx];
        if p < m {
            in_deg[p] = in_deg[p].saturating_sub(1);
            if in_deg[p] == 0 {
                ready.push(TopoMergeKey { dist: raw_merges[p].distance, orig_idx: p });
            }
        }
    }
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
            let left_id = ids[keep];
            let right_id = ids[drop_slot];
            // Reducible linkages satisfy parent.distance >= max(child_left, child_right) in exact
            // arithmetic; clamp 1-2 ULP Lance-Williams roundoff inversions before recording.
            let mut merge_dist = best_d;
            if left_id >= 0 {
                merge_dist = merge_dist.max(raw_merges[left_id as usize].distance);
            }
            if right_id >= 0 {
                merge_dist = merge_dist.max(raw_merges[right_id as usize].distance);
            }
            raw_merges.push(DendrogramMerge { left: left_id, right: right_id, distance: merge_dist, cluster_size: new_size });
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

    reorder_merges_topologically(raw_merges)
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

pub(super) fn hierarchical_clustering_from_dist(dist_mat: &[Vec<f64>], linkage: ClusterLinkage) -> Result<Vec<DendrogramMerge>, StatsError> {
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

fn pairwise_complete_row_distance(a: &[Option<f64>], b: &[Option<f64>], n_cols: usize, metric: ClusterDistance) -> Option<f64> {
    match metric {
        ClusterDistance::Euclidean | ClusterDistance::Manhattan => {
            let mut count = 0_usize;
            let mut sum = 0.0_f64;
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let diff = (va - vb).abs();
                sum += if matches!(metric, ClusterDistance::Euclidean) { diff * diff } else { diff };
                count += 1;
            }
            if count == 0 {
                return None;
            }
            let scaled = sum * ((n_cols as f64) / (count as f64));
            let dist = if matches!(metric, ClusterDistance::Euclidean) { scaled.sqrt() } else { scaled };
            dist.is_finite().then_some(dist)
        }
        ClusterDistance::Cosine => {
            let (mut count, mut dot, mut norm_a, mut norm_b) = (0_usize, 0.0_f64, 0.0_f64, 0.0_f64);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                dot += va * vb;
                norm_a += va * va;
                norm_b += vb * vb;
                count += 1;
            }
            if count == 0 {
                return None;
            }
            let dist = if norm_a <= 1e-30 && norm_b <= 1e-30 {
                0.0
            } else if norm_a <= 1e-30 || norm_b <= 1e-30 {
                1.0
            } else {
                let cos = (dot / (norm_a * norm_b).sqrt()).clamp(-1.0, 1.0);
                (1.0 - cos).clamp(0.0, 2.0)
            };
            dist.is_finite().then_some(dist)
        }
        ClusterDistance::Pearson => {
            let (mut count, mut sum_a, mut sum_b) = (0_usize, 0.0_f64, 0.0_f64);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                sum_a += va;
                sum_b += vb;
                count += 1;
            }
            if count < 2 {
                return None;
            }
            let mean_a = sum_a / (count as f64);
            let mean_b = sum_b / (count as f64);
            let (mut num, mut da, mut db, mut max_diff) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
            for c in 0..n_cols {
                let Some(va) = a.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let Some(vb) = b.get(c).copied().flatten().filter(|x| x.is_finite()) else {
                    continue;
                };
                let xa = va - mean_a;
                let xb = vb - mean_b;
                num += xa * xb;
                da += xa * xa;
                db += xb * xb;
                max_diff = max_diff.max((va - vb).abs());
            }
            let denom = (da * db).sqrt();
            let dist = if !denom.is_finite() || denom <= 1e-30 {
                if max_diff < 1e-12 { 0.0 } else { 1.0 }
            } else {
                let r = (num / denom).clamp(-1.0, 1.0);
                (1.0 - r).clamp(0.0, 2.0)
            };
            dist.is_finite().then_some(dist)
        }
    }
}

/// Pairwise-complete agglomerative hierarchical clustering for matrices with missing (`None`)
/// or non-finite entries.
///
/// Computes pairwise distances over overlapping finite coordinates (scaling Euclidean/Manhattan
/// distances by `n_cols / overlap_count` per Dixon 1979) and applies Lance–Williams linkage
/// (`Lance & Williams 1967`; `Murtagh 1983`). Returns `Ok(None)` when fewer than 2 rows or
/// columns exist or when any row pair lacks sufficient overlapping finite entries.
///
/// # Errors
/// Returns [`StatsError::InvalidParameter`] if `rows.len() > MAX_CLUSTERING_N`.
pub fn hierarchical_clustering_pairwise_complete(
    rows: &[Vec<Option<f64>>],
    n_cols: usize,
    distance: ClusterDistance,
    linkage: ClusterLinkage,
) -> Result<Option<Vec<DendrogramMerge>>, StatsError> {
    let n = rows.len();
    if n < 2 || n_cols < 2 {
        return Ok(None);
    }
    if n > MAX_CLUSTERING_N {
        return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
    }
    let mut dist_mat = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let Some(d) = pairwise_complete_row_distance(&rows[i], &rows[j], n_cols, distance) else {
                return Ok(None);
            };
            dist_mat[i][j] = d;
            dist_mat[j][i] = d;
        }
    }
    hierarchical_clustering_from_dist(&dist_mat, linkage).map(Some)
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

/// Cut an `n`-leaf hierarchical merge schedule at dissimilarity height `cut_height >= 0.0`,
/// merging every step whose height is `<= cut_height` and returning zero-based flat cluster
/// labels `0..k_cut` ordered by each cluster's smallest observation index.
pub fn cut_dendrogram_at_height(n: usize, merges: &[DendrogramMerge], cut_height: f64) -> Result<Vec<usize>, StatsError> {
    if n < 2 || merges.len() != n - 1 {
        return Err(StatsError::DimensionMismatch { expected: n.saturating_sub(1), actual: merges.len() });
    }
    if !cut_height.is_finite() || cut_height < 0.0 {
        return Err(StatsError::InvalidParameter { name: "cut_height", value: cut_height });
    }
    let mut node_members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut active_roots: Vec<bool> = vec![true; n + merges.len()];
    for (step, merge) in merges.iter().enumerate() {
        let left_slot = if merge.left < 0 { (-merge.left - 1) as usize } else { n + (merge.left as usize) };
        let right_slot = if merge.right < 0 { (-merge.right - 1) as usize } else { n + (merge.right as usize) };
        if left_slot >= node_members.len() || right_slot >= node_members.len() {
            return Err(StatsError::InvalidParameter { name: "dendrogram_merge_ref", value: step as f64 });
        }
        let mut combined = node_members[left_slot].clone();
        combined.extend_from_slice(&node_members[right_slot]);
        if merge.distance <= cut_height {
            active_roots[left_slot] = false;
            active_roots[right_slot] = false;
        } else {
            active_roots[n + step] = false;
        }
        node_members.push(combined);
    }

    let mut clusters: Vec<Vec<usize>> = node_members
        .into_iter()
        .enumerate()
        .filter_map(|(slot, mut members)| {
            if active_roots.get(slot).copied().unwrap_or(false) {
                members.sort_unstable();
                Some(members)
            } else {
                None
            }
        })
        .collect();
    clusters.sort_by_key(|m| m.first().copied().unwrap_or(usize::MAX));
    let mut assignments = vec![0_usize; n];
    for (cid, members) in clusters.iter().enumerate() {
        for &leaf in members {
            assignments[leaf] = cid;
        }
    }
    Ok(assignments)
}
