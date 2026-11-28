//! Row-wise multiple two-group *t*-tests with multiplicity adjustment (Volcano plot source).
//!
//! # References
//!
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 8.

use curvebook_math::ConfidenceLevel;
use serde::{Deserialize, Serialize};

use super::parametric::{unpaired_t_test, welch_t_test};
use crate::error::StatsError;
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values};

/// Single row comparison entry for multiple *t*-tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleRowTestEntry {
    /// Zero-based table row index tested.
    pub row_index: usize,
    /// Human-readable row feature label.
    pub label: String,
    /// Sample mean (or median in nonparametric rows) of group A on this row.
    pub mean_a: f64,
    /// Sample mean (or median in nonparametric rows) of group B on this row.
    pub mean_b: f64,
    /// Signed shift/difference `mean_a - mean_b` (or HL shift / difference of paired medians).
    pub difference: f64,
    /// Row-level test statistic (*t*, Mann–Whitney *U*/*U_min*, or Wilcoxon *W*/*stat*).
    pub statistic: f64,
    /// Degrees of freedom (`n_a + n_b - 2`, Welch *df*, `n_pairs - 1`, or `f64::INFINITY` for rank tests).
    pub df: f64,
    /// Unadjusted two-sided single-row *P* value.
    pub raw_p: f64,
    /// Familywise-error adjusted *P* or FDR-adjusted *q* value across all tested rows.
    pub adjusted_p_or_q: f64,
    /// True when the row passes the multiplicity threshold (discovery).
    pub significant: bool,
}

/// Reject ANOVA-specific pairwise/post-hoc procedures that are undefined for raw-P row families.
pub(super) fn validate_raw_p_multiplicity_procedure(procedure: MultiplicityProcedure) -> Result<(), StatsError> {
    match procedure {
        MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::NewmanKeuls
        | MultiplicityProcedure::Duncan
        | MultiplicityProcedure::Dunnett { .. }
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnRankSum
        | MultiplicityProcedure::LinearTrend
        | MultiplicityProcedure::TamhaneT2 => Err(StatsError::IncompatibleAnalysisSpec {
            expected: "raw p-value multiplicity procedure (None, Bonferroni, Sidak, HolmBonferroni, HolmSidak, BenjaminiHochbergFdr, BenjaminiYekutieliFdr, or TwoStageBkyFdr)",
        }),
        MultiplicityProcedure::None
        | MultiplicityProcedure::FisherLsd
        | MultiplicityProcedure::Sidak
        | MultiplicityProcedure::Bonferroni
        | MultiplicityProcedure::HolmSidak
        | MultiplicityProcedure::HolmBonferroni
        | MultiplicityProcedure::BenjaminiHochbergFdr
        | MultiplicityProcedure::BenjaminiYekutieliFdr
        | MultiplicityProcedure::TwoStageBkyFdr => Ok(()),
    }
}

/// Run one unpaired or Welch *t*-test per row across two replicate groups and apply multiplicity.
///
/// Each row uses [`unpaired_t_test`] (Student 1908) or [`welch_t_test`] (Welch 1947); the adjustment
/// is delegated to `crate::multiplicity`, which carries its own references. Rows whose test fails
/// (e.g. fewer than two values or zero variance) are omitted from the output and from the adjustment.
/// ANOVA-only procedures (`TukeyKramer`, `NewmanKeuls`, `Dunnett`, `DunnettT3`, `GamesHowell`,
/// `DunnRankSum`, `LinearTrend`, `TamhaneT2`) are rejected with [`StatsError::IncompatibleAnalysisSpec`].
pub fn multiple_row_t_tests(
    row_labels: &[String],
    group_a_rows: &[Vec<f64>],
    group_b_rows: &[Vec<f64>],
    welch: bool,
    procedure: MultiplicityProcedure,
    threshold: f64,
) -> Result<Vec<MultipleRowTestEntry>, StatsError> {
    validate_raw_p_multiplicity_procedure(procedure)?;
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if group_a_rows.len() != group_b_rows.len() {
        return Err(StatsError::DimensionMismatch { expected: group_a_rows.len(), actual: group_b_rows.len() });
    }
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let res =
            if welch { welch_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) } else { unpaired_t_test(ra, rb, true, ConfidenceLevel::DEFAULT_95) };
        if let Ok(r) = res
            && r.p_value.is_finite()
        {
            let lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
            raw_p.push(r.p_value);
            staged.push((idx, lbl, r.mean_a, r.mean_b, r.difference, r.t_statistic, r.df, r.p_value));
        }
    }
    let (adj, _) = adjust_p_values(&raw_p, procedure, threshold);
    Ok(staged
        .into_iter()
        .zip(adj)
        .map(|((row_index, label, mean_a, mean_b, difference, statistic, df, raw_p), adj_p)| MultipleRowTestEntry {
            row_index,
            label,
            mean_a,
            mean_b,
            difference,
            statistic,
            df,
            raw_p,
            adjusted_p_or_q: adj_p,
            significant: threshold.is_finite() && adj_p.is_finite() && adj_p <= threshold,
        })
        .collect())
}
