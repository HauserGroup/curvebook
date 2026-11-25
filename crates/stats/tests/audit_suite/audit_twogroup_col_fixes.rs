#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Wave 2 audit regression suite covering `crates/stats/src/{two_group,column,common}.rs`.
//!
//! Regression tests for local code audit findings:
//! - `#4`: `BlandAltmanSpec::xy_compare_x_vs_y` locks X vs Y0 across dataset additions
//! - `#7`: `MultipleRowTTests` paired & selected dataset parity validation across Column/MV/Grouped/XY
//! - `#21`: Exact small-sample untied Hodges–Lehmann CIs (`n <= 30`) and achieved coverage disclosure
//! - `#32`: `try_resolve_selected_indices` rejects out-of-range and duplicate dataset selections
//! - `#66`: Exclude `Label` and `Categorical` roles from default numeric selections on MV tables
//! - `#67`: Lognormal unpaired *t*-test labels log-scale vs geometric-ratio rows clearly
//! - `#68`: Custom percentiles use Hyndman & Fan Type 7 consistently and emit in descriptive tab
//! - `#69`: Glass's Delta avoids substituting group B SD when group A SD = 0 and discloses group A
//! - `#81`: `MultipleRowTTests` rejects ANOVA-only multiplicity procedures (`IncompatibleAnalysisSpec`)
//! - `#82`: `ResultCell::PValue` asterisk formatting tracks analysis `alpha` with `p <= alpha` boundary
//! - `#93`: Persisted `mv_group_column` / `mv_response_column` resist retargeting when calculated columns are added
//! - `#100`: Anderson–Darling *P*-value remains monotonically non-increasing past quadratic vertex (`A2* ~ 153.5`)
//! - `#104`: Bland–Altman percent mode rejects non-positive and non-finite pair averages

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    GroupedTable, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, OutlierSpec, PValueFormat, ResultCell,
    ResultSheet, ResultTab, SheetId, SubcolumnAggregation, SubcolumnMode, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableColumn, VariableRole,
    XyTable,
};
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::column::{anderson_darling, one_sample_wilcoxon, summarize_column};
use curvebook_stats::common::{p_cell_at_alpha, resolve_selected_indices, try_resolve_selected_indices};
use curvebook_stats::error::StatsError;
use curvebook_stats::multiplicity::MultiplicityProcedure;
use curvebook_stats::run_analysis;
use curvebook_stats::two_group::{bland_altman, mann_whitney_u, multiple_row_t_tests, unpaired_t_test, welch_t_test, wilcoxon_matched_pairs};

fn assert_close(actual: f64, expected: f64, tol: f64) {
    curvebook_testkit::assert_close(actual, expected, tol, "assert_close");
}

fn col_sheet(title: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c, (name, vals)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*name).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("data-col"), title, TableArchetype::Column(tbl)).unwrap()
}

fn grouped_sheet(title: &str, n_sets: usize, n_rows: usize, subs: usize, fill: impl Fn(usize, usize, usize) -> Option<f64>) -> DataSheet {
    let mut tbl = GroupedTable::new(SubcolumnMode::RawReplicates(subs), n_sets, n_rows).unwrap();
    for c in 0..n_sets {
        for r in 0..n_rows {
            for s in 0..subs {
                if let Some(v) = fill(c, r, s) {
                    tbl.data_sets[c].cells[r][s] = Cell::number(v);
                }
            }
        }
    }
    DataSheet::new(SheetId::new("data-grp"), title, TableArchetype::Grouped(tbl)).unwrap()
}

fn xy_multi_sheet(title: &str, xs: &[f64], ys_per_set: &[(&str, &[f64])], subs: usize) -> DataSheet {
    let n_rows = xs.len();
    let mut tbl = XyTable::new(SubcolumnMode::RawReplicates(subs), ys_per_set.len(), n_rows).unwrap();
    for (r, &x) in xs.iter().enumerate() {
        tbl.x_cells[r][0] = Cell::number(x);
    }
    for (c, (name, ys)) in ys_per_set.iter().enumerate() {
        tbl.data_sets[c].title = (*name).to_owned();
        for r in 0..n_rows {
            for s in 0..subs {
                tbl.data_sets[c].cells[r][s] = Cell::number(ys[r] + 0.01 * (s as f64));
            }
        }
    }
    DataSheet::new(SheetId::new("data-xy"), title, TableArchetype::Xy(tbl)).unwrap()
}

fn find_tab<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn find_row_index(tab: &ResultTab, row_label: &str) -> usize {
    tab.row_labels.iter().position(|r| r == row_label).unwrap_or_else(|| panic!("missing row `{row_label}` in tab `{}`", tab.id))
}

fn row_cell<'a>(tab: &'a ResultTab, row_label: &str, col_idx: usize) -> &'a ResultCell {
    let r = find_row_index(tab, row_label);
    &tab.rows[r][col_idx]
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number cell, got {other:?}"),
    }
}

fn scalar_val(sheet: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index == ds).unwrap_or_else(|| panic!("missing scalar {key:?} ds={ds:?}")).value
}

