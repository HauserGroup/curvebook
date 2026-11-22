//! Regression and reference-oracle tests for Phase 3 SEC-05 remediation (`FA-SEC05-05`..`FA-SEC05-13`).
//!
//! Covers:
//! - `FA-SEC05-05`: Multiple row-wise nonparametric tests emit `f64::INFINITY` / `ResultCell::Empty` for `df`.
//! - `FA-SEC05-06`: `TwoGroupMethod::WilcoxonSignedRank` and paired nonparametric multiple-row tests
//!   dispatch Pratt (1959) zero handling, exact DP runs when `n_zeros == 0 && n_eff <= 25` without ties,
//!   classic `wilcoxon_matched_pairs(..., false)` remains intact, and Pratt handling is disclosed.
//! - `FA-SEC05-07`: XY Bland-Altman pairs all replicate subcolumns in `Y[A]` vs `Y[B]`, reports skipped/text
//!   diagnostics in `X` vs `Y[0]`, and rejects nonpositive measurements (`a <= 0 || b <= 0`) in percent mode.
//! - `FA-SEC05-08`: One-sided `DetectableEffect` solvers support `target_power <= alpha` cleanly across families.
//! - `FA-SEC05-09`: `mann_whitney_u` returns `p = 1.0` for all-identical samples when `na + nb > 18`.
//! - `FA-SEC05-10`: `compare_groups_*`, `compare_dunn_ranks`, and `linear_trend_test` validate `mse`, `df_error`,
//!   and per-group `(n, mean, sd)` domains without clamping `NaN`/negative variances.
//! - `FA-SEC05-11`: `run_roc_analysis` rejects ambiguous multi-column selections, warns on ignored text, and
//!   sanitizes non-finite floats; `run_multiple_row_t_tests_analysis` warns on ignored text / incomplete pairs.
//! - `FA-SEC05-12`: Significance stars honor configured `alpha` / `threshold` across contingency, multiplicity,
//!   P-value stack, and ROC tabs; `one_sample_ks_normal_at_alpha` honors configurable `alpha`.
//! - `FA-SEC05-13`: Documented semantics verified for `BarnardExactResult::statistic`, `NormalityTestOutcome::passed`,
//!   and `adjust_p_values` vs `try_adjust_p_values`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec,
    ContingencyTable, DataSheet, GroupedTable, MultipleRowTestsSpec, MultipleVariablesTable, MultiplicitySpec, PValueFormat, PValueStackMethod,
    PValueStackSpec, PartsOfWholeTable, ResultCell, ResultSheet, ResultTab, RocCiMethod, RocSpec, SheetId, SubcolumnMode, TableArchetype,
    TwoGroupMethod, TwoGroupSpec, VariableColumn, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::categorical::{BarnardOrdering, barnard_exact_2x2_with_ordering};
use curvebook_stats::column::{one_sample_ks_normal, one_sample_ks_normal_at_alpha, shapiro_wilk};
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_p_values, build_multiplicity_tab, compare_dunn_ranks, compare_groups_with_procedure,
    linear_trend_test, try_adjust_p_values,
};
use curvebook_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode,
    TwoProportionsMethod, solve_power,
};
use curvebook_stats::two_group::{bland_altman, mann_whitney_u, wilcoxon_matched_pairs};
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected} +/- {tol}, got {actual}");
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn scalar_val(sheet: &ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index.is_none()).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number, got {other:?}"),
    }
}

fn cell_p_stars(cell: &ResultCell) -> (f64, &str) {
    match cell {
        ResultCell::PValue { raw, stars, .. } => (*raw, stars.as_str()),
        other => panic!("expected PValue, got {other:?}"),
    }
}

fn col_sheet_cells(cols: &[(&str, &[Cell])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, cell) in vals.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = cell.clone();
        }
    }
    DataSheet::new(SheetId::new("col"), "Columns", TableArchetype::Column(tbl)).unwrap()
}

fn col_sheet_nums(cols: &[(&str, &[f64])]) -> DataSheet {
    let owned: Vec<(&str, Vec<Cell>)> = cols.iter().map(|&(t, v)| (t, v.iter().copied().map(Cell::number).collect())).collect();
    let refs: Vec<(&str, &[Cell])> = owned.iter().map(|(t, v)| (*t, v.as_slice())).collect();
    col_sheet_cells(&refs)
}

fn grouped_sheet_cells(n_rows: usize, n_subs: usize, title_a: &str, a_rows: &[Vec<Cell>], title_b: &str, b_rows: &[Vec<Cell>]) -> DataSheet {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(n_subs), 2, n_rows).unwrap();
    grp.data_sets[0].title = title_a.to_owned();
    grp.data_sets[1].title = title_b.to_owned();
    for r in 0..n_rows {
        grp.row_titles[r] = format!("Gene{}", r + 1);
        grp.data_sets[0].cells[r] = a_rows[r].clone();
        grp.data_sets[1].cells[r] = b_rows[r].clone();
    }
    DataSheet::new(SheetId::new("grp"), "Grouped", TableArchetype::Grouped(grp)).unwrap()
}

// ============================================================================
// FA-SEC05-05: Multiple row-wise nonparametric tests emit Empty df (f64::INFINITY)
// ============================================================================

