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
//!
//! Module layout: shared result/configuration types (`types`), pairwise correlations and matrices
//! (`correlation`), Principal Component Analysis (`pca`), *k*-means++ and hierarchical clustering
//! (`clustering`), MICE PMM and Rubin pooling (`mice`), and `ResultSheet` builders (`sheet`).
//! Every public item is re-exported here.

mod clustering;
mod correlation;
mod kmeans_consensus;
mod mice;
mod pca;
mod sheet;
mod types;

pub use clustering::{
    MAX_CLUSTERING_N, compute_mean_silhouette_from_dist, cophenetic_correlation, cophenetic_distance_matrix, cut_dendrogram,
    cut_dendrogram_at_height, hierarchical_clustering, hierarchical_clustering_pairwise_complete, hierarchical_clustering_with_metrics, kmeans_pp,
    kmeans_pp_with_distance, pairwise_distance_matrix, pairwise_distance_value,
};
pub use correlation::{correlation_matrix, kendall_tau_b, pearson_correlation, spearman_correlation};
pub use curvebook_doc::{ClusterDirection, ClusterDistance, ClusterLinkage, KMeansAlgorithm, PcaRetentionRule};
pub use kmeans_consensus::{
    ConsensusIndexVote, ConsensusOptimalKResult, GapCurvePoint, KMeansFeatureAnovaRow, KMeansValidityIndices, compute_gap_statistic,
    compute_kmeans_validity_indices, consensus_optimal_k, kmeans_feature_anova, kmeans_with_algorithm, kmeans_with_centers,
};
pub use mice::{mice_predictive_mean_matching, rubin_pool_estimates};
pub use pca::{PcaExtraMatrices, compute_pca_extra_matrices, pca_analysis, select_pca_retained_components};
pub use sheet::{ci_cell_from_option, run_multivariate_analysis};
pub use types::{
    CorrelationMatrixResult, CorrelationPairResult, DendrogramMerge, KMeansResult, LinkageMethod, MultipleImputationConfig, MultipleImputationResult,
    PcaResult, RubinPooledEstimate,
};