#[test]
fn audit_04_bland_altman_xy_compare_x_vs_y_modes() {
    let xs = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let y1 = [11.0, 19.0, 32.0, 39.0, 51.0, 58.0];
    let y2 = [100.0, 200.0, 300.0, 400.0, 500.0, 600.0];

    let one_y = xy_multi_sheet("BA 1Y", &xs, &[("Method B", &y1)], 1);
    let two_y = xy_multi_sheet("BA 2Y", &xs, &[("Method B", &y1), ("Method C", &y2)], 1);

    let expected_x_vs_y1 = bland_altman(&xs, &y1, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let expected_y1_vs_y2 = bland_altman(&y1, &y2, false, ConfidenceLevel::DEFAULT_95).unwrap();

    // Some(true) stays X vs Y[0] even when a second Y dataset exists.
    let spec_locked_x_y = AnalysisSpec::new(
        "ba-locked",
        one_y.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(true) }),
    )
    .unwrap();
    let res_1y = run_analysis(&one_y, &spec_locked_x_y).unwrap();
    let res_2y = run_analysis(&two_y, &spec_locked_x_y).unwrap();
    assert_close(scalar_val(&res_1y, "Bias", None), expected_x_vs_y1.bias, 1e-12);
    assert_close(scalar_val(&res_2y, "Bias", None), expected_x_vs_y1.bias, 1e-12);
    assert_eq!(find_tab(&res_2y, "bland-altman-summary").column_headers, vec![String::from("X vs. Method B")]);

    // Invalid column indices when Some(true) must return MissingColumn.
    let spec_bad_b = AnalysisSpec::new(
        "ba-bad-b",
        two_y.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 2, percent_difference: false, xy_compare_x_vs_y: Some(true) }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&two_y, &spec_bad_b), Err(StatsError::MissingColumn { index: 2 })));

    // Some(false) always compares Y[a] vs Y[b] and rejects 1-dataset XY tables.
    let spec_y_vs_y = AnalysisSpec::new(
        "ba-y-vs-y",
        one_y.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(false) }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&one_y, &spec_y_vs_y), Err(StatsError::MissingColumn { index: 1 })));
    let res_y1_y2 = run_analysis(&two_y, &spec_y_vs_y).unwrap();
    assert_close(scalar_val(&res_y1_y2, "Bias", None), expected_y1_vs_y2.bias, 1e-12);

    // None preserves legacy behavior: X vs Y[0] on 1-dataset, Y[0] vs Y[1] on 2-dataset.
    let spec_legacy = AnalysisSpec::new(
        "ba-legacy",
        one_y.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: None }),
    )
    .unwrap();
    assert_close(scalar_val(&run_analysis(&one_y, &spec_legacy).unwrap(), "Bias", None), expected_x_vs_y1.bias, 1e-12);
    assert_close(scalar_val(&run_analysis(&two_y, &spec_legacy).unwrap(), "Bias", None), expected_y1_vs_y2.bias, 1e-12);
}

#[test]
fn audit_07_multiple_row_t_tests_paired_and_selection_parity() {
    let paired_kind = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: true,
        nonparametric: false,
        assume_equal_sd: true,
        multiplicity: MultiplicitySpec::default(),
    });
    let unpaired_kind = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: false,
        nonparametric: false,
        assume_equal_sd: true,
        multiplicity: MultiplicitySpec::default(),
    });

    // Column table: 1, 3, and 5 selected columns fail with InvalidParameter("paired_selected_columns"), NOT InsufficientSampleSize.
    let col5 = col_sheet("Col 5", &[("C1", &[1.0, 2.0]), ("C2", &[1.2, 2.1]), ("C3", &[2.0, 3.0]), ("C4", &[2.2, 3.1]), ("C5", &[3.0, 4.0])]);
    for sel in [vec![0], vec![0, 1, 2], vec![0, 1, 2, 3, 4]] {
        let mut spec = AnalysisSpec::new("mr-col-odd", col5.id.clone(), paired_kind.clone()).unwrap();
        spec.selected_data_sets = sel.clone();
        let err = run_analysis(&col5, &spec).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - sel.len() as f64).abs() < 1e-9),
            "expected InvalidParameter for odd Column selection {sel:?}, got {err:?}"
        );
    }

    // MultipleVariables: 1 Label + 3 Continuous -> default selection has 3 numeric columns -> odd paired error.
    let mut mv = MultipleVariablesTable::new(0, 2).unwrap();
    let mut lbl = VariableColumn::new("RowName", 2).unwrap();
    lbl.cells = vec![Cell::text("Gene1"), Cell::text("Gene2")];
    lbl.lock_role(VariableRole::Label);
    let mut v1 = VariableColumn::new("A1", 2).unwrap();
    v1.cells = vec![Cell::number(1.0), Cell::number(2.0)];
    let mut v2 = VariableColumn::new("A2", 2).unwrap();
    v2.cells = vec![Cell::number(1.1), Cell::number(2.1)];
    let mut v3 = VariableColumn::new("B1", 2).unwrap();
    v3.cells = vec![Cell::number(2.0), Cell::number(3.0)];
    mv.variables = vec![lbl, v1, v2, v3];
    let mv_sheet = DataSheet::new(SheetId::new("mv-odd"), "MV 3+label", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec_mv_odd = AnalysisSpec::new("mr-mv-odd", mv_sheet.id.clone(), paired_kind.clone()).unwrap();
    let err_mv = run_analysis(&mv_sheet, &spec_mv_odd).unwrap_err();
    assert!(matches!(err_mv, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - 3.0).abs() < 1e-9));

    // Grouped and XY tables: empty selection across 3 datasets fails paired (!= 2), explicit != 2 fails both paired and unpaired.
    let grp3 = grouped_sheet("Grp 3", 3, 2, 3, |c, r, s| Some((c + 1) as f64 + 0.1 * (r as f64) + 0.05 * (s as f64)));
    let xy3 = xy_multi_sheet("XY 3", &[1.0, 2.0], &[("G1", &[1.0, 2.0]), ("G2", &[2.0, 3.0]), ("G3", &[3.0, 4.0])], 3);
    for sheet in [&grp3, &xy3] {
        let spec_empty_paired = AnalysisSpec::new("mr-3ds-paired", sheet.id.clone(), paired_kind.clone()).unwrap();
        assert!(matches!(
            run_analysis(sheet, &spec_empty_paired),
            Err(StatsError::InvalidParameter { name: "paired_selected_columns", value }) if (value - 3.0).abs() < 1e-9
        ));

        for sel in [vec![0], vec![0, 1, 2]] {
            let mut spec_sel_paired = AnalysisSpec::new("mr-sel-paired", sheet.id.clone(), paired_kind.clone()).unwrap();
            spec_sel_paired.selected_data_sets = sel.clone();
            assert!(matches!(run_analysis(sheet, &spec_sel_paired), Err(StatsError::InvalidParameter { name: "paired_selected_columns", .. })));

            let mut spec_sel_unpaired = AnalysisSpec::new("mr-sel-unpaired", sheet.id.clone(), unpaired_kind.clone()).unwrap();
            spec_sel_unpaired.selected_data_sets = sel;
            assert!(matches!(run_analysis(sheet, &spec_sel_unpaired), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
        }
    }

    // Grouped table paired replicate width mismatch -> DimensionMismatch.
    let mut asym_sheet = grouped_sheet("Asym", 2, 2, 3, |c, _, s| Some((c + 1) as f64 + s as f64));
    if let TableArchetype::Grouped(grp_asym) = &mut asym_sheet.archetype {
        for r in 0..2 {
            grp_asym.data_sets[1].cells[r].pop(); // width 2 vs 3
        }
    }
    let spec_asym = AnalysisSpec::new("mr-asym", asym_sheet.id.clone(), paired_kind).unwrap();
    assert!(matches!(run_analysis(&asym_sheet, &spec_asym), Err(StatsError::DimensionMismatch { expected: 3, actual: 2 })));

    // Equal-width replicates with a blank cell skip only that incomplete pair instead of shifting pairs.
    let grp_gap = grouped_sheet("Grp gap", 2, 1, 3, |c, _, s| match (c, s) {
        (0, 0) => None, // missing replicate 0 in group A
        (0, 1) => Some(10.0),
        (0, 2) => Some(20.0),
        (1, 0) => Some(999.0), // paired with missing -> must be ignored
        (1, 1) => Some(12.0),
        (1, 2) => Some(23.0),
        _ => None,
    });
    let spec_gap = AnalysisSpec::new(
        "mr-gap",
        grp_gap.id.clone(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            paired: true,
            nonparametric: false,
            assume_equal_sd: true,
            multiplicity: MultiplicitySpec::default(),
        }),
    )
    .unwrap();
    let res_gap = run_analysis(&grp_gap, &spec_gap).unwrap();
    let tab_gap = find_tab(&res_gap, "multiple-row-comparisons");
    let diff = cell_num(row_cell(tab_gap, "Row 1", 2));
    assert_close(diff, -2.5, 1e-12); // (10-12 + 20-23)/2 = -2.5 (if mis-paired with 999, diff would be huge)
}