#[test]
fn fa_sec05_05_multiple_row_nonparametric_emits_empty_df() {
    let a = vec![
        vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0), Cell::number(5.0), Cell::number(6.0)],
        vec![Cell::number(10.0), Cell::number(11.0), Cell::number(12.0), Cell::number(13.0), Cell::number(14.0), Cell::number(15.0)],
    ];
    let b = vec![
        vec![Cell::number(7.0), Cell::number(8.0), Cell::number(9.0), Cell::number(10.0), Cell::number(11.0), Cell::number(12.0)],
        vec![Cell::number(10.0), Cell::number(12.0), Cell::number(14.0), Cell::number(16.0), Cell::number(18.0), Cell::number(20.0)],
    ];
    let sheet = grouped_sheet_cells(2, 6, "Ctrl", &a, "Trt", &b);

    for paired in [false, true] {
        let spec = AnalysisSpec::new(
            SheetId::new("mr-np"),
            sheet.id.clone(),
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired, nonparametric: true, ..MultipleRowTestsSpec::default() }),
        )
        .unwrap();
        let res = run_analysis(&sheet, &spec).unwrap();
        let tab = tab_by_id(&res, "multiple-row-comparisons");
        assert_eq!(tab.column_headers[4], "df");
        for (r_idx, row) in tab.rows.iter().enumerate() {
            assert_eq!(row[4], ResultCell::Empty, "paired={paired} row {r_idx} must emit Empty df for rank test");
        }
        assert!(tab.footnotes.iter().any(|f| f.contains("df is omitted")));
    }

    // Parametric unpaired/paired rows retain finite numeric df.
    let spec_param = AnalysisSpec::new(
        SheetId::new("mr-p"),
        sheet.id.clone(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: false, nonparametric: false, ..MultipleRowTestsSpec::default() }),
    )
    .unwrap();
    let res_param = run_analysis(&sheet, &spec_param).unwrap();
    let tab_param = tab_by_id(&res_param, "multiple-row-comparisons");
    assert_close(cell_num(&tab_param.rows[0][4]), 10.0, 1e-12, "parametric unpaired df = 10");
}

// ============================================================================
// FA-SEC05-06: TwoGroupMethod::WilcoxonSignedRank uses Pratt (1959) zero handling
// ============================================================================

#[test]
fn fa_sec05_06_wilcoxon_spec_uses_pratt_zeros_and_exact_dp_when_zero_free() {
    // 1. Data with exact zero differences: TwoGroupMethod::WilcoxonSignedRank matches Pratt (true), not classic (false).
    let a = [1.0, 2.0, 3.0, 4.0, 5.0];
    let b = [1.0, 1.0, 1.0, 2.0, 3.0];
    let conf = ConfidenceLevel::DEFAULT_95;
    let pratt_direct = wilcoxon_matched_pairs(&a, &b, true, conf, true).unwrap();
    let classic_direct = wilcoxon_matched_pairs(&a, &b, true, conf, false).unwrap();
    assert_ne!(pratt_direct.n_effective, classic_direct.n_effective);
    assert!((pratt_direct.p_value - classic_direct.p_value).abs() > 1e-4);

    let sheet = col_sheet_nums(&[("A", &a), ("B", &b)]);
    let spec = AnalysisSpec::new(
        SheetId::new("wil-pratt"),
        sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WilcoxonSignedRank, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_close(scalar_val(&res, "PValue"), pratt_direct.p_value, 1e-12, "TwoGroup Wilcoxon Pratt P");
    let tab = tab_by_id(&res, "wilcoxon-paired");
    assert_eq!(tab.rows[1][0], ResultCell::Boolean(false)); // Exact P used = false when zeros exist
    assert_close(cell_num(&tab.rows[2][0]), pratt_direct.w_plus, 1e-12, "Pratt W+");
    assert_close(cell_num(&tab.rows[3][0]), pratt_direct.w_minus, 1e-12, "Pratt W-");
    assert!(tab.footnotes.iter().any(|f| f.contains("Pratt's (1959)")));
    assert!(res.checklist.iter().any(|c| c.explanation.contains("Pratt (1959)")));

    // Direct classic library API stays intact (untied nonzero sample uses exact DP).
    let a_cl = [10.0, 11.0, 13.0, 16.0, 20.0];
    let b_cl = [10.0, 10.0, 11.0, 13.0, 16.0];
    let classic_untied = wilcoxon_matched_pairs(&a_cl, &b_cl, true, conf, false).unwrap();
    assert_eq!(classic_untied.n_effective, 4);
    assert!(classic_untied.exact_p_used);
    let pratt_tied_zero = wilcoxon_matched_pairs(&a_cl, &b_cl, true, conf, true).unwrap();
    assert_eq!(pratt_tied_zero.n_effective, 5);
    assert!(!pratt_tied_zero.exact_p_used);

    // 2. Data with NO zero differences and NO ties (n <= 25): Pratt and classic are identical and exact DP runs!
    let a_nozero = [10.1, 11.3, 12.5, 14.2, 15.8, 17.1, 18.9, 20.6];
    let b_nozero = [9.0, 10.1, 11.1, 12.6, 14.0, 15.1, 16.7, 18.2];
    let pratt_nz = wilcoxon_matched_pairs(&a_nozero, &b_nozero, true, conf, true).unwrap();
    let classic_nz = wilcoxon_matched_pairs(&a_nozero, &b_nozero, true, conf, false).unwrap();
    assert!(pratt_nz.exact_p_used, "exact DP must run in Pratt mode when n_zeros == 0 and no ties");
    assert_close(pratt_nz.p_value, classic_nz.p_value, 1e-15, "zero-free Pratt == classic exact P");
    assert_close(pratt_nz.p_value, 1.0 / 128.0, 1e-15, "exact DP P = 1/128");

    // 3. MultipleRowTestsSpec (paired + nonparametric) also uses Pratt (1959) and discloses it.
    let grp =
        grouped_sheet_cells(1, 5, "Pre", &[a.iter().copied().map(Cell::number).collect()], "Post", &[b.iter().copied().map(Cell::number).collect()]);
    let mr_spec = AnalysisSpec::new(
        SheetId::new("mr-pratt"),
        grp.id.clone(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: true, nonparametric: true, ..MultipleRowTestsSpec::default() }),
    )
    .unwrap();
    let mr_res = run_analysis(&grp, &mr_spec).unwrap();
    let mr_tab = tab_by_id(&mr_res, "multiple-row-comparisons");
    assert_close(cell_p_stars(&mr_tab.rows[0][5]).0, pratt_direct.p_value, 1e-12, "MultipleRow paired nonparametric uses Pratt P");
    assert!(mr_tab.footnotes.iter().any(|f| f.contains("Pratt (1959)")));
}

