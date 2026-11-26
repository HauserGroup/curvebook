//! Typed result and configuration structures for multivariate statistics: correlation pairs and
//! matrices, PCA, *k*-means++ and hierarchical clustering, and MICE multiple imputation with Rubin
//! pooling.
//!
//! References:
//! - Pearson, K. (1896). Mathematical contributions to the theory of evolution. III. Regression,
//!   heredity, and panmixia. *Philosophical Transactions of the Royal Society A*, 187, 253-318.
//!   `doi:10.1098/rsta.1896.0007`.
//! - Spearman, C. (1904). The proof and measurement of association between two things. *American
//!   Journal of Psychology*, 15(1), 72-101. `doi:10.2307/1412159`.
//! - Kendall, M. G. (1945). The treatment of ties in ranking problems. *Biometrika*, 33(3), 239-251.
//!   `doi:10.1093/biomet/33.3.239`.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). New York: Springer.
//! - Arthur, D., & Vassilvitskii, S. (2007). k-means++: The advantages of careful seeding.
//!   *Proceedings of the 18th Annual ACM-SIAM Symposium on Discrete Algorithms (SODA)*, 1027-1035.
//! - Rousseeuw, P. J. (1987). Silhouettes: A graphical aid to the interpretation and validation of
//!   cluster analysis. *Journal of Computational and Applied Mathematics*, 20, 53-65.
//!   `doi:10.1016/0377-0427(87)90125-7`.
//! - Ward, J. H. (1963). Hierarchical grouping to optimize an objective function. *Journal of the
//!   American Statistical Association*, 58(301), 236-244. `doi:10.1080/01621459.1963.10500845`.
//! - Rubin, D. B. (1987). *Multiple Imputation for Nonresponse in Surveys*. Wiley.
//! - Barnard, J., & Rubin, D. B. (1999). Small-sample degrees of freedom with multiple imputation.
//!   *Biometrika*, 86(4), 948-955.

use curvebook_math::ConfidenceLevel;
use serde::{Deserialize, Serialize};

/// Single pairwise correlation summary (Pearson, Spearman, or Kendall tau-b).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationPairResult {
    /// Sample correlation coefficient (`r`, Spearman `rho`, or Kendall `tau_b`) in `[-1, 1]`.
    pub r: f64,
    /// Squared coefficient `r^2` in `[0, 1]` representing shared linear variance (Pearson) or squared rank association.
    pub r_squared: f64,
    /// Two-sided Fisher *z*-transformed asymptotic confidence interval `(lower, upper)` clamped to `[-1, 1]` (`None` when `n < 4` or degenerate).
    pub ci: Option<(f64, f64)>,
    /// Test statistic (`t = r sqrt((n - 2) / (1 - r^2))` for Pearson/Spearman or normal `z` for Kendall tau-b).
    pub statistic: f64,
    /// Student *t* degrees of freedom `n - 2` for Pearson/Spearman (`None` for asymptotic normal Kendall tau-b).
    pub df: Option<f64>,
    /// Two-sided *P* value testing zero correlation ($H_0: \rho = 0$ or $\tau = 0$).
    pub p_value: f64,
    /// Number of complete finite `(x, y)` pairs included in this pairwise estimate.
    pub n: usize,
}

/// Correlation matrix output across `p` variables with BH-FDR *q*-values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationMatrixResult {
    /// Ordered variable names of length `p`.
    pub labels: Vec<String>,
    /// Symmetric `p x p` pairwise correlation matrix (`r_{ij}` in `[-1, 1]`; `1.0` on diagonal, `NaN` when pairwise `n < 3` or zero variance).
    pub r_matrix: Vec<Vec<f64>>,
    /// Symmetric `p x p` unadjusted two-sided *P* value matrix (`0.0` on diagonal, `NaN` for unestimable pairs).
    pub p_matrix: Vec<Vec<f64>>,
    /// Symmetric `p x p` Benjamini–Hochberg (1995) step-up FDR *q* value matrix adjusted over the defined off-diagonal pairs (`0.0` on diagonal, `NaN` for unestimable pairs excluded from the family).
    pub fdr_q_matrix: Vec<Vec<f64>>,
    /// Symmetric `p x p` matrix of pairwise complete-case sample sizes `n_{ij}`.
    pub n_matrix: Vec<Vec<usize>>,
    /// True when Spearman rank correlation was computed; false for Pearson product-moment correlation.
    pub spearman: bool,
}