#[test]
fn audit_21_exact_small_sample_hodges_lehmann_ci() {
    // 1. One-sample Wilcoxon: n = 4 (max achievable = 87.5%), n = 5 (93.75%), n = 10 (k=9 -> 95.1171875%).
    let w4 = one_sample_wilcoxon(&[1.0, 2.0, 4.0, 8.0], 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(w4.pseudomedian_ci, Some((1.0, 8.0)));
    assert_close(w4.ci_actual_confidence_percent.unwrap(), 87.5, 1e-9);

    let w5 = one_sample_wilcoxon(&[1.0, 2.0, 4.0, 8.0, 16.0], 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(w5.pseudomedian_ci, Some((1.0, 16.0)));
    assert_close(w5.ci_actual_confidence_percent.unwrap(), 93.75, 1e-9);

    let vals10 = [1.1, 2.3, 3.7, 5.2, 8.1, 11.4, 15.8, 20.9, 27.3, 35.6];
    let w10 = one_sample_wilcoxon(&vals10, 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(w10.ci_actual_confidence_percent.unwrap(), 100.0 * (1.0 - 2.0 * 25.0 / 1024.0), 1e-9);

    // 2. Wilcoxon matched pairs: n = 4 -> exact 87.5% + sheet warning/label/row/footnote.
    let s_w4 = col_sheet("Wilcoxon n=4", &[("Before", &[10.0, 20.0, 40.0, 80.0]), ("After", &[9.0, 18.0, 36.0, 72.0])]);
    let res_w4 = run_analysis(
        &s_w4,
        &AnalysisSpec::new(
            "wilc-4",
            s_w4.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::WilcoxonSignedRank,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: None,
                mv_response_column: None,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let tab_w4 = find_tab(&res_w4, "wilcoxon-paired");
    assert_close(cell_num(row_cell(tab_w4, "HL CI actual confidence (%)", 0)), 87.5, 1e-9);
    assert!(tab_w4.row_labels.iter().any(|l| l.contains("87.50% CI") && l.contains("requested 95%")));
    assert!(tab_w4.footnotes.iter().any(|f| f.contains("87.50%")));
    assert!(res_w4.checklist.iter().any(|c| c.assumption == "Hodges-Lehmann CI achieved coverage" && c.status == ChecklistStatus::Warn));

    // Direct function check matches sheet (TwoGroupMethod::WilcoxonSignedRank uses Pratt 1959 zero handling).
    let direct_w4 = wilcoxon_matched_pairs(&[10.0, 20.0, 40.0, 80.0], &[9.0, 18.0, 36.0, 72.0], true, ConfidenceLevel::DEFAULT_95, true).unwrap();
    assert_close(direct_w4.ci_actual_confidence_percent.unwrap(), 87.5, 1e-9);
    assert_close(cell_num(row_cell(tab_w4, "Sum of positive ranks (W+)", 0)), direct_w4.w_plus, 1e-12);

    // Paired input with exact zero differences: run_analysis WilcoxonSignedRank matches Pratt (true), not classic (false).
    let a_zero = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let b_zero = [10.0, 18.0, 26.0, 35.0, 44.0, 52.0];
    let s_wz = col_sheet("Wilcoxon Pratt Zeros", &[("Before", &a_zero), ("After", &b_zero)]);
    let res_wz = run_analysis(
        &s_wz,
        &AnalysisSpec::new(
            "wilc-pratt-z",
            s_wz.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::WilcoxonSignedRank,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: None,
                mv_response_column: None,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let pratt_wz = wilcoxon_matched_pairs(&a_zero, &b_zero, true, ConfidenceLevel::DEFAULT_95, true).unwrap();
    let classic_wz = wilcoxon_matched_pairs(&a_zero, &b_zero, true, ConfidenceLevel::DEFAULT_95, false).unwrap();
    let tab_wz = find_tab(&res_wz, "wilcoxon-paired");
    assert_close(cell_num(row_cell(tab_wz, "Sum of positive ranks (W+)", 0)), pratt_wz.w_plus, 1e-12);
    assert_close(cell_num(row_cell(tab_wz, "Sum of negative ranks (W-)", 0)), pratt_wz.w_minus, 1e-12);
    assert_ne!((pratt_wz.w_plus - classic_wz.w_plus).abs(), 0.0, "Pratt zero handling must retain zero ranks in W+ ranking");

    // 3. Mann-Whitney U: n_a=3, n_b=3 -> C(6,3)=20 -> max achievable 90.0%; n_a=8, n_b=8 -> exact DP >= 95%.
    let mw33 = mann_whitney_u(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0], true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(mw33.hodges_lehmann_ci, Some((-29.0, -7.0)));
    assert_close(mw33.ci_actual_confidence_percent.unwrap(), 90.0, 1e-9);

    let a8 = [1.1, 2.2, 3.4, 4.7, 6.1, 7.6, 9.2, 11.0];
    let b8 = [13.5, 15.8, 18.2, 21.0, 24.3, 27.9, 31.4, 36.2];
    let mw88 = mann_whitney_u(&a8, &b8, true, ConfidenceLevel::DEFAULT_95).unwrap();
    let pct88 = mw88.ci_actual_confidence_percent.unwrap();
    assert!((95.0..=96.0).contains(&pct88), "expected exact (8,8) coverage in [95, 96], got {pct88}");

    let s_mw88 = col_sheet("MW 8x8", &[("A", &a8), ("B", &b8)]);
    let res_mw88 = run_analysis(
        &s_mw88,
        &AnalysisSpec::new(
            "mw-88",
            s_mw88.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                method: TwoGroupMethod::MannWhitneyU,
                two_tailed: true,
                compute_effect_sizes: true,
                mv_group_column: None,
                mv_response_column: None,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let tab_mw88 = find_tab(&res_mw88, "mann-whitney");
    assert_close(cell_num(row_cell(tab_mw88, "HL CI actual confidence (%)", 0)), pct88, 1e-9);
    assert!(!res_mw88.checklist.iter().any(|c| c.assumption == "Hodges-Lehmann CI achieved coverage" && c.status == ChecklistStatus::Warn));
}

#[test]
fn audit_32_try_resolve_selected_indices_rejects_bad_selections() {
    assert_eq!(try_resolve_selected_indices(&[], 3).unwrap(), vec![0, 1, 2]);
    assert_eq!(try_resolve_selected_indices(&[2, 0], 3).unwrap(), vec![2, 0]);
    assert_eq!(resolve_selected_indices(&[99, 1], 3), vec![1]);
    assert!(matches!(try_resolve_selected_indices(&[3], 3), Err(StatsError::MissingColumn { index: 3 })));
    assert!(matches!(
        try_resolve_selected_indices(&[1, 1], 3),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 1.0).abs() < 1e-9
    ));

    let sheet = col_sheet("Sel check", &[("A", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("B", &[2.0, 3.0, 4.0, 5.0, 6.0])]);

    let kinds = vec![
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..TwoGroupSpec::default() }),
    ];

    for kind in kinds {
        let mut oob = AnalysisSpec::new("oob", sheet.id.clone(), kind.clone()).unwrap();
        oob.selected_data_sets = vec![0, 99];
        assert!(matches!(run_analysis(&sheet, &oob), Err(StatsError::MissingColumn { index: 99 })));

        let mut dup = AnalysisSpec::new("dup", sheet.id.clone(), kind).unwrap();
        dup.selected_data_sets = vec![0, 0];
        assert!(matches!(run_analysis(&sheet, &dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
    }
}

#[test]
fn audit_66_exclude_label_and_categorical_from_default_selections() {
    let mut mv = MultipleVariablesTable::new(0, 6).unwrap();
    let mut id_col = VariableColumn::new("SubjectID", 6).unwrap();
    id_col.cells = vec![Cell::number(101.0), Cell::number(102.0), Cell::number(103.0), Cell::number(104.0), Cell::number(105.0), Cell::number(106.0)];
    id_col.lock_role(VariableRole::Label);

    let mut grp_col = VariableColumn::new("Group", 6).unwrap();
    grp_col.cells = vec![Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Drug"), Cell::text("Drug"), Cell::text("Drug")];
    grp_col.refresh_inferred_role();

    let mut score_col = VariableColumn::new("Score", 6).unwrap();
    score_col.cells = vec![Cell::number(10.0), Cell::number(12.0), Cell::number(11.0), Cell::number(20.0), Cell::number(22.0), Cell::number(21.0)];

    mv.variables = vec![id_col, grp_col, score_col];
    let mv_sheet = DataSheet::new(SheetId::new("mv-66"), "MV Label/Cat", TableArchetype::MultipleVariables(mv)).unwrap();

    // 1. Descriptive stats default selection excludes numeric SubjectID (Label) & Group (Categorical).
    let desc_spec = AnalysisSpec::new("desc-mv", mv_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let desc_res = run_analysis(&mv_sheet, &desc_spec).unwrap();
    assert_eq!(find_tab(&desc_res, "desc-summary").column_headers, vec![String::from("Score")]);

    // 2. Outlier detection excludes SubjectID & Group (only variable index 2 scanned).
    let out_spec =
        AnalysisSpec::new("out-mv", mv_sheet.id.clone(), AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }))
            .unwrap();
    let out_res = run_analysis(&mv_sheet, &out_spec).unwrap();
    assert!(out_res.scalars.iter().all(|s| s.data_set_index == Some(2)));

    // 3. One-sample t-test & Wilcoxon test Score (mean = 16.0), NOT SubjectID (mean = 103.5).
    let one_t_spec = AnalysisSpec::new(
        "one-t-mv",
        mv_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 16.0 }, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    let one_t_res = run_analysis(&mv_sheet, &one_t_spec).unwrap();
    let one_t_tab = find_tab(&one_t_res, "one-sample-t");
    assert_eq!(one_t_tab.column_headers, vec![String::from("Score")]);
    assert_close(cell_num(row_cell(one_t_tab, "Sample mean", 0)), 16.0, 1e-12);
    assert_close(scalar_val(&one_t_res, "t", None), 0.0, 1e-12);

    // 4. Unpaired t-test groups Score by Group (Ctrl=11 vs Drug=21 => diff = -10).
    let unp_spec = AnalysisSpec::new(
        "unp-mv",
        mv_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    let unp_res = run_analysis(&mv_sheet, &unp_spec).unwrap();
    let unp_tab = find_tab(&unp_res, "two-group");
    assert_close(cell_num(row_cell(unp_tab, "Difference between means (A - B)", 0)), -10.0, 1e-12);

    // 5. Paired t-test must NOT pair numeric Label SubjectID with Score when only 1 Continuous column exists.
    let mut mv_no_cat = MultipleVariablesTable::new(0, 4).unwrap();
    let mut id_only = VariableColumn::new("SubjectID", 4).unwrap();
    id_only.cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)];
    id_only.lock_role(VariableRole::Label);
    let mut y_only = VariableColumn::new("Score", 4).unwrap();
    y_only.cells = vec![Cell::number(10.0), Cell::number(12.0), Cell::number(11.0), Cell::number(13.0)];
    mv_no_cat.variables = vec![id_only, y_only];
    let mv_no_cat_sheet = DataSheet::new(SheetId::new("mv-no-cat"), "MV Label+1", TableArchetype::MultipleVariables(mv_no_cat)).unwrap();
    let paired_spec = AnalysisSpec::new(
        "pair-mv",
        mv_no_cat_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&mv_no_cat_sheet, &paired_spec), Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })));
}

#[test]
fn audit_67_lognormal_unpaired_t_log_scale_labels() {
    let sheet = col_sheet("Lognormal", &[("Ctrl", &[2.0, 4.0, 8.0, 16.0]), ("Trt", &[4.0, 8.0, 16.0, 32.0])]);
    let spec = AnalysisSpec::new(
        "lognorm-t",
        sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::LognormalUnpairedT,
            two_tailed: true,
            compute_effect_sizes: true,
            mv_group_column: None,
            mv_response_column: None,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let tab = find_tab(&res, "two-group");

    for required in [
        "Mean of ln(Ctrl)",
        "Mean of ln(Trt)",
        "Difference of ln means (ln A - ln B)",
        "SE of ln difference",
        "95% CI of ln difference",
        "F-test to compare ln variances (F)",
        "F-test P value (ln scale)",
        "Geometric mean ratio (A / B)",
        "95% CI of geometric ratio",
        "Cohen's d (ln scale)",
        "95% CI of Cohen's d (ln scale)",
        "Hedges' g (ln scale)",
        "95% CI of Hedges' g (ln scale)",
        "Glass's Delta (ln scale)",
        "Glass's Delta standardizing group",
        "R squared (ln scale)",
    ] {
        assert!(tab.row_labels.iter().any(|l| l == required), "missing expected lognormal row `{required}` in {:?}", tab.row_labels);
    }

    assert_close(cell_num(row_cell(tab, "Geometric mean ratio (A / B)", 0)), 0.5, 1e-12);
    assert_eq!(row_cell(tab, "Glass's Delta standardizing group", 0), &ResultCell::Text(String::from("ln(Ctrl)")));
    assert!(tab.footnotes.iter().any(|f| f.contains("ln(Y)") && f.contains("geometric mean ratio")));
}

#[test]
fn audit_68_custom_percentiles_type7_consistent_and_emitted() {
    let vals = [2.0, 4.0, 4.0, 8.0, 16.0, 32.0];
    let direct = summarize_column("X", &vals, 0, ConfidenceLevel::DEFAULT_95, Some(90.0)).unwrap();
    assert_eq!(direct.custom_lower_percentile, Some((10.0, 3.0)));
    assert_eq!(direct.custom_upper_percentile, Some((90.0, 24.0)));

    let sheet = col_sheet("Custom pct", &[("X", &vals)]);
    let spec90 = AnalysisSpec::new(
        "desc-p90",
        sheet.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::RowMeans,
            custom_upper_percentile: Some(90.0),
            run_normality_battery: false,
            hypothetical_value: None,
        }),
    )
    .unwrap();
    let res90 = run_analysis(&sheet, &spec90).unwrap();
    let tab90 = find_tab(&res90, "desc-summary");
    assert_close(cell_num(row_cell(tab90, "10% Percentile (Type 7)", 0)), 3.0, 1e-12);
    assert_close(cell_num(row_cell(tab90, "90% Percentile (Type 7)", 0)), 24.0, 1e-12);
    assert!(tab90.footnotes.iter().any(|f| f.contains("Type 7") && f.contains("Type 6")));

    // Requesting 75% custom percentile coexists with standard Type 6 quartiles without label collision.
    let spec75 = AnalysisSpec::new(
        "desc-p75",
        sheet.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::RowMeans,
            custom_upper_percentile: Some(75.0),
            run_normality_battery: false,
            hypothetical_value: None,
        }),
    )
    .unwrap();
    let res75 = run_analysis(&sheet, &spec75).unwrap();
    let tab75 = find_tab(&res75, "desc-summary");
    assert!(tab75.row_labels.iter().any(|l| l == "25% Percentile"));
    assert!(tab75.row_labels.iter().any(|l| l == "75% Percentile"));
    assert!(tab75.row_labels.iter().any(|l| l == "25% Percentile (Type 7)"));
    assert!(tab75.row_labels.iter().any(|l| l == "75% Percentile (Type 7)"));
}

#[test]
fn descriptive_tab_emits_geometric_harmonic_quadratic_mean_cis() {
    let pos = [10.0, 12.0, 14.0, 16.0, 18.0];
    let non_pos = [-2.0, 0.0, 3.0, 5.0, 7.0];
    let single = [42.0];

    let direct_pos = summarize_column("Pos", &pos, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let direct_non_pos = summarize_column("NonPos", &non_pos, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
    let (gm_lo, gm_hi) = direct_pos.geometric_mean_ci.expect("positive geometric mean CI");
    let (hm_lo, hm_hi) = direct_pos.harmonic_mean_ci.expect("positive harmonic mean CI");
    let (qm_lo, qm_hi) = direct_pos.quadratic_mean_ci.expect("positive quadratic mean CI");
    let (np_qm_lo, np_qm_hi) = direct_non_pos.quadratic_mean_ci.expect("non-positive quadratic mean CI");

    let sheet = col_sheet("Generalized Mean CIs", &[("Pos", &pos), ("NonPos", &non_pos), ("Single", &single)]);
    let spec = AnalysisSpec::new("desc-gen-ci", sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let tab = find_tab(&res, "desc-summary");

    // Positive column: all three CI rows are populated and match summarize_column().
    match row_cell(tab, "95% CI of Geometric mean", 0) {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lower), upper: CiBound::Finite(upper) } => {
            assert_close(*lower, gm_lo, 1e-12);
            assert_close(*upper, gm_hi, 1e-12);
        }
        other => panic!("expected ConfidenceInterval for Geometric mean CI, got {other:?}"),
    }
    match row_cell(tab, "95% CI of Harmonic mean", 0) {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lower), upper: CiBound::Finite(upper) } => {
            assert_close(*lower, hm_lo, 1e-12);
            assert_close(*upper, hm_hi, 1e-12);
        }
        other => panic!("expected ConfidenceInterval for Harmonic mean CI, got {other:?}"),
    }
    match row_cell(tab, "95% CI of Quadratic mean (RMS)", 0) {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lower), upper: CiBound::Finite(upper) } => {
            assert_close(*lower, qm_lo, 1e-12);
            assert_close(*upper, qm_hi, 1e-12);
        }
        other => panic!("expected ConfidenceInterval for Quadratic mean CI, got {other:?}"),
    }

    // Non-positive column: geometric & harmonic CIs are Empty due to domain guards, quadratic CI is populated.
    assert_eq!(row_cell(tab, "95% CI of Geometric mean", 1), &ResultCell::Empty);
    assert_eq!(row_cell(tab, "95% CI of Harmonic mean", 1), &ResultCell::Empty);
    match row_cell(tab, "95% CI of Quadratic mean (RMS)", 1) {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lower), upper: CiBound::Finite(upper) } => {
            assert_close(*lower, np_qm_lo, 1e-12);
            assert_close(*upper, np_qm_hi, 1e-12);
        }
        other => panic!("expected ConfidenceInterval for NonPos Quadratic mean CI, got {other:?}"),
    }

    // Single-observation column: all three CIs require n >= 2 and emit Empty.
    assert_eq!(row_cell(tab, "95% CI of Geometric mean", 2), &ResultCell::Empty);
    assert_eq!(row_cell(tab, "95% CI of Harmonic mean", 2), &ResultCell::Empty);
    assert_eq!(row_cell(tab, "95% CI of Quadratic mean (RMS)", 2), &ResultCell::Empty);
}