// ============================================================================
// FA-SEC05-07: XY Bland-Altman replicates, missing/text tracking, percent sign guard
// ============================================================================

#[test]
fn fa_sec05_07_xy_bland_altman_replicates_diagnostics_and_percent_sign_guard() {
    // 1. Y[A] vs Y[B] with RawReplicates(2) pairs all matching subcolumns across every row and tracks ND/incomplete.
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap();
    xy.x_title = String::from("Time");
    xy.data_sets[0].title = String::from("DeviceA");
    xy.data_sets[1].title = String::from("DeviceB");
    let x_vals = [1.0, 2.0, 3.0, 4.0];
    let a_subs = [
        [Cell::number(10.0), Cell::number(12.0)],
        [Cell::number(20.0), Cell::number(22.0)],
        [Cell::number(30.0), Cell::text("ND")],
        [Cell::number(40.0), Cell::number(42.0)],
    ];
    let b_subs = [
        [Cell::number(11.0), Cell::number(13.0)],
        [Cell::number(21.0), Cell::number(23.0)],
        [Cell::number(31.0), Cell::number(33.0)],
        [Cell::number(41.0), Cell::number(43.0)],
    ];
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number(x_vals[r])).unwrap();
        xy.data_sets[0].cells[r] = a_subs[r].to_vec();
        xy.data_sets[1].cells[r] = b_subs[r].to_vec();
    }
    let xy_sheet = DataSheet::new(SheetId::new("xy-rep"), "XY Rep", TableArchetype::Xy(xy)).unwrap();
    let spec_yvsy = AnalysisSpec::new(
        SheetId::new("ba-yvsy"),
        xy_sheet.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(false) }),
    )
    .unwrap();
    let res_yvsy = run_analysis(&xy_sheet, &spec_yvsy).unwrap();
    // 4 rows * 2 replicates - 1 ND cell in (row 2, rep 1) = 7 complete replicate pairs.
    assert_close(scalar_val(&res_yvsy, "N"), 7.0, 1e-12, "XY Y[A] vs Y[B] pairs all replicates");
    assert_close(scalar_val(&res_yvsy, "Bias"), -1.0, 1e-12, "XY Y[A] vs Y[B] bias across all replicates");
    assert!(res_yvsy.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(res_yvsy.checklist.iter().any(|c| c.assumption == "Complete-case row alignment"));

    // 2. X vs Y[0] mode tracks ignored text & skipped incomplete rows (including GeneratedSequence).
    let mut xy_x = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy_x.x_title = String::from("Ref");
    xy_x.data_sets[0].title = String::from("Test");
    let x_cells = [Cell::number(10.0), Cell::text("ND"), Cell::number(30.0), Cell::number(40.0), Cell::number(50.0)];
    let y_cells = [Cell::number(10.5), Cell::number(20.5), Cell::empty(), Cell::number(41.0), Cell::number(50.5)];
    for r in 0..5 {
        xy_x.set_x_cell(r, x_cells[r].clone()).unwrap();
        xy_x.data_sets[0].cells[r][0] = y_cells[r].clone();
    }
    let xy_x_sheet = DataSheet::new(SheetId::new("xy-x"), "XY X", TableArchetype::Xy(xy_x)).unwrap();
    let spec_xvsy = AnalysisSpec::new(
        SheetId::new("ba-xvsy"),
        xy_x_sheet.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(true) }),
    )
    .unwrap();
    let res_xvsy = run_analysis(&xy_x_sheet, &spec_xvsy).unwrap();
    assert_close(scalar_val(&res_xvsy, "N"), 3.0, 1e-12, "X vs Y[0] retained pairs");
    assert!(res_xvsy.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(
        res_xvsy
            .checklist
            .iter()
            .any(|c| c.assumption == "Complete-case row alignment" && c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 3 retained"))
    );

    let mut xy_gen = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 10.0, step: 10.0, progression: curvebook_doc::SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        4,
    )
    .unwrap();
    xy_gen.data_sets[0].cells = vec![vec![Cell::number(10.5)], vec![Cell::text("BAD")], vec![Cell::number(31.0)], vec![Cell::number(40.5)]];
    let xy_gen_sheet = DataSheet::new(SheetId::new("xy-gen"), "XY Gen", TableArchetype::Xy(xy_gen)).unwrap();
    let spec_gen = AnalysisSpec::new(
        SheetId::new("ba-gen"),
        xy_gen_sheet.id.clone(),
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, xy_compare_x_vs_y: Some(true) }),
    )
    .unwrap();
    let res_gen = run_analysis(&xy_gen_sheet, &spec_gen).unwrap();
    assert_close(scalar_val(&res_gen, "N"), 3.0, 1e-12, "GeneratedSequence retained pairs");
    assert!(res_gen.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(res_gen.checklist.iter().any(|c| c.assumption == "Complete-case row alignment"));

    // 3. Percent mode rejects sign-crossing pairs (-1.0, 1.5 -> avg = 0.25 > 0) and zero values (0.0, 1.5).
    let err_cross = bland_altman(&[-1.0, 10.0], &[1.5, 11.0], true, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_cross, StatsError::InvalidParameter { name: "percent_difference_measurement", value } if (value + 1.0).abs() < 1e-12));
    let err_zero_val = bland_altman(&[0.0, 10.0], &[1.5, 11.0], true, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err_zero_val, StatsError::InvalidParameter { name: "percent_difference_measurement", value } if value == 0.0));
    // Absolute mode still accepts sign-crossing pairs cleanly.
    assert!(bland_altman(&[-1.0, 10.0], &[1.5, 11.0], false, ConfidenceLevel::DEFAULT_95).is_ok());
}