/// Complete Principal Component Analysis result (Hotelling 1933; Horn 1965 parallel analysis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcaResult {
    /// Number of complete observations (rows) used in PCA (`n >= 2`).
    pub n: usize,
    /// Number of numeric predictor variables (columns) analyzed (`p >= 1`).
    pub p: usize,
    /// True when variables were standardized to correlation matrix (unit variance), false for centered covariance matrix.
    pub standardized: bool,
    /// Ordered variable column titles of length `p`.
    pub variable_labels: Vec<String>,
    /// Ordered eigenvalues $\lambda_1 \ge \lambda_2 \ge \dots \ge \lambda_p \ge 0$ of the correlation or covariance matrix.
    pub eigenvalues: Vec<f64>,
    /// Fraction of total variance explained by each principal component ($\lambda_k / \sum \lambda_j \in [0, 1]$).
    pub variance_proportion: Vec<f64>,
    /// Cumulative fraction of variance explained $\sum_{j=1}^k (\lambda_j / \sum \lambda) \in [0, 1]$.
    pub cumulative_proportion: Vec<f64>,
    /// Count of components satisfying the Kaiser–Guttman criterion (`eigenvalue > 1.0` for standardized correlation PCA, else `> mean(eigenvalues)`).
    pub kaiser_components: usize,
    /// Optional 95th-percentile Monte Carlo parallel analysis eigenvalue thresholds (Horn 1965; `None` when not requested).
    pub parallel_analysis_thresholds: Option<Vec<f64>>,
    /// Count of components exceeding their corresponding parallel-analysis random eigenvalue threshold (`None` when parallel analysis was skipped).
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
    /// Nearest-neighbor minimum inter-cluster dissimilarity.
    Single,
    /// Farthest-neighbor maximum inter-cluster dissimilarity.
    Complete,
    /// Unweighted Pair Group Method with Arithmetic Mean (UPGMA average linkage).
    Average,
    /// Ward's minimum-variance criterion with Euclidean dissimilarity scaling (`hclust(method = "ward.D2")`).
    WardD2,
}

/// Single merge step in a hierarchical clustering dendrogram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DendrogramMerge {
    /// Negative IDs (`-(i + 1)`) denote original observations `0..n`; non-negative denote prior merge step `0..step`.
    pub left: isize,
    /// Second merged cluster identifier using the same negative-leaf / non-negative-node encoding as `left`.
    pub right: isize,
    /// Linkage dissimilarity height at which `left` and `right` merged.
    pub distance: f64,
    /// Total number of original observations contained in the newly formed merged cluster.
    pub cluster_size: usize,
}

/// Complete *k*-means++ clustering result (Arthur & Vassilvitskii 2007).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KMeansResult {
    /// Target number of clusters `k >= 1`.
    pub k: usize,
    /// Zero-based cluster assignment `0..k` for each input observation row.
    pub assignments: Vec<usize>,
    /// Cluster centroid coordinates (`k x p` vectors in input feature space).
    pub centroids: Vec<Vec<f64>>,
    /// Total within-cluster objective sum (squared Euclidean distances for `Euclidean`, or $L_1$ distances for `Manhattan`).
    pub wcss: f64,
    /// Mean Rousseeuw (1987) silhouette width in `[-1, 1]` across all observations (`0.0` for degenerate `k == 1` or `k == n`).
    pub mean_silhouette: f64,
}