#[test]
fn audit_69_glass_delta_no_sd_b_fallback_and_group_label() {
    let zero_a = [5.0, 5.0, 5.0];
    let var_b = [1.0, 2.0, 3.0];

    let student = unpaired_t_test(&zero_a, &var_b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(student.effect_sizes.glass_delta, None);
    assert_eq!(student.effect_sizes.glass_delta_ci, None);

    let welch = welch_t_test(&zero_a, &var_b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(welch.effect_sizes.glass_delta, None);
    assert_eq!(welch.effect_sizes.glass_delta_ci, None);

    let rev = unpaired_t_test(&var_b, &zero_a, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(rev.effect_sizes.glass_delta.unwrap(), -3.0, 1e-12);

    let sheet = col_sheet("Glass", &[("Placebo", &zero_a), ("Drug", &var_b)]);
    let spec = AnalysisSpec::new("glass-spec", sheet.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    let tab = find_tab(&res, "two-group");

    // Preserve exact "Glass's Delta" numeric row label for downstream consumers, emit Empty cell & companion label row.
    assert_eq!(row_cell(tab, "Glass's Delta", 0), &ResultCell::Empty);
    assert_eq!(row_cell(tab, "Glass's Delta standardizing group", 0), &ResultCell::Text(String::from("Placebo")));
    assert!(tab.footnotes.iter().any(|f| f.contains("Placebo")));
    assert!(res.checklist.iter().any(|c| c.assumption == "Glass's Delta standardizing SD"
        && c.status == ChecklistStatus::Info
        && c.explanation.contains("Placebo")
        && c.explanation.contains("Drug")));
}

#[test]
fn audit_81_multiple_row_t_tests_rejects_anova_procedures() {
    let labels = vec![String::from("R1"), String::from("R2")];
    let rows_a = vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]];
    let rows_b = vec![vec![2.0, 3.0, 4.0], vec![8.0, 9.0, 10.0]];

    let disallowed_procs = [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::DunnRankSum,
        MultiplicityProcedure::LinearTrend,
        MultiplicityProcedure::TamhaneT2,
    ];
    for proc in disallowed_procs {
        assert!(
            matches!(multiple_row_t_tests(&labels, &rows_a, &rows_b, false, proc, 0.05), Err(StatsError::IncompatibleAnalysisSpec { .. })),
            "expected IncompatibleAnalysisSpec for direct procedure {proc:?}"
        );
    }

    let sheet = grouped_sheet("MR 81", 2, 2, 3, |c, r, s| Some((c * 5 + r * 2) as f64 + 0.1 * (s as f64)));
    let disallowed_methods = [
        MultiplicityMethod::TukeyHsd,
        MultiplicityMethod::NewmanKeuls,
        MultiplicityMethod::Dunnett { control_column: 0 },
        MultiplicityMethod::DunnettT3,
        MultiplicityMethod::GamesHowell,
        MultiplicityMethod::DunnRankSum,
        MultiplicityMethod::LinearTrend,
        MultiplicityMethod::TamhaneT2,
    ];
    for method in disallowed_methods {
        let spec = AnalysisSpec::new(
            "mr-81",
            sheet.id.clone(),
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
                ..MultipleRowTestsSpec::default()
            }),
        )
        .unwrap();
        assert!(
            matches!(run_analysis(&sheet, &spec), Err(StatsError::IncompatibleAnalysisSpec { .. })),
            "expected IncompatibleAnalysisSpec for spec method {method:?}"
        );
    }

    let allowed_methods = [
        MultiplicityMethod::None,
        MultiplicityMethod::FisherLsd,
        MultiplicityMethod::Bonferroni,
        MultiplicityMethod::Sidak,
        MultiplicityMethod::HolmSidak,
        MultiplicityMethod::BenjaminiHochbergFdr,
        MultiplicityMethod::BenjaminiYekutieliFdr,
        MultiplicityMethod::TwoStageBkyFdr,
    ];
    for method in allowed_methods {
        let spec = AnalysisSpec::new(
            "mr-ok",
            sheet.id.clone(),
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
                ..MultipleRowTestsSpec::default()
            }),
        )
        .unwrap();
        assert!(run_analysis(&sheet, &spec).is_ok(), "expected allowed method {method:?} to succeed");
    }
}

