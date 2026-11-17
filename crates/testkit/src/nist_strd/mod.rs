//! NIST Statistical Reference Datasets (StRD) parser, data structures, and offline certified
//! reference benchmarks (`docs/development.md` §Verified NIST StRD corpus;
//! `docs/implementation/statistics-implementation.md` §1.3, §6).

pub mod corpus;
pub mod datasets;
pub mod parser;

pub use corpus::{
    PINNED_NIST_STRD_CORPUS, PinnedStrdSpec, StrdCorpusLoadError, StrdCorpusSuite, corpus_available, default_nist_strd_dir, find_pinned_spec,
    load_corpus_anova, load_corpus_anova_all, load_corpus_linear, load_corpus_linear_all, load_corpus_nonlinear, load_corpus_nonlinear_all,
    load_corpus_univariate, load_corpus_univariate_all, verify_pinned_corpus_all,
};

pub use datasets::{
    all_offline_anova_benchmarks, all_offline_linear_regression_benchmarks, all_offline_univariate_benchmarks, binary_stiff_1e8,
    filip_scale_vandermonde_benchmark, noint1_exact, noint2_exact, numacc1, numacc2, numacc3, numacc4, smls_stiff_anova, wampler_orthogonal_stiff,
    wampler1, wampler2,
};
pub use parser::{
    AnovaDataset, LinearRegressionDataset, MAX_STRD_BYTES, MAX_STRD_ROWS, NistDifficulty, NonlinearDataset, StrdParseError, UnivariateDataset,
    parse_anova_dat, parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat,
};