// ============================================================================
// FA-SEC05-08: One-sided DetectableEffect with target_power <= alpha
// ============================================================================

#[test]
fn fa_sec05_08_one_sided_detectable_effect_below_alpha_converges() {
    let alpha = 0.05;
    for target_power in [0.02, 0.05] {
        for tail in [TailMode::OneSided(OneSidedDirection::Greater), TailMode::OneSided(OneSidedDirection::Less)] {
            for family in [
                PowerTestFamily::OneSampleT,
                PowerTestFamily::TwoSamplePairedT,
                PowerTestFamily::TwoSampleUnpairedT,
                PowerTestFamily::SimpleLinearRegression,
            ] {
                let req = PowerAnalysisRequest::new(family)
                    .with_target(PowerSolveTarget::DetectableEffect)
                    .with_tail(tail)
                    .with_alpha(alpha)
                    .with_target_power(target_power)
                    .with_sample_size(20.0);
                let res = solve_power(&req).unwrap_or_else(|e| panic!("failed {family:?} {tail:?} tp={target_power}: {e:?}"));
                assert_close(res.achieved_power, target_power, 1e-7, &format!("{family:?} {tail:?} tp={target_power}"));
                if (target_power - alpha).abs() < 1e-12 {
                    assert_close(res.effect_size, 0.0, 1e-12, &format!("{family:?} tp==alpha effect"));
                } else if family != PowerTestFamily::SimpleLinearRegression {
                    match tail {
                        TailMode::OneSided(OneSidedDirection::Greater) => assert!(res.effect_size < 0.0),
                        TailMode::OneSided(OneSidedDirection::Less) => assert!(res.effect_size > 0.0),
                        TailMode::TwoSided => {}
                    }
                }
            }

            // Welch t DetectableEffect with explicit SDs
            let welch_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_tail(tail)
                .with_alpha(alpha)
                .with_target_power(target_power)
                .with_sample_size(20.0)
                .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 0.0, sd: 2.0, sd_group2: Some(3.0), within_pair_correlation: None });
            let welch_res = solve_power(&welch_req).unwrap();
            assert_close(welch_res.achieved_power, target_power, 1e-7, "Welch MDE tp <= alpha");

            // Correlation across all 4 methods
            for cm in [
                CorrelationPowerMethod::FisherZ,
                CorrelationPowerMethod::FisherZWithTCrit,
                CorrelationPowerMethod::ExactTTransform,
                CorrelationPowerMethod::ConditionalSlopeT,
            ] {
                let corr_req = PowerAnalysisRequest::new(PowerTestFamily::Correlation)
                    .with_correlation_method(cm)
                    .with_target(PowerSolveTarget::DetectableEffect)
                    .with_tail(tail)
                    .with_alpha(alpha)
                    .with_target_power(target_power)
                    .with_sample_size(25.0);
                let corr_res = solve_power(&corr_req).unwrap_or_else(|e| panic!("failed Corr {cm:?} {tail:?} tp={target_power}: {e:?}"));
                assert_close(corr_res.achieved_power, target_power, 1e-6, &format!("Corr {cm:?} {tail:?} tp={target_power}"));
            }

            // TwoProportions (Cohen's h and BaselineProportion)
            let p_h_req = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
                .with_two_proportions_mode(TwoProportionsMethod::CohensH)
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_tail(tail)
                .with_alpha(alpha)
                .with_target_power(target_power)
                .with_sample_size(50.0);
            let p_h_res = solve_power(&p_h_req).unwrap();
            assert_close(p_h_res.achieved_power, target_power, 1e-7, "TwoProportions Cohen's h MDE tp <= alpha");

            let p_base_req = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
                .with_two_proportions_mode(TwoProportionsMethod::PooledNullUnpooledAlt)
                .with_effect(PowerEffectSpec::BaselineProportion { p1: 0.5 })
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_tail(tail)
                .with_alpha(alpha)
                .with_target_power(target_power)
                .with_sample_size(50.0);
            let p_base_res = solve_power(&p_base_req).unwrap();
            assert_close(p_base_res.achieved_power, target_power, 1e-7, "TwoProportions p1=0.5 MDE tp <= alpha");
        }
    }

    // Two-sided and non-directional families continue to reject target_power <= alpha.
    assert!(matches!(
        solve_power(
            &PowerAnalysisRequest::new(PowerTestFamily::OneSampleT)
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_alpha(0.05)
                .with_target_power(0.02)
                .with_sample_size(20.0)
        ),
        Err(StatsError::InvalidParameter { name: "target_power", .. })
    ));
    assert!(matches!(
        solve_power(
            &PowerAnalysisRequest::new(PowerTestFamily::Correlation)
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_alpha(0.05)
                .with_target_power(0.02)
                .with_sample_size(20.0)
        ),
        Err(StatsError::InvalidParameter { name: "target_power", .. })
    ));
}