#[test]
fn audit_82_p_value_stars_respect_analysis_alpha_and_le_boundary() {
    let stars = |p: f64, fmt: PValueFormat, alpha: f64| match p_cell_at_alpha(p, fmt, alpha) {
        ResultCell::PValue { stars, .. } => stars,
        other => panic!("expected PValue, got {other:?}"),
    };

    assert_eq!(stars(0.0369, PValueFormat::FourDecimal, 0.01), "ns");
    assert_eq!(stars(0.01, PValueFormat::FourDecimal, 0.01), "*");
    assert_eq!(stars(0.010_000_1, PValueFormat::FourDecimal, 0.01), "ns");
    assert_eq!(stars(0.002, PValueFormat::FourDecimal, 0.01), "**");
    assert_eq!(stars(0.05, PValueFormat::Apa, 0.05), "*");
    assert_eq!(stars(0.050_000_1, PValueFormat::Apa, 0.05), "ns");

    // Two groups whose unpaired t-test p-value sits cleanly in (0.01, 0.05).
    let a = [1.0, 2.0, 3.0, 4.0, 5.0];
    let b = [3.5, 4.5, 5.5, 6.5, 7.5];
    let t_out = unpaired_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!((0.01..0.05).contains(&t_out.p_value), "expected p in (0.01, 0.05), got {}", t_out.p_value);

    let sheet = col_sheet("Alpha stars", &[("A", &a), ("B", &b)]);
    let mut spec95 = AnalysisSpec::new("p95", sheet.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    spec95.set_confidence_percent(95.0).unwrap();
    let res95 = run_analysis(&sheet, &spec95).unwrap();
    let tab95 = find_tab(&res95, "two-group");
    match row_cell(tab95, "P value", 0) {
        ResultCell::PValue { stars, .. } => assert_eq!(stars, "*"),
        other => panic!("expected PValue, got {other:?}"),
    }

    let mut spec99 = AnalysisSpec::new("p99", sheet.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    spec99.set_confidence_percent(99.0).unwrap();
    let res99 = run_analysis(&sheet, &spec99).unwrap();
    let tab99 = find_tab(&res99, "two-group");
    match row_cell(tab99, "P value", 0) {
        ResultCell::PValue { stars, .. } => assert_eq!(stars, "ns"),
        other => panic!("expected PValue, got {other:?}"),
    }

    // DescriptiveStats one-sample tab at 99% confidence also suppresses stars for 0.01 < p < 0.05.
    let one_col = col_sheet("One sample stars", &[("X", &[1.0, 2.0, 3.0, 4.0, 5.0])]);
    let mut desc99 = AnalysisSpec::new(
        "desc99",
        one_col.id.clone(),
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::RowMeans,
            custom_upper_percentile: None,
            run_normality_battery: false,
            hypothetical_value: Some(1.0),
        }),
    )
    .unwrap();
    desc99.set_confidence_percent(99.0).unwrap();
    let res_desc99 = run_analysis(&one_col, &desc99).unwrap();
    match row_cell(find_tab(&res_desc99, "one-sample"), "One-sample t P", 0) {
        ResultCell::PValue { raw, stars, .. } => {
            assert!((0.01..0.05).contains(raw), "expected p in (0.01, 0.05), got {raw}");
            assert_eq!(stars, "ns");
        }
        other => panic!("expected PValue, got {other:?}"),
    }

    // Multiple row t-tests: raw P tracks confidence alpha (0.05), adjusted P tracks multiplicity threshold (0.01).
    let grp_mr = grouped_sheet("MR 82", 2, 1, 5, |c, _, s| if c == 0 { Some(a[s]) } else { Some(b[s]) });
    let mut spec_mr = AnalysisSpec::new(
        "mr-82",
        grp_mr.id.clone(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.01, report_confidence_intervals: false },
            ..MultipleRowTestsSpec::default()
        }),
    )
    .unwrap();
    spec_mr.set_confidence_percent(95.0).unwrap();
    let res_mr = run_analysis(&grp_mr, &spec_mr).unwrap();
    let tab_mr = find_tab(&res_mr, "multiple-row-comparisons");
    match row_cell(tab_mr, "Row 1", 5) {
        ResultCell::PValue { stars, .. } => assert_eq!(stars, "*"),
        other => panic!("expected raw PValue, got {other:?}"),
    }
    match row_cell(tab_mr, "Row 1", 6) {
        ResultCell::PValue { stars, .. } => assert_eq!(stars, "ns"),
        other => panic!("expected adjusted PValue, got {other:?}"),
    }
}

