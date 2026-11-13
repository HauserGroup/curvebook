//! Biostatistics engine for LumenCraft: parametric and nonparametric group comparisons, one-, two-
//! and three-way ANOVA, mixed-effects models, multiple-comparison corrections, contingency tables,
//! ROC curves, Bland-Altman agreement, Kaplan-Meier / Cox survival analysis and multivariate models.
//!
//! Layer L2 domain crate. Governing spec: `docs/statistics-implementation.md` and `docs/stats-parity.md`.
#![forbid(unsafe_code)]

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

#[cfg(test)]
mod tests {
    use super::AnalysisFamily;

    #[test]
    fn analysis_families_are_distinct() {
        assert_ne!(AnalysisFamily::OneWayAnova, AnalysisFamily::Survival);
    }
}