// ============================================================================
// FA-SEC05-09: Mann-Whitney all-identical normal approx returns p = 1.0
// ============================================================================

#[test]
fn fa_sec05_09_mann_whitney_all_identical_one_tailed_returns_one() {
    for (na, nb) in [(9, 9), (10, 10), (10, 15), (50, 50)] {
        let a = vec![5.0_f64; na];
        let b = vec![5.0_f64; nb];
        let one = mann_whitney_u(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
        let two = mann_whitney_u(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_close(one.p_value, 1.0, 1e-12, &format!("identical MW ({na}, {nb}) one-tailed"));
        assert_close(two.p_value, 1.0, 1e-12, &format!("identical MW ({na}, {nb}) two-tailed"));
        assert_close(one.cliffs_delta, 0.0, 1e-12, "identical Cliff's delta");
    }
}

// ============================================================================
// FA-SEC05-10: compare_groups_* / Dunn / linear_trend domain validation
// ============================================================================

#[test]
fn fa_sec05_10_compare_groups_rejects_invalid_variance_df_and_group_domains() {
    let valid_groups = vec![
        GroupComparisonInput { index: 0, label: "A".into(), n: 5.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: "B".into(), n: 5.0, mean: 12.0, sd: 1.5 },
        GroupComparisonInput { index: 2, label: "C".into(), n: 5.0, mean: 15.0, sd: 1.5 },
    ];

    // Negative / NaN / infinite mse rejected for pooled procedures (including NewmanKeuls with zero/bad df).
    for bad_mse in [-1.0, -10.0, f64::NAN, f64::INFINITY] {
        for proc in [
            MultiplicityProcedure::Bonferroni,
            MultiplicityProcedure::TukeyKramer,
            MultiplicityProcedure::NewmanKeuls,
            MultiplicityProcedure::Dunnett { control_column: 0 },
        ] {
            let err = compare_groups_with_procedure(&valid_groups, bad_mse, 12.0, proc, 0.05, true).unwrap_err();
            assert!(matches!(err, StatsError::InvalidParameter { name: "mse", .. }), "proc={proc:?} mse={bad_mse} got {err:?}");
        }
    }

    // Invalid df_error rejected for pooled procedures even when mse == 0.0.
    for bad_df in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        for proc in [MultiplicityProcedure::Bonferroni, MultiplicityProcedure::NewmanKeuls, MultiplicityProcedure::TukeyKramer] {
            let err = compare_groups_with_procedure(&valid_groups, 0.0, bad_df, proc, 0.05, true).unwrap_err();
            assert!(matches!(err, StatsError::InvalidParameter { name: "df_error", .. }), "proc={proc:?} df={bad_df} got {err:?}");
        }
    }

    // Welch procedures still allow unused placeholder mse = 0.0, df_error = 0.0.
    for welch_proc in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        assert!(compare_groups_with_procedure(&valid_groups, 0.0, 0.0, welch_proc, 0.05, true).is_ok());
    }

    // Invalid group n, mean, sd rejected across both pooled and Welch procedures.
    let bad_n_groups = vec![GroupComparisonInput { index: 0, label: "A".into(), n: 0.0, mean: 10.0, sd: 1.5 }, valid_groups[1].clone()];
    let bad_mean_groups = vec![GroupComparisonInput { index: 0, label: "A".into(), n: 5.0, mean: f64::NAN, sd: 1.5 }, valid_groups[1].clone()];
    let bad_neg_sd = vec![GroupComparisonInput { index: 0, label: "A".into(), n: 5.0, mean: 10.0, sd: -1.0 }, valid_groups[1].clone()];
    let bad_nan_sd = vec![GroupComparisonInput { index: 0, label: "A".into(), n: 5.0, mean: 10.0, sd: f64::NAN }, valid_groups[1].clone()];

    for proc in [MultiplicityProcedure::Bonferroni, MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2] {
        assert!(matches!(
            compare_groups_with_procedure(&bad_n_groups, 2.0, 8.0, proc, 0.05, true),
            Err(StatsError::InvalidParameter { name: "group_n", .. })
        ));
        assert!(matches!(
            compare_groups_with_procedure(&bad_mean_groups, 2.0, 8.0, proc, 0.05, true),
            Err(StatsError::InvalidParameter { name: "group_mean", .. })
        ));
        assert!(matches!(
            compare_groups_with_procedure(&bad_neg_sd, 2.0, 8.0, proc, 0.05, true),
            Err(StatsError::InvalidParameter { name: "group_sd", .. })
        ));
        assert!(matches!(
            compare_groups_with_procedure(&bad_nan_sd, 2.0, 8.0, proc, 0.05, true),
            Err(StatsError::InvalidParameter { name: "group_sd", .. })
        ));
    }

    // Dunn and linear trend domain guards.
    assert!(matches!(
        compare_dunn_ranks(&[(0, "A".into()), (1, "B".into())], &[0, 5], &[3.0, 8.0], 5, 0.0, &MultiplicitySpec::default(),),
        Err(StatsError::InvalidParameter { name: "group_n", .. })
    ));
    assert!(matches!(linear_trend_test(&bad_mean_groups, 2.0, 8.0), Err(StatsError::InvalidParameter { name: "group_mean", .. })));
}