#[test]
fn audit_93_mv_persisted_columns_stable_under_calculated_var() {
    let mut mv = MultipleVariablesTable::new(0, 6).unwrap();
    let mut grp = VariableColumn::new("Group", 6).unwrap();
    grp.cells = vec![Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Trt"), Cell::text("Trt"), Cell::text("Trt")];
    grp.refresh_inferred_role();

    let mut base = VariableColumn::new("Baseline", 6).unwrap();
    base.cells = vec![Cell::number(10.0), Cell::number(11.0), Cell::number(12.0), Cell::number(20.0), Cell::number(21.0), Cell::number(22.0)];

    let mut follow = VariableColumn::new("Followup", 6).unwrap();
    follow.cells = vec![Cell::number(100.0), Cell::number(101.0), Cell::number(102.0), Cell::number(100.0), Cell::number(101.0), Cell::number(102.0)];

    mv.variables = vec![grp, base, follow];
    let sheet_before = DataSheet::new(SheetId::new("mv-93"), "MV 93", TableArchetype::MultipleVariables(mv.clone())).unwrap();

    let spec_unp = AnalysisSpec::new(
        "mv-unp-pinned",
        sheet_before.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::UnpairedT,
            two_tailed: true,
            compute_effect_sizes: true,
            mv_group_column: Some(0),
            mv_response_column: Some(1),
        }),
    )
    .unwrap();
    let spec_one = AnalysisSpec::new(
        "mv-one-pinned",
        sheet_before.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 },
            two_tailed: true,
            compute_effect_sizes: false,
            mv_group_column: None,
            mv_response_column: Some(1),
        }),
    )
    .unwrap();

    let unp_before = run_analysis(&sheet_before, &spec_unp).unwrap();
    let one_before = run_analysis(&sheet_before, &spec_one).unwrap();

    // Append an active Calculated variable; persisted MV specs must keep targeting Baseline (index 1).
    let mut delta = VariableColumn::new("Delta", 6).unwrap();
    delta.set_calculated_formula("=[Followup]-[Baseline]");
    delta.cells = vec![Cell::number(90.0), Cell::number(90.0), Cell::number(90.0), Cell::number(80.0), Cell::number(80.0), Cell::number(80.0)];
    mv.variables.push(delta);
    let sheet_after = DataSheet::new(SheetId::new("mv-93"), "MV 93", TableArchetype::MultipleVariables(mv)).unwrap();

    let unp_after = run_analysis(&sheet_after, &spec_unp).unwrap();
    let one_after = run_analysis(&sheet_after, &spec_one).unwrap();
    assert_close(
        cell_num(row_cell(find_tab(&unp_after, "two-group"), "Difference between means (A - B)", 0)),
        cell_num(row_cell(find_tab(&unp_before, "two-group"), "Difference between means (A - B)", 0)),
        1e-12,
    );
    assert_close(scalar_val(&unp_after, "t", None), scalar_val(&unp_before, "t", None), 1e-12);
    assert_close(cell_num(row_cell(find_tab(&one_after, "one-sample-t"), "Sample mean", 0)), 16.0, 1e-12);
    assert_eq!(find_tab(&one_after, "one-sample-t").column_headers, find_tab(&one_before, "one-sample-t").column_headers);
}

