//! Biostatistics & Multiplicity Engine for CurveBook (Layer L2).
//!
//! Provides column descriptive statistics, normality & lognormality tests, outlier detection
//! (Grubbs & standalone ROUT), two-group comparisons & Bland–Altman agreement, one-/two-/three-/N-way
//! ANOVA, repeated-measures sphericity corrections, mixed-effects REML, nested hierarchical ANOVA,
//! all 14+ FWER/FDR multiple comparison procedures, contingency tables & kappa agreement,
//! Kaplan–Meier & Cox PH survival models, DeLong ROC curves, multiple linear/logistic/Poisson GLMs,
//! and PCA / correlation / clustering.
//!
//! # Example
//!
//! ```
//! use curvebook_math::ConfidenceLevel;
//! use curvebook_stats::two_group::welch_t_test;
//!
//! let control = [10.2, 11.0, 9.8, 10.5, 10.9];
//! let treated = [13.8, 14.4, 15.1, 13.5, 14.9];
//!
//! let res = welch_t_test(&treated, &control, true, ConfidenceLevel::DEFAULT_95)
//!     .expect("two finite samples with n >= 2");
//! assert!((res.difference - 3.86).abs() < 1e-12);
//! assert!(res.ci.0 > 2.9 && res.ci.1 < 4.8);
//! assert!(res.df > 6.5 && res.df < 8.0);
//! assert!(res.p_value < 1e-4);
//! assert!(res.effect_sizes.hedges_g > 5.0);
//! ```

#![forbid(unsafe_code)]
#![allow(clippy::indexing_slicing)]
// Legacy dense matrix/solver indexing (FA-SEC23-03); string_slice remains denied and all 14 I/O/boundary/app crates enforce workspace indexing_slicing = "deny".
#![deny(missing_docs)]

pub mod anova;
pub mod categorical;
pub mod cld;
pub mod column;
pub mod common;
pub mod error;
pub mod multiplicity;
pub mod multivariate;
pub mod power;
pub mod pvalue_stack;
pub mod regression;
pub mod roc;
pub mod simple_logistic;
pub mod survival;
pub mod two_group;

use curvebook_doc::{AnalysisKind, AnalysisSpec, DataSheet, ResultSheet};

pub use error::StatsError;
pub use power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerAnalysisResult, PowerCurvePoint, PowerEffectSpec, PowerSolveTarget,
    PowerTestFamily, SensitivityCell, TailMode, TwoProportionsMethod, compute_power_curve, compute_sensitivity_table, solve_power,
};

/// High-level families of statistical analyses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisFamily {
    /// Column-wise descriptive statistics, quantiles, medians, normality, and single-column outlier tests.
    Descriptive,
    /// Two-sample paired and unpaired comparisons (Student/Welch *t*, Mann–Whitney, Wilcoxon, KS, Bland–Altman).
    TwoGroupComparison,
    /// One-way parametric (ordinary/Welch/Brown–Forsythe/RM) and Kruskal–Wallis / Friedman ANOVAs.
    OneWayAnova,
    /// Two-way factorial and repeated-measures ANOVA / mixed-effects REML analyses.
    TwoWayAnova,
    /// Three-way factorial and repeated-measures ANOVA / mixed-effects REML analyses.
    ThreeWayAnova,
    /// Hierarchical nested *t*-test and nested one-way random-effects ANOVA.
    NestedComparison,
    /// Contingency table tests ($\chi^2$, Fisher exact, McNemar, Barnard exact, binomial, Cohen's kappa, relative ratios).
    Contingency,
    /// Kaplan–Meier curves, log-rank/Gehan tests, hazard ratios, and Cox proportional hazards regression.
    Survival,
    /// Pairwise Pearson/Spearman correlation matrices, simple logistic, Passing–Bablok, and GLMs.
    Correlation,
    /// Multivariate PCA, hierarchical/k-means clustering, and multiple imputation.
    Multivariate,
    /// Sample size, achieved power, minimum detectable effect, power curves, and sensitivity grids.
    PowerAndSampleSize,
}

/// Dispatch any statistical [`AnalysisSpec`] handled by `curvebook-stats` against a [`DataSheet`]
/// and produce a complete [`ResultSheet`] (scalars, tabs, pairwise comparison records, curves, and
/// plain-language `ChecklistItem` diagnostics).
pub fn run_analysis(sheet: &DataSheet, spec: &AnalysisSpec) -> Result<ResultSheet, StatsError> {
    match &spec.kind {
        AnalysisKind::DescriptiveStats(d) => column::run_descriptive_analysis(sheet, spec, d),
        AnalysisKind::OutlierDetection(o) => column::run_outlier_analysis(sheet, spec, *o),
        AnalysisKind::TwoGroupComparison(t) => two_group::run_two_group_analysis(sheet, spec, t),
        AnalysisKind::BlandAltman(ba) => two_group::run_bland_altman_analysis(sheet, spec, ba),
        AnalysisKind::MultipleRowTTests(mr) => two_group::run_multiple_row_t_tests_analysis(sheet, spec, mr),
        AnalysisKind::PassingBablok(pb) => regression::run_passing_bablok_analysis(sheet, spec, pb),
        AnalysisKind::OneWayAnova(a) => anova::run_one_way_anova(sheet, spec, a),
        AnalysisKind::FactorialAnova(f) => anova::run_factorial_anova(sheet, spec, f),
        AnalysisKind::NestedComparison(n) => anova::run_nested_analysis(sheet, spec, n),
        AnalysisKind::Contingency(c) => categorical::run_contingency_analysis(sheet, spec, c),
        AnalysisKind::Survival(s) => survival::run_survival_analysis(sheet, spec, s),
        AnalysisKind::RocCurve(r) => roc::run_roc_analysis(sheet, spec, r),
        AnalysisKind::GeneralizedLinearModel(g) => regression::run_glm_analysis(sheet, spec, g),
        AnalysisKind::Multivariate(m) => multivariate::run_multivariate_analysis(sheet, spec, m),
        AnalysisKind::Transform(_) | AnalysisKind::FitSplineLowess(_) => {
            Err(StatsError::IncompatibleTable { analysis: "transform (handled by curvebook-transform)", actual: sheet.archetype.kind() })
        }
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => {
            Err(StatsError::IncompatibleTable { analysis: "curve fitting (handled by curvebook-fit)", actual: sheet.archetype.kind() })
        }
        AnalysisKind::PValueStack(ps) => pvalue_stack::run_pvalue_stack_analysis(sheet, spec, ps),
        AnalysisKind::PowerAnalysis(p) => power::run_power_analysis(sheet, spec, p),
        AnalysisKind::SimpleLogisticRegression(sl) => simple_logistic::run_simple_logistic_analysis(sheet, spec, sl),
    }
}