// ============================================================================
// FA-SEC05-11: ROC & MultipleRow input validation, ignored-text warnings, sanitization
// ============================================================================

#[test]
fn fa_sec05_11_roc_and_multiple_row_reject_ambiguous_selections_and_warn_on_text() {
    // 1. ROC rejects 3-column and 1-column explicit selections on Column & MultipleVariables.
    let col3 = col_sheet_cells(&[
        ("Ctrl", &[Cell::number(1.0), Cell::number(2.0), Cell::text("ND"), Cell::number(3.0)]),
        ("Case", &[Cell::number(5.0), Cell::number(6.0), Cell::number(7.0), Cell::number(8.0)]),
        ("Extra", &[Cell::number(9.0), Cell::number(10.0), Cell::number(11.0), Cell::number(12.0)]),
    ]);
    for bad_sel in [vec![0], vec![0, 1, 2]] {
        let mut sp = AnalysisSpec::new(SheetId::new("roc-bad"), col3.id.clone(), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
        sp.selected_data_sets = bad_sel.clone();
        assert!(matches!(
            run_analysis(&col3, &sp),
            Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - bad_sel.len() as f64).abs() < 1e-12
        ));
    }

    // Valid 2-column ROC with ND text emits warning checklist & footnote and sanitizes inf LR+ to Empty.
    let mut sp_ok = AnalysisSpec::new(SheetId::new("roc-ok"), col3.id.clone(), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
    sp_ok.selected_data_sets = vec![0, 1];
    let res_roc = run_analysis(&col3, &sp_ok).unwrap();
    assert!(res_roc.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(tab_by_id(&res_roc, "roc-summary").footnotes.iter().any(|f| f.contains("Non-numeric values ignored: 1")));
    let coords = tab_by_id(&res_roc, "roc-coordinates");
    assert!(
        coords.rows.iter().all(|r| r.iter().all(|c| !matches!(c, ResultCell::Number { value, .. } if !value.is_finite()))),
        "all numeric cells in roc-coordinates must be finite"
    );

    // MultipleVariables with Label column: [0=ID, 1=Ctrl, 2=Case] ignores Label; [0, 1] rejects.
    let mut mv = MultipleVariablesTable::new(0, 4).unwrap();
    let mut id = VariableColumn::new("ID", 4).unwrap();
    id.cells = vec![Cell::number(101.0), Cell::number(102.0), Cell::number(103.0), Cell::number(104.0)];
    id.lock_role(VariableRole::Label);
    let mut c1 = VariableColumn::new("Ctrl", 4).unwrap();
    c1.cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)];
    let mut c2 = VariableColumn::new("Case", 4).unwrap();
    c2.cells = vec![Cell::number(5.0), Cell::number(6.0), Cell::number(7.0), Cell::number(8.0)];
    mv.variables = vec![id, c1, c2];
    let mv_sheet = DataSheet::new(SheetId::new("mv-roc"), "MV ROC", TableArchetype::MultipleVariables(mv)).unwrap();

    let mut sp_mv_3 = AnalysisSpec::new(SheetId::new("roc-mv3"), mv_sheet.id.clone(), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
    sp_mv_3.selected_data_sets = vec![0, 1, 2];
    let res_mv = run_analysis(&mv_sheet, &sp_mv_3).unwrap();
    assert_close(scalar_val(&res_mv, "Area under ROC curve (AUC)"), 1.0, 1e-12, "MV ROC skips Label");

    let mut sp_mv_2 = sp_mv_3;
    sp_mv_2.selected_data_sets = vec![0, 1];
    assert!(matches!(run_analysis(&mv_sheet, &sp_mv_2), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    // 2. MultipleRowTTests emits ignored-text and listwise-incomplete warnings across Grouped and Column modes.
    let grp_nd = grouped_sheet_cells(
        1,
        4,
        "A",
        &[vec![Cell::number(1.0), Cell::text("ND"), Cell::number(3.5), Cell::number(5.0)]],
        "B",
        &[vec![Cell::number(5.0), Cell::number(6.0), Cell::number(7.0), Cell::number(8.0)]],
    );
    let mr_paired = AnalysisSpec::new(
        SheetId::new("mr-nd"),
        grp_nd.id.clone(),
        AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec { paired: true, ..MultipleRowTestsSpec::default() }),
    )
    .unwrap();
    let res_mr = run_analysis(&grp_nd, &mr_paired).unwrap();
    assert!(res_mr.checklist.iter().any(|c| c.assumption == "Numeric input validation" && c.status == ChecklistStatus::Warn));
    assert!(res_mr.checklist.iter().any(|c| c.assumption == "Complete-case row alignment"));
    let sum_tab = tab_by_id(&res_mr, "multiple-row-summary");
    assert!(sum_tab.footnotes.iter().any(|f| f.contains("Non-numeric values ignored: 1")));
    assert!(sum_tab.footnotes.iter().any(|f| f.contains("omitted 1 incomplete")));
}

// ============================================================================
// FA-SEC05-12: Alpha-aware p_cell stars & one_sample_ks_normal_at_alpha
// ============================================================================

#[test]
fn fa_sec05_12_alpha_aware_stars_and_ks_threshold() {
    // 1. PValueStack at threshold = 0.01 with p = 0.03: Discovery? = false AND stars = "ns".
    let mut col_p = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    col_p.data_sets[0].title = String::from("P");
    col_p.data_sets[0].cells = vec![vec![Cell::number(0.03)], vec![Cell::number(0.002)], vec![Cell::number(0.50)]];
    let p_sheet = DataSheet::new(SheetId::new("ps"), "PStack", TableArchetype::Column(col_p)).unwrap();
    let ps_spec = AnalysisSpec::new(
        SheetId::new("r-ps"),
        p_sheet.id.clone(),
        AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::Bonferroni, threshold: 0.01, column: None }),
    )
    .unwrap();
    let res_ps = run_analysis(&p_sheet, &ps_spec).unwrap();
    let ps_adj = tab_by_id(&res_ps, "pvalue-stack-adjusted");
    assert_eq!(ps_adj.rows[0][5], ResultCell::Boolean(false));
    assert_eq!(cell_p_stars(&ps_adj.rows[0][3]).1, "ns"); // raw_p = 0.03 > 0.01
    assert_eq!(cell_p_stars(&ps_adj.rows[0][4]).1, "ns"); // adj_p = 0.09 > 0.01
    assert_eq!(ps_adj.rows[1][5], ResultCell::Boolean(true));
    assert_eq!(cell_p_stars(&ps_adj.rows[1][4]).1, "*"); // adj_p = 0.006 <= 0.01 (t2 = 0.01/5 = 0.002 < 0.006)

    // 2. Contingency at 99% confidence (alpha = 0.01) with p in (0.01, 0.05) emits "ns".
    // [[12, 3], [5, 10]]: Pearson chi2 = 6.6522..., df = 1, p ~ 0.0099 < 0.01 vs [[11, 4], [5, 10]]: chi2 = 4.8214..., p ~ 0.0281 in (0.01, 0.05).
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].cells[0][0] = Cell::number(11.0);
    ct.data_sets[1].cells[0][0] = Cell::number(4.0);
    ct.data_sets[0].cells[1][0] = Cell::number(5.0);
    ct.data_sets[1].cells[1][0] = Cell::number(10.0);
    let ct_sheet = DataSheet::new(SheetId::new("ct-stars"), "CT", TableArchetype::Contingency(ct)).unwrap();
    let mut ct_spec99 = AnalysisSpec::new(
        SheetId::new("r-ct99"),
        ct_sheet.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ..Default::default() }),
    )
    .unwrap();
    ct_spec99.set_confidence_percent(99.0).unwrap();
    let res_ct99 = run_analysis(&ct_sheet, &ct_spec99).unwrap();
    let (p_ct, stars_ct) = cell_p_stars(&tab_by_id(&res_ct99, "contingency-main").rows[0][2]);
    assert!(p_ct > 0.01 && p_ct < 0.05);
    assert_eq!(stars_ct, "ns");

    // PartsOfWhole GOF at 99% confidence: [13, 5] vs uniform (chi2 = 3.555 -> p ~ 0.059) or [14, 4] (chi2 = 5.555 -> p ~ 0.0184 in (0.01, 0.05)).
    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.data_sets[0].cells[0][0] = Cell::number(14.0);
    pow.data_sets[0].cells[1][0] = Cell::number(4.0);
    let pow_sheet = DataSheet::new(SheetId::new("pow-99"), "POW", TableArchetype::PartsOfWhole(pow)).unwrap();
    let mut pow_spec99 = AnalysisSpec::new(
        SheetId::new("r-pow99"),
        pow_sheet.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ..Default::default() }),
    )
    .unwrap();
    pow_spec99.set_confidence_percent(99.0).unwrap();
    let res_pow99 = run_analysis(&pow_sheet, &pow_spec99).unwrap();
    let (p_pow, stars_pow) = cell_p_stars(&tab_by_id(&res_pow99, "parts-gof").rows[0][2]);
    assert!(p_pow > 0.01 && p_pow < 0.05);
    assert_eq!(stars_pow, "ns");

    // 3. build_multiplicity_tab at 99% confidence and mismatched threshold vs confidence never contradicts Significant.
    let groups = vec![
        GroupComparisonInput { index: 0, label: "A".into(), n: 6.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: "B".into(), n: 6.0, mean: 12.1, sd: 1.5 },
    ];
    let details = compare_groups_with_procedure(&groups, 2.25, 10.0, MultiplicityProcedure::None, 0.01, false).unwrap();
    assert!((0.01..0.05).contains(&details[0].raw_p));
    assert!(!details[0].significant);
    let mult_tab_99 = build_multiplicity_tab(&details, "MC", PValueFormat::FourDecimal, ConfidenceLevel::PERCENT_99);
    assert_eq!(cell_p_stars(&mult_tab_99.rows[0][4]).1, "ns");
    assert_eq!(cell_p_stars(&mult_tab_99.rows[0][5]).1, "ns");
    // Even if caller supplies 95% confidence alongside threshold=0.01 details, Significant=false prevents "*" contradiction.
    let mult_tab_95 = build_multiplicity_tab(&details, "MC", PValueFormat::FourDecimal, ConfidenceLevel::DEFAULT_95);
    assert_eq!(cell_p_stars(&mult_tab_95.rows[0][5]).1, "ns");

    // 4. ROC at 99% confidence with p in (0.01, 0.05) emits "ns".
    let roc_ds = col_sheet_nums(&[("Ctrl", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("Case", &[3.0, 4.0, 5.0, 6.0, 7.5])]);
    let mut roc_spec99 =
        AnalysisSpec::new(SheetId::new("r-roc99"), roc_ds.id.clone(), AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong })).unwrap();
    roc_spec99.set_confidence_percent(99.0).unwrap();
    let res_roc99 = run_analysis(&roc_ds, &roc_spec99).unwrap();
    let (p_roc, stars_roc) = cell_p_stars(&tab_by_id(&res_roc99, "roc-summary").rows[0][3]);
    assert!(p_roc > 0.01 && p_roc < 0.05, "expected ROC p in (0.01, 0.05), got {p_roc}");
    assert_eq!(stars_roc, "ns");

    // 5. one_sample_ks_normal_at_alpha honors custom alpha while one_sample_ks_normal preserves 0.05.
    let shifted = [0.2, 0.4, 0.6, 0.8, 1.0, 1.2, 1.4, 1.6, 1.8, 2.0];
    let ks_05 = one_sample_ks_normal(&shifted, 0.0, 1.0).unwrap();
    let ks_001 = one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.001).unwrap();
    let ks_50 = one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.50).unwrap();
    assert_close(ks_05.p_value, ks_001.p_value, 1e-15, "KS p_value independent of alpha");
    assert!(ks_001.passed);
    assert!(!ks_50.passed);
    assert!(matches!(one_sample_ks_normal_at_alpha(&shifted, 0.0, 1.0, 0.0), Err(StatsError::InvalidParameter { name: "alpha", .. })));
    assert!(matches!(one_sample_ks_normal_at_alpha(&shifted, f64::NAN, 1.0, 0.05), Err(StatsError::InvalidParameter { name: "mean", .. })));
}