#[test]
fn audit_100_anderson_darling_monotone_for_large_a2_star() {
    // Repro from audit #100: 999 zeros + single 1.0 produces huge A2* well past 153.5 quadratic vertex.
    let mut vals = vec![0.0; 999];
    vals.push(1.0);
    let ad = anderson_darling(&vals, 0.05).unwrap();
    assert!(ad.statistic > 153.5, "expected large A2*, got {}", ad.statistic);
    assert!(ad.p_value < 1e-6, "expected tiny P value, got {}", ad.p_value);
    assert!(!ad.passed, "degenerate point-mass + outlier must fail normality");

    // Heavy-tailed / skewed large samples remain monotonically tiny and never rebound toward 1.0.
    let mut prev_p = 1.0;
    for n in [30_usize, 100, 300, 1000, 3000] {
        let mut skew = vec![0.0; n - 1];
        skew.push(10.0);
        let outcome = anderson_darling(&skew, 0.05).unwrap();
        assert!(outcome.p_value <= prev_p + 1e-15, "AD p_value increased from {prev_p} to {} at n={n}", outcome.p_value);
        assert!(outcome.p_value < 1e-6);
        assert!(!outcome.passed);
        prev_p = outcome.p_value;
    }
}

#[test]
fn audit_104_bland_altman_rejects_nonpositive_and_nonfinite_percent_averages() {
    // Negative pair averages in percent mode must be rejected cleanly.
    let neg_a = [-3.0, -5.0, -2.0, -4.0];
    let neg_b = [-2.0, -4.5, -1.0, -3.6];
    let err_neg = bland_altman(&neg_a, &neg_b, true, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_neg, StatsError::InvalidParameter { name: "average", value } if value < 0.0));

    // Exact zero pair average in percent mode must be rejected cleanly.
    let zero_avg_a = [2.0, 4.0, -3.0, 5.0];
    let zero_avg_b = [2.0, 4.0, 3.0, 5.0];
    let err_zero = bland_altman(&zero_avg_a, &zero_avg_b, true, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_zero, StatsError::InvalidParameter { name: "average", value } if value == 0.0));

    // Cancelling f64::MAX / -f64::MAX gives average = 0.0 -> rejected cleanly.
    let zero_cancel_a = [f64::MAX, 1.0, 2.0, 3.0];
    let zero_cancel_b = [-f64::MAX, 1.0, 2.0, 3.0];
    let err_cancel = bland_altman(&zero_cancel_a, &zero_cancel_b, true, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_cancel, StatsError::InvalidParameter { name: "average", value } if value == 0.0));

    // Non-finite pair observations fail validation cleanly.
    assert!(matches!(
        bland_altman(&[f64::NAN, 1.0], &[1.0, 2.0], true, ConfidenceLevel::DEFAULT_95),
        Err(StatsError::Math(MathError::DomainError { .. }))
    ));
    assert!(matches!(
        bland_altman(&[f64::INFINITY, 1.0], &[1.0, 2.0], true, ConfidenceLevel::DEFAULT_95),
        Err(StatsError::Math(MathError::DomainError { .. }))
    ));

    // Absolute mode remains valid on negative measurements.
    assert!(bland_altman(&neg_a, &neg_b, false, ConfidenceLevel::DEFAULT_95).is_ok());

    // End-to-end ResultSheet runner propagates InvalidParameter cleanly.
    let sheet = col_sheet("BA neg", &[("A", &neg_a), ("B", &neg_b)]);
    let spec = AnalysisSpec::new(
        "ba-neg-pct",
        sheet.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: true, xy_compare_x_vs_y: None }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &spec), Err(StatsError::InvalidParameter { name: "average", .. })));
}
