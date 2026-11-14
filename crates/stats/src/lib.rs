//! Biostatistics & Multiplicity Engine for LumenCraft (Layer L2).
//!
//! Provides column descriptive statistics, normality & lognormality tests, outlier detection
//! (Grubbs & standalone ROUT), two-group comparisons & Bland–Altman agreement, one-/two-/three-/N-way
//! ANOVA, repeated-measures sphericity corrections, mixed-effects REML, nested hierarchical ANOVA,
//! all 14+ FWER/FDR multiple comparison procedures, contingency tables & kappa agreement,
//! Kaplan–Meier & Cox PH survival models, DeLong ROC curves, multiple linear/logistic/Poisson GLMs,
//! and PCA / correlation / clustering.

#![forbid(unsafe_code)]

pub mod anova;
pub mod categorical;
pub mod column;
pub mod common;
pub mod error;
pub mod multiplicity;
pub mod multivariate;
pub mod regression;
pub mod roc;
pub mod survival;
pub mod two_group;

use lumencraft_doc::{AnalysisKind, AnalysisSpec, DataSheet, ResultSheet};

pub use error::StatsError;

/// High-level families of statistical analyses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisFamily {
    Descriptive,
    TwoGroupComparison,
    OneWayAnova,
    TwoWayAnova,
    ThreeWayAnova,
    NestedComparison,
    Contingency,
    Survival,
    Correlation,
    Multivariate,
}

/// Dispatch any statistical [`AnalysisSpec`] handled by `lumencraft-stats` against a [`DataSheet`]
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
        AnalysisKind::Transform(_) => {
            Err(StatsError::IncompatibleTable { analysis: "transform (handled by lumencraft-transform)", actual: sheet.archetype.kind() })
        }
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) => {
            Err(StatsError::IncompatibleTable { analysis: "curve fitting (handled by lumencraft-fit)", actual: sheet.archetype.kind() })
        }
    }
}
