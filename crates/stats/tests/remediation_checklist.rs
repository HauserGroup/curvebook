//! Integration tests verifying structured [`ChecklistItem::remediation`] diagnostics (`C2`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, OneWayAnovaSpec, OneWayMethod,
    RemediationKind, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use lumencraft_stats::run_analysis;

fn build_column_sheet(cols: Vec<Vec<f64>>) -> DataSheet {
    let n_rows = cols.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).expect("column table");
    let mut sheet = DataSheet::new("data-1", "Assay", TableArchetype::Column(table)).expect("data sheet");
    for (c_idx, col_vals) in cols.iter().enumerate() {
        for (r_idx, &val) in col_vals.iter().enumerate() {
            sheet.set_cell(CellCoord::new(r_idx, c_idx, 0), Cell::number(val)).expect("set cell");
        }
    }
    sheet
}

#[test]
fn two_group_unequal_variance_and_non_normality_populate_remediation() {
    // Unequal variances with normal-looking groups -> WelchT.
    let sheet_var = build_column_sheet(vec![vec![10.0, 10.1, 9.9, 10.2, 9.8, 10.0], vec![15.0, 25.0, 5.0, 35.0, -5.0, 45.0]]);
    let spec_unpaired = AnalysisSpec::new(
        "res-unpaired",
        "data-1",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let res_var = run_analysis(&sheet_var, &spec_unpaired).expect("unpaired t");
    assert!(
        res_var.checklist.iter().any(|c| c.status == ChecklistStatus::Violated && c.remediation == Some(RemediationKind::WelchT)),
        "expected Violated -> WelchT in {:?}",
        res_var.checklist
    );

    // Strongly skewed non-normal data -> MannWhitneyU (even when using WelchT).
    let sheet_skew = build_column_sheet(vec![vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.1, 100.0], vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.1, 200.0]]);
    let spec_welch = AnalysisSpec::new(
        "res-welch",
        "data-1",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let res_skew = run_analysis(&sheet_skew, &spec_welch).expect("welch t");
    assert!(
        res_skew.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::MannWhitneyU)),
        "expected Warn -> MannWhitneyU in {:?}",
        res_skew.checklist
    );

    // Healthy Gaussian balanced sample -> Pass, remediation: None everywhere.
    let sheet_ok = build_column_sheet(vec![vec![10.0, 11.2, 9.8, 10.5, 10.9, 9.6, 10.3], vec![12.1, 13.0, 11.7, 12.6, 12.9, 11.9, 12.4]]);
    let res_ok = run_analysis(&sheet_ok, &spec_unpaired).expect("healthy unpaired t");
    assert!(res_ok.checklist.iter().all(|c| c.remediation.is_none()), "healthy data should not emit remediation: {:?}", res_ok.checklist);

    // Nonparametric Mann-Whitney U should never suggest parametric remediation.
    let spec_mw = AnalysisSpec::new(
        "res-mw",
        "data-1",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let res_mw = run_analysis(&sheet_skew, &spec_mw).expect("mann whitney");
    assert!(res_mw.checklist.iter().all(|c| c.remediation.is_none()));
}

#[test]
fn paired_and_one_sample_t_tests_populate_wilcoxon_remediation() {
    let sheet_small_pairs = build_column_sheet(vec![vec![1.0, 2.0, 3.0], vec![2.0, 3.5, 4.1]]);
    let spec_paired = AnalysisSpec::new(
        "res-paired",
        "data-1",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let res_paired = run_analysis(&sheet_small_pairs, &spec_paired).expect("paired t");
    assert!(
        res_paired.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::WilcoxonSignedRank)),
        "expected WilcoxonSignedRank in {:?}",
        res_paired.checklist
    );

    let spec_one_sample = AnalysisSpec::new(
        "res-1s",
        "data-1",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..TwoGroupSpec::default() }),
    )
    .expect("spec");
    let res_1s = run_analysis(&sheet_small_pairs, &spec_one_sample).expect("one sample t");
    assert!(
        res_1s.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::OneSampleWilcoxon)),
        "expected OneSampleWilcoxon in {:?}",
        res_1s.checklist
    );
}

#[test]
fn one_way_and_rm_anova_populate_welch_kruskal_and_friedman_remediations() {
    // Unequal variances across 3 groups -> WelchAnova.
    let sheet_var = build_column_sheet(vec![
        vec![10.0, 10.1, 9.9, 10.2, 9.8, 10.0],
        vec![15.0, 15.1, 14.9, 15.2, 14.8, 15.0],
        vec![20.0, 50.0, -10.0, 80.0, -40.0, 110.0],
    ]);
    let spec_ord = AnalysisSpec::new(
        "res-anova",
        "data-1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..OneWayAnovaSpec::default() }),
    )
    .expect("spec");
    let res_ord = run_analysis(&sheet_var, &spec_ord).expect("ordinary anova");
    assert!(
        res_ord.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::WelchAnova)),
        "expected WelchAnova in {:?}",
        res_ord.checklist
    );

    // Strongly non-normal 3-group data under Brown-Forsythe & Welch ANOVA -> KruskalWallis.
    let sheet_skew = build_column_sheet(vec![
        vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.1, 100.0],
        vec![2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.1, 200.0],
        vec![3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.1, 300.0],
    ]);
    let spec_welch = AnalysisSpec::new(
        "res-welch-anova",
        "data-1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..OneWayAnovaSpec::default() }),
    )
    .expect("spec");
    let res_welch = run_analysis(&sheet_skew, &spec_welch).expect("welch anova");
    assert!(
        res_welch.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::KruskalWallis)),
        "expected KruskalWallis in {:?}",
        res_welch.checklist
    );

    // Repeated measures with skewed condition values (no GG correction) -> Friedman.
    let spec_rm = AnalysisSpec::new(
        "res-rm",
        "data-1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
            ..OneWayAnovaSpec::default()
        }),
    )
    .expect("spec");
    let res_rm = run_analysis(&sheet_skew, &spec_rm).expect("rm anova");
    assert!(
        res_rm.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.remediation == Some(RemediationKind::Friedman)),
        "expected Friedman in {:?}",
        res_rm.checklist
    );
}
