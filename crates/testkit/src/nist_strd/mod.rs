//! NIST Statistical Reference Datasets (StRD) parser, data structures, and offline certified
//! reference benchmarks (`docs/development.md` §Verified NIST StRD corpus;
//! `docs/statistics-implementation.md` §1.3, §6).

pub mod datasets;
pub mod parser;

pub use datasets::{
    all_offline_anova_benchmarks, all_offline_linear_regression_benchmarks, all_offline_univariate_benchmarks, binary_stiff_1e8,
    filip_scale_vandermonde_benchmark, noint1_exact, noint2_exact, numacc1, numacc2, numacc3, numacc4, smls_stiff_anova, wampler_orthogonal_stiff,
    wampler1, wampler2,
};
pub use parser::{
    AnovaDataset, LinearRegressionDataset, NistDifficulty, NonlinearDataset, StrdParseError, UnivariateDataset, parse_anova_dat,
    parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat,
};