// ============================================================================
// FA-SEC05-13: Verified semantics matching updated docstrings
// ============================================================================

#[test]
fn fa_sec05_13_doc_contract_semantics() {
    // Signed Z in BarnardExactResult::statistic (negative when a/n1 < c/n2, positive when a/n1 > c/n2).
    let b_pos = barnard_exact_2x2_with_ordering(10, 2, 3, 9, BarnardOrdering::Boschloo).unwrap();
    let b_neg = barnard_exact_2x2_with_ordering(2, 10, 9, 3, BarnardOrdering::Boschloo).unwrap();
    assert!(b_pos.statistic > 0.0);
    assert!(b_neg.statistic < 0.0);
    assert_close(b_pos.statistic, -b_neg.statistic, 1e-12, "signed z_pooled symmetry");

    // NormalityTestOutcome::passed uses strict > alpha with caller-supplied alpha.
    let vals = [1.0, 1.1, 1.2, 1.3, 1.4, 1.5, 5.0];
    let sw_hi = shapiro_wilk(&vals, 0.99).unwrap();
    assert!(!sw_hi.passed);
    let sw_lo = shapiro_wilk(&vals, 1e-12).unwrap();
    assert!(sw_lo.passed);

    // adjust_p_values legacy ANOVA fallback vs try_adjust_p_values rejection.
    let raw = [0.01, 0.04, 0.20];
    let (bonf, _) = adjust_p_values(&raw, MultiplicityProcedure::TukeyKramer, 0.05);
    assert_close(bonf[0], 0.03, 1e-12, "adjust_p_values TukeyKramer -> Bonferroni");
    assert!(matches!(try_adjust_p_values(&raw, MultiplicityProcedure::TukeyKramer, 0.05), Err(StatsError::IncompatibleAnalysisSpec { .. })));
}