/// Pooled parameter estimate across `m` multiply imputed datasets using Rubin's (1987) rules
/// with Barnard–Rubin (1999) small-sample degrees-of-freedom adjustment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubinPooledEstimate {
    /// Estimated variable name or linear regression coefficient term name.
    pub term: String,
    /// Number of imputed datasets pooled (`m >= 2`).
    pub m: usize,
    /// Rubin pooled point estimate $\bar{Q} = \frac{1}{m}\sum_{k=1}^m \hat{Q}_k$.
    pub estimate: f64,
    /// Average within-imputation sampling variance $\bar{U} = \frac{1}{m}\sum_{k=1}^m U_k$.
    pub within_variance: f64,
    /// Between-imputation variance $B = \frac{1}{m - 1}\sum_{k=1}^m (\hat{Q}_k - \bar{Q})^2$.
    pub between_variance: f64,
    /// Rubin total variance $T = \bar{U} + (1 + 1/m) B$.
    pub total_variance: f64,
    /// Pooled standard error $\sqrt{T}$.
    pub pooled_se: f64,
    /// Two-sided Wald *t* statistic $\bar{Q} / \sqrt{T}$ (`f64::NAN` when total variance `T <= 0`).
    pub t_statistic: f64,
    /// Barnard–Rubin (1999) adjusted *t* degrees of freedom accounting for finite `m` and complete-data df.
    pub df: f64,
    /// Two-sided `t(df)` *P* value testing $H_0: Q = 0$ (`f64::NAN` when total variance `T <= 0`).
    pub p_value: f64,
    /// Two-sided `t(df)` confidence interval `(lower, upper)` around `estimate` at configured `ConfidenceLevel` (`(f64::NAN, f64::NAN)` when `T <= 0`).
    pub ci: (f64, f64),
    /// Relative increase in variance due to nonresponse $r = (1 + 1/m) B / \bar{U}$ (`f64::INFINITY` when $\bar{U} = 0$ and $B > 0$; `0.0` when $T = 0$).
    pub riv: f64,
    /// Proportion of total variance attributable to missingness $\lambda = (1 + 1/m) B / T = r / (1 + r)$ in `[0, 1]` (Rubin 1987).
    pub lambda: f64,
    /// Fraction of missing information `FMI = (r + 2 / (df + 3)) / (1 + r)` in `[0, 1]` (Rubin 1987).
    pub fmi: f64,
}

/// Configuration options for seeded MICE Predictive Mean Matching multiple imputation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleImputationConfig {
    /// Number of multiply imputed completed datasets `m >= 2` (default `5`).
    pub imputations_m: usize,
    /// Number of chained-equation Gibbs sweep iterations per imputation (default `10`).
    pub max_iters: usize,
    /// Deterministic PRNG seed controlling PMM stochastic draws and Bayesian regression perturbations.
    pub seed: u64,
    /// Optional zero-based response variable index for pooled OLS regression after imputation (`None` omits pooled regression).
    pub response_col: Option<usize>,
    /// Zero-based predictor variable indices used alongside `response_col` when pooling an OLS model.
    pub predictor_cols: Vec<usize>,
    /// Two-sided confidence level for Rubin-pooled confidence intervals (default `0.95` / 95%).
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
    /// Number of imputed datasets generated (`m`).
    pub m: usize,
    /// Number of MICE sweep iterations executed per dataset.
    pub max_iters: usize,
    /// Seed used for deterministic PMM draws.
    pub seed: u64,
    /// Total number of rows in the input multivariate matrix.
    pub n_rows: usize,
    /// Column labels aligned with table variable indices `0..p`.
    pub variable_labels: Vec<String>,
    /// Number of missing entries imputed per column (`0..p`).
    pub missing_counts: Vec<usize>,
    /// Grand total number of missing values imputed across all columns.
    pub total_missing: usize,
    /// Rubin-pooled sample means (with total variance, *df*, *P* vs 0, CI, and FMI) for every column.
    pub pooled_means: Vec<RubinPooledEstimate>,
    /// Optional Rubin-pooled OLS regression coefficients when `response_col` and `predictor_cols` were specified (`None` otherwise).
    pub pooled_regression: Option<Vec<RubinPooledEstimate>>,
    /// Completed datasets indexed as `imputed_datasets[imp_idx][row_idx][col_idx]`.
    pub imputed_datasets: Vec<Vec<Vec<f64>>>,
}
