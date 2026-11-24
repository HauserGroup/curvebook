//! D1 reference-oracle and regression tests for Wave 1 SEC-05 S1 findings:
//! - `FA-SEC05-01`: `PartsOfWhole` method compatibility & `LikelihoodRatioG2` primary tab/scalar dispatch
//! - `FA-SEC05-02`: Explicit `selected_data_sets` validation (no silent OOB drop, `[0, 0]` dup, or multi-column truncation)
//! - `FA-SEC05-03`: Two-group & MV column selection truncation, `>= 2` categorical factor guard, and `Label` exclusion
//! - `FA-SEC05-04`: Relative / log-space tolerances for extreme-tail Boschloo and CSM unconditional exact tests
//!
//! Reference provenance (clean-room Decision D1):
//! - Boschloo, R. D. (1970). *Stat. Neerlandica*, 24(1), 1–9; Barnard, G. A. (1947). *Biometrika*, 34(1–2), 123–138 (CSM);
//!   exact combinatorial sums verified in Python 3.13 `math.comb` / `fractions.Fraction` / `decimal.Decimal` (2026-11-22).
//! - Analytical chi-square survival `exp(-x / 2)` for `df = 2` (goodness-of-fit on 3 categories).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable, DataSheet, KappaWeighting,
    MultipleRowTestsSpec, MultipleVariablesTable, PartsOfWholeTable, ResultCell, ResultSheet, ResultTab, SheetId, SubcolumnMode, TableArchetype,
    TableKind, TwoGroupMethod, TwoGroupSpec, VariableColumn, VariableRole,
};
use curvebook_stats::categorical::{BarnardOrdering, barnard_exact_2x2_with_ordering, exact_binomial_test, fisher_exact_2x2};
use curvebook_stats::column::one_sample_t_test;
use curvebook_stats::two_group::unpaired_t_test;
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::{assert_close, assert_rel_close};

fn scalar_val(sheet: &ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index.is_none()).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected ResultCell::Number, got {other:?}"),
    }
}

fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected ResultCell::PValue, got {other:?}"),
    }
}

fn parts_sheet(slices: &[(&str, f64)]) -> DataSheet {
    let mut pow = PartsOfWholeTable::new(1, slices.len()).unwrap();
    pow.slice_labels = slices.iter().map(|(l, _)| (*l).to_owned()).collect();
    for (r, &(_, v)) in slices.iter().enumerate() {
        pow.data_sets[0].cells[r][0] = Cell::number(v);
    }
    DataSheet::new(SheetId::new("pow"), "Parts", TableArchetype::PartsOfWhole(pow)).unwrap()
}

fn contingency_sheet(rows: &[&[u64]]) -> DataSheet {
    let n_rows = rows.len();
    let n_cols = rows[0].len();
    let mut tab = ContingencyTable::new(n_cols, n_rows).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(v as f64);
        }
    }
    DataSheet::new(SheetId::new("ct"), "Contingency", TableArchetype::Contingency(tab)).unwrap()
}

fn col_sheet(cols: &[(&str, &[f64])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tbl.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("col"), "Columns", TableArchetype::Column(tbl)).unwrap()
}

// ============================================================================
// FA-SEC05-01: PartsOfWhole compatibility & LikelihoodRatioG2 primary dispatch
// ============================================================================

#[test]
fn fa_sec05_01_parts_of_whole_rejects_incompatible_contingency_methods_and_honors_g2() {
    let pow3 = parts_sheet(&[("A", 50.0), ("B", 30.0), ("C", 20.0)]);

    for bad_method in [
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear },
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic },
        ContingencyMethod::FleissKappa,
        ContingencyMethod::FisherExact,
        ContingencyMethod::BarnardExact,
        ContingencyMethod::CochranArmitageTrend,
        ContingencyMethod::McNemarPaired,
        ContingencyMethod::PearsonChiSquare { yates_continuity: true },
    ] {
        let spec = AnalysisSpec::new(
            SheetId::new("r-bad"),
            pow3.id.clone(),
            AnalysisKind::Contingency(ContingencySpec { method: bad_method, ..Default::default() }),
        )
        .unwrap();
        let err = run_analysis(&pow3, &spec).unwrap_err();
        assert!(
            matches!(err, StatsError::IncompatibleTable { actual: TableKind::PartsOfWhole, .. }),
            "expected IncompatibleTable(PartsOfWhole) for {bad_method:?}, got {err:?}"
        );
    }

    // Allowed: PearsonChiSquare { yates_continuity: false } vs LikelihoodRatioG2.
    // D1 exact analytic oracle for df = 2: chi2 = 14.0 -> P = exp(-7); G2 = 13.791854920707223 -> P = exp(-G2/2).
    let expected_chi2 = 14.0_f64;
    let expected_p_chi2 = (-expected_chi2 / 2.0).exp();
    let expected_g2: f64 = 2.0
        * (50.0_f64 * (50.0_f64 / (100.0_f64 / 3.0)).ln()
            + 30.0_f64 * (30.0_f64 / (100.0_f64 / 3.0)).ln()
            + 20.0_f64 * (20.0_f64 / (100.0_f64 / 3.0)).ln());
    let expected_p_g2: f64 = (-expected_g2 / 2.0).exp();

    let pearson_spec = AnalysisSpec::new(
        SheetId::new("r-p"),
        pow3.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ..Default::default() }),
    )
    .unwrap();
    let res_p = run_analysis(&pow3, &pearson_spec).unwrap();
    let tab_p = tab_by_id(&res_p, "parts-gof");
    assert!(tab_p.title.starts_with("Chi-square goodness of fit"), "unexpected title: {}", tab_p.title);
    assert_eq!(tab_p.row_labels, vec![String::from("Pearson chi-square"), String::from("Likelihood-ratio G2")]);
    assert_close(scalar_val(&res_p, "P value"), expected_p_chi2, 1e-12, "Pearson primary P");
    assert_close(cell_num(&tab_p.rows[0][0]), expected_chi2, 1e-12, "Pearson row 0 stat");
    assert_close(cell_p(&tab_p.rows[0][2]), expected_p_chi2, 1e-12, "Pearson row 0 P");

    let g2_spec = AnalysisSpec::new(
        SheetId::new("r-g2"),
        pow3.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::LikelihoodRatioG2, ..Default::default() }),
    )
    .unwrap();
    let res_g2 = run_analysis(&pow3, &g2_spec).unwrap();
    let tab_g2 = tab_by_id(&res_g2, "parts-gof");
    assert!(tab_g2.title.starts_with("Likelihood-ratio G2 goodness of fit"), "unexpected title: {}", tab_g2.title);
    assert_eq!(tab_g2.row_labels, vec![String::from("Likelihood-ratio G2"), String::from("Pearson chi-square")]);
    assert_close(scalar_val(&res_g2, "P value"), expected_p_g2, 1e-12, "G2 primary P");
    assert_close(scalar_val(&res_g2, "G-squared P value"), expected_p_g2, 1e-12, "G2 scalar P");
    assert_close(scalar_val(&res_g2, "Chi-square P value"), expected_p_chi2, 1e-12, "Chi2 scalar P");
    assert_close(cell_num(&tab_g2.rows[0][0]), expected_g2, 1e-12, "G2 row 0 stat");
    assert_close(cell_p(&tab_g2.rows[0][2]), expected_p_g2, 1e-12, "G2 row 0 P");

    // ExactBinomial remains valid on 2-slice PartsOfWhole.
    let pow2 = parts_sheet(&[("Success", 14.0), ("Failure", 6.0)]);
    let bin_spec = AnalysisSpec::new(
        SheetId::new("r-bin"),
        pow2.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() }),
    )
    .unwrap();
    let res_bin = run_analysis(&pow2, &bin_spec).unwrap();
    let direct_bin = exact_binomial_test(14, 20, 0.5, bin_spec.confidence_level).unwrap();
    assert_close(scalar_val(&res_bin, "p_value"), direct_bin.p_value_sterne, 1e-12, "PoW ExactBinomial Sterne P");
}

// ============================================================================
// FA-SEC05-02: Contingency & ExactBinomial selected_data_sets validation
// ============================================================================

#[test]
fn fa_sec05_02_contingency_and_exact_binomial_reject_duplicate_oob_and_multi_column_selections() {
    let ct_2x2 = contingency_sheet(&[&[10, 2], &[3, 9]]);
    for method in [
        ContingencyMethod::PearsonChiSquare { yates_continuity: false },
        ContingencyMethod::FisherExact,
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
    ] {
        let mut dup =
            AnalysisSpec::new(SheetId::new("ct-dup"), ct_2x2.id.clone(), AnalysisKind::Contingency(ContingencySpec { method, ..Default::default() }))
                .unwrap();
        dup.selected_data_sets = vec![0, 0];
        assert!(
            matches!(run_analysis(&ct_2x2, &dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 0.0).abs() < 1e-12),
            "expected duplicate index rejection for {method:?}"
        );

        let mut oob = dup;
        oob.selected_data_sets = vec![0, 1, 99];
        assert!(matches!(run_analysis(&ct_2x2, &oob), Err(StatsError::MissingColumn { index: 99 })), "expected OOB index rejection for {method:?}");
    }

    // ExactBinomial on Contingency 1x2 rejects [0, 0] and [0, 99].
    let ct_1x2 = contingency_sheet(&[&[14, 6]]);
    let mut bin_dup = AnalysisSpec::new(
        SheetId::new("bin-dup"),
        ct_1x2.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() }),
    )
    .unwrap();
    bin_dup.selected_data_sets = vec![0, 0];
    assert!(matches!(run_analysis(&ct_1x2, &bin_dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut bin_oob = bin_dup;
    bin_oob.selected_data_sets = vec![0, 99];
    assert!(matches!(run_analysis(&ct_1x2, &bin_oob), Err(StatsError::MissingColumn { index: 99 })));

    // ExactBinomial on Column rejects multi-column [0, 1], duplicate [0, 0], and OOB [99], while honoring [1].
    let col_bin = col_sheet(&[("Col0", &[1.0, 0.0, 1.0, 0.0]), ("Col1", &[1.0, 1.0, 1.0, 1.0, 1.0, 0.0])]);
    let mut col_multi = AnalysisSpec::new(
        SheetId::new("col-bin-multi"),
        col_bin.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::ExactBinomial, null_proportion: Some(0.5), ..Default::default() }),
    )
    .unwrap();
    col_multi.selected_data_sets = vec![0, 1];
    assert!(matches!(
        run_analysis(&col_bin, &col_multi),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 2.0).abs() < 1e-12
    ));

    let mut col_dup = col_multi.clone();
    col_dup.selected_data_sets = vec![0, 0];
    assert!(matches!(run_analysis(&col_bin, &col_dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut col_oob = col_multi.clone();
    col_oob.selected_data_sets = vec![99];
    assert!(matches!(run_analysis(&col_bin, &col_oob), Err(StatsError::MissingColumn { index: 99 })));

    let mut col_one = col_multi;
    col_one.selected_data_sets = vec![1];
    let res_col1 = run_analysis(&col_bin, &col_one).unwrap();
    assert_close(scalar_val(&res_col1, "successes"), 5.0, 1e-12, "ExactBinomial Column[1] successes");
    assert_close(scalar_val(&res_col1, "total"), 6.0, 1e-12, "ExactBinomial Column[1] total");
}

// ============================================================================
// FA-SEC05-03: Two-group & MV selection truncation, >=2 categorical factors, Label leakage
// ============================================================================

#[test]
fn fa_sec05_03_column_selection_rejects_multi_and_partial_explicit_sets() {
    // 1. Column table: multi-column selection rejected on both two-group and one-sample tests (raw & pre-summarized).
    let c3 = col_sheet(&[("A", &[1.0, 2.0, 3.0, 4.0]), ("B", &[5.0, 6.0, 7.0, 8.0]), ("C", &[20.0, 22.0, 24.0, 26.0])]);
    for method in [
        TwoGroupMethod::UnpairedT,
        TwoGroupMethod::WelchT,
        TwoGroupMethod::LognormalUnpairedT,
        TwoGroupMethod::PairedT,
        TwoGroupMethod::RatioPairedT,
        TwoGroupMethod::MannWhitneyU,
        TwoGroupMethod::WilcoxonSignedRank,
        TwoGroupMethod::PairedSignTest,
        TwoGroupMethod::KolmogorovSmirnov,
    ] {
        let mut s3 =
            AnalysisSpec::new(SheetId::new("tg-3"), c3.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }))
                .unwrap();
        s3.selected_data_sets = vec![0, 1, 2];
        assert!(
            matches!(run_analysis(&c3, &s3), Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 3.0).abs() < 1e-12),
            "expected 3-col rejection for {method:?}"
        );

        let mut s1 = s3;
        s1.selected_data_sets = vec![0];
        assert!(
            matches!(run_analysis(&c3, &s1), Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 1.0).abs() < 1e-12),
            "expected 1-col rejection for {method:?}"
        );
    }

    for method in [TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, TwoGroupMethod::OneSampleWilcoxon { hypothetical: 0.0 }] {
        let mut s2 =
            AnalysisSpec::new(SheetId::new("os-2"), c3.id.clone(), AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }))
                .unwrap();
        s2.selected_data_sets = vec![0, 1];
        assert!(
            matches!(run_analysis(&c3, &s2), Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 2.0).abs() < 1e-12),
            "expected 2-col rejection for {method:?}"
        );

        // Selecting explicit single column [1] tests B cleanly.
        let mut s1 = s2;
        s1.selected_data_sets = vec![1];
        let res = run_analysis(&c3, &s1).unwrap();
        let expected = one_sample_t_test(&[5.0, 6.0, 7.0, 8.0], 0.0, true, s1.confidence_level).unwrap();
        if matches!(method, TwoGroupMethod::OneSampleT { .. }) {
            assert_close(scalar_val(&res, "t"), expected.t_statistic, 1e-12, "OneSampleT col[1] t");
        }
    }

    // Pre-summarized Column table also rejects multi-column explicit selections.
    let mut pre = ColumnTable::new(ColumnDesignHint::PrecomputedError, 3, 1).unwrap();
    pre.subcolumn_mode = SubcolumnMode::MeanSdN;
    for (c, (m, s, n)) in [(10.0, 1.0, 5.0), (15.0, 1.5, 5.0), (20.0, 2.0, 5.0)].into_iter().enumerate() {
        pre.data_sets[c].cells[0] = vec![Cell::number(m), Cell::number(s), Cell::number(n)];
    }
    let pre_sheet = DataSheet::new(SheetId::new("pre3"), "Pre3", TableArchetype::Column(pre)).unwrap();
    let mut pre_2g = AnalysisSpec::new(
        SheetId::new("pre-3"),
        pre_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
    )
    .unwrap();
    pre_2g.selected_data_sets = vec![0, 1, 2];
    assert!(matches!(run_analysis(&pre_sheet, &pre_2g), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut pre_1s = AnalysisSpec::new(
        SheetId::new("pre-1s"),
        pre_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() }),
    )
    .unwrap();
    pre_1s.selected_data_sets = vec![0, 1];
    assert!(matches!(run_analysis(&pre_sheet, &pre_1s), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
}

#[test]
fn fa_sec05_03_mv_rejects_ambiguous_multi_categorical_factors() {
    // MultipleVariables with >= 2 Categorical columns: never falls through to categorical text columns.
    let mut mv_2cat = MultipleVariablesTable::new(0, 4).unwrap();
    let mut sex = VariableColumn::new("Sex", 4).unwrap();
    sex.cells = vec![Cell::text("F"), Cell::text("F"), Cell::text("M"), Cell::text("M")];
    sex.lock_role(VariableRole::Categorical);
    let mut arm = VariableColumn::new("Arm", 4).unwrap();
    arm.cells = vec![Cell::text("Ctrl"), Cell::text("Trt"), Cell::text("Ctrl"), Cell::text("Trt")];
    arm.lock_role(VariableRole::Categorical);
    let mut before = VariableColumn::new("Before", 4).unwrap();
    before.cells = vec![Cell::number(10.0), Cell::number(11.0), Cell::number(12.0), Cell::number(13.0)];
    let mut after = VariableColumn::new("After", 4).unwrap();
    after.cells = vec![Cell::number(20.0), Cell::number(22.0), Cell::number(25.0), Cell::number(27.0)];
    mv_2cat.variables = vec![sex, arm, before, after];
    let mv_2cat_sheet = DataSheet::new(SheetId::new("mv-2cat"), "MV 2 Cat", TableArchetype::MultipleVariables(mv_2cat)).unwrap();

    for sel in [vec![], vec![0, 1, 2, 3]] {
        let mut sp = AnalysisSpec::new(
            SheetId::new("mv2c"),
            mv_2cat_sheet.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
        )
        .unwrap();
        sp.selected_data_sets = sel;
        let err = run_analysis(&mv_2cat_sheet, &sp).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "mv_group_column", value } if (value - 2.0).abs() < 1e-12),
            "expected mv_group_column error, got {err:?}"
        );
    }

    // Narrowing to numeric columns [2, 3] performs wide comparison Before vs After cleanly.
    let mut wide_23 = AnalysisSpec::new(
        SheetId::new("mv2c-wide"),
        mv_2cat_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
    )
    .unwrap();
    wide_23.selected_data_sets = vec![2, 3];
    let res_wide = run_analysis(&mv_2cat_sheet, &wide_23).unwrap();
    let expected_wide = unpaired_t_test(&[10.0, 11.0, 12.0, 13.0], &[20.0, 22.0, 25.0, 27.0], true, wide_23.confidence_level).unwrap();
    assert_close(scalar_val(&res_wide, "t"), expected_wide.t_statistic, 1e-12, "MV wide [2, 3] t");

    // Explicit mv_group_column + mv_response_column still succeeds when >= 2 categorical columns exist.
    let explicit_mv = AnalysisSpec::new(
        SheetId::new("mv2c-exp"),
        mv_2cat_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::UnpairedT,
            mv_group_column: Some(0),
            mv_response_column: Some(2),
            ..Default::default()
        }),
    )
    .unwrap();
    let res_exp = run_analysis(&mv_2cat_sheet, &explicit_mv).unwrap();
    let expected_sex = unpaired_t_test(&[10.0, 11.0], &[12.0, 13.0], true, explicit_mv.confidence_level).unwrap();
    assert_close(scalar_val(&res_exp, "t"), expected_sex.t_statistic, 1e-12, "MV explicit group=Sex, resp=Before t");

    // 1 Categorical + 1 Label (no continuous response): rejected cleanly rather than falling back to Label/Categorical.
    let mut mv_no_resp = MultipleVariablesTable::new(0, 4).unwrap();
    let mut grp = VariableColumn::new("Arm", 4).unwrap();
    grp.cells = vec![Cell::text("Ctrl"), Cell::text("Ctrl"), Cell::text("Trt"), Cell::text("Trt")];
    grp.lock_role(VariableRole::Categorical);
    let mut lbl = VariableColumn::new("SubjectID", 4).unwrap();
    lbl.cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)];
    lbl.lock_role(VariableRole::Label);
    mv_no_resp.variables = vec![grp, lbl];
    let mv_no_resp_sheet = DataSheet::new(SheetId::new("mv-no-resp"), "MV No Resp", TableArchetype::MultipleVariables(mv_no_resp)).unwrap();
    let no_resp_spec = AnalysisSpec::new(
        SheetId::new("mv-nr"),
        mv_no_resp_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&mv_no_resp_sheet, &no_resp_spec), Err(StatsError::InvalidParameter { name: "mv_response_column", .. })));
}

#[test]
fn fa_sec05_03_mv_excludes_label_columns_from_numeric_selections() {
    // MultipleVariables with numeric VariableRole::Label: never leaks SubjectID into OneSample, TwoGroup, or MultipleRowTTests.
    let mut mv_lbl = MultipleVariablesTable::new(0, 4).unwrap();
    let mut id = VariableColumn::new("SubjectID", 4).unwrap();
    id.cells = vec![Cell::number(1000.0), Cell::number(1001.0), Cell::number(1002.0), Cell::number(1003.0)];
    id.lock_role(VariableRole::Label);
    let mut a1 = VariableColumn::new("A1", 4).unwrap();
    a1.cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)];
    let mut a2 = VariableColumn::new("A2", 4).unwrap();
    a2.cells = vec![Cell::number(1.5), Cell::number(2.5), Cell::number(3.5), Cell::number(4.5)];
    let mut b1 = VariableColumn::new("B1", 4).unwrap();
    b1.cells = vec![Cell::number(10.0), Cell::number(11.0), Cell::number(12.0), Cell::number(13.0)];
    let mut b2 = VariableColumn::new("B2", 4).unwrap();
    b2.cells = vec![Cell::number(10.5), Cell::number(11.5), Cell::number(12.5), Cell::number(13.5)];
    mv_lbl.variables = vec![id, a1, a2, b1, b2];
    let mv_lbl_sheet = DataSheet::new(SheetId::new("mv-lbl"), "MV Label", TableArchetype::MultipleVariables(mv_lbl)).unwrap();

    // OneSampleT [0, 1, 2] filters Label(0) leaving [1, 2] -> rejected as multi-column (!= 1).
    let mut os_lbl_multi = AnalysisSpec::new(
        SheetId::new("os-lbl-m"),
        mv_lbl_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 }, ..Default::default() }),
    )
    .unwrap();
    os_lbl_multi.selected_data_sets = vec![0, 1, 2];
    assert!(matches!(
        run_analysis(&mv_lbl_sheet, &os_lbl_multi),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 2.0).abs() < 1e-12
    ));

    // OneSampleT [0] (only Label) -> rejected (never tests SubjectID).
    let mut os_lbl_only = os_lbl_multi.clone();
    os_lbl_only.selected_data_sets = vec![0];
    assert!(matches!(run_analysis(&mv_lbl_sheet, &os_lbl_only), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    // OneSampleT [0, 1] skips Label(0) and tests A1(1), never SubjectID.
    let mut os_lbl_and_a1 = os_lbl_multi;
    os_lbl_and_a1.selected_data_sets = vec![0, 1];
    let res_a1 = run_analysis(&mv_lbl_sheet, &os_lbl_and_a1).unwrap();
    assert_close(cell_num(&tab_by_id(&res_a1, "one-sample-t").rows[1][0]), 2.5, 1e-12, "OneSampleT [0,1] skips Label and uses A1");

    // Explicit mv_response_column: Some(0) works when caller specifically requests Label column.
    let os_explicit_lbl = AnalysisSpec::new(
        SheetId::new("os-exp-lbl"),
        mv_lbl_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::OneSampleT { hypothetical: 0.0 },
            mv_response_column: Some(0),
            ..Default::default()
        }),
    )
    .unwrap();
    let res_exp_lbl = run_analysis(&mv_lbl_sheet, &os_explicit_lbl).unwrap();
    assert_close(cell_num(&tab_by_id(&res_exp_lbl, "one-sample-t").rows[1][0]), 1001.5, 1e-12, "explicit mv_response_column=0");

    // UnpairedT [0, 1, 2] skips Label(0) and compares A1(1) vs A2(2), while [0, 1] fails (only 1 numeric col).
    let mut unp_lbl_3 = AnalysisSpec::new(
        SheetId::new("unp-lbl-3"),
        mv_lbl_sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
    )
    .unwrap();
    unp_lbl_3.selected_data_sets = vec![0, 1, 2];
    let res_unp_12 = run_analysis(&mv_lbl_sheet, &unp_lbl_3).unwrap();
    let exp_12 = unpaired_t_test(&[1.0, 2.0, 3.0, 4.0], &[1.5, 2.5, 3.5, 4.5], true, unp_lbl_3.confidence_level).unwrap();
    assert_close(scalar_val(&res_unp_12, "t"), exp_12.t_statistic, 1e-12, "UnpairedT [0,1,2] compares A1 vs A2");

    let mut unp_lbl_2 = unp_lbl_3;
    unp_lbl_2.selected_data_sets = vec![0, 1];
    assert!(matches!(
        run_analysis(&mv_lbl_sheet, &unp_lbl_2),
        Err(StatsError::InvalidParameter { name: "selected_data_sets", value }) if (value - 1.0).abs() < 1e-12
    ));

    // MultipleRowTTests [0, 1, 2, 3] excludes Label(0) -> only 3 numeric cols (< 4) -> InsufficientSampleSize, never pulls SubjectID into Group A.
    let mut mr_4 =
        AnalysisSpec::new(SheetId::new("mr-lbl-4"), mv_lbl_sheet.id.clone(), AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default()))
            .unwrap();
    mr_4.selected_data_sets = vec![0, 1, 2, 3];
    assert!(matches!(run_analysis(&mv_lbl_sheet, &mr_4), Err(StatsError::InsufficientSampleSize { required: 4, actual: 3, .. })));

    // MultipleRowTTests [0, 1, 2, 3, 4] excludes Label(0) and compares [A1, A2] vs [B1, B2] cleanly.
    let mut mr_5 = mr_4;
    mr_5.selected_data_sets = vec![0, 1, 2, 3, 4];
    let res_mr = run_analysis(&mv_lbl_sheet, &mr_5).unwrap();
    let cmp_tab = tab_by_id(&res_mr, "multiple-row-comparisons");
    assert_close(cell_num(&cmp_tab.rows[0][0]), 1.25, 1e-12, "Row 0 Mean(A) excludes SubjectID=1000");
    assert_close(cell_num(&cmp_tab.rows[0][1]), 10.25, 1e-12, "Row 0 Mean(B)");
}

// ============================================================================
// FA-SEC05-04: Extreme-tail Boschloo and CSM relative/log-space tolerances
// ============================================================================

#[test]
fn fa_sec05_04_boschloo_and_csm_extreme_tails_match_exact_d1_oracles() {
    // 1. Symmetric extreme table [[40, 0], [0, 40]]:
    //    D1 Python exact combinatorics: Fisher P = 2 / C(80, 40) = 1.8603403656036264e-23;
    //    Boschloo P = 2^(-79) = 1.6543612251060449e-24 at pi = 0.5.
    let fisher_40 = fisher_exact_2x2(40, 0, 0, 40).unwrap();
    assert_rel_close(fisher_40, 1.860_340_365_603_626_4e-23, 1e-10, "Fisher (40,0,0,40)");

    let boschloo_40 = barnard_exact_2x2_with_ordering(40, 0, 0, 40, BarnardOrdering::Boschloo).unwrap();
    assert!(!boschloo_40.used_asymptotic_fallback);
    assert_rel_close(boschloo_40.p_value, 1.654_361_225_106_045e-24, 1e-10, "Boschloo (40,0,0,40) P");
    assert!(boschloo_40.p_value <= fisher_40, "Boschloo must dominate Fisher");
    assert_close(boschloo_40.nuisance_pi, 0.5, 1e-6, "Boschloo (40,0,0,40) pi");

    // 2. Near-balanced extreme table [[30, 0], [1, 30]]:
    //    D1 Python exact: Boschloo P = 5.377642775528156e-17 (Fisher P = 1.3750773787711129e-16).
    let boschloo_30 = barnard_exact_2x2_with_ordering(30, 0, 1, 30, BarnardOrdering::Boschloo).unwrap();
    assert_rel_close(boschloo_30.p_value, 5.377_642_775_528_156e-17, 1e-9, "Boschloo (30,0,1,30) P");

    // 3. Asymmetric extreme tables where nuisance pi != 0.5:
    //    Checks both exact P < 1e-12 AND that symmetric tie-break in maximize_nuisance_pi does not flip pi to 1 - pi!
    //    D1 Python exact:
    //    - (25, 0, 0, 40): P = 1.5556454779833036e-19 at pi = 0.384817949797
    //    - (30, 0, 1, 15): P = 5.846276880745333e-12 at pi = 0.659205576796
    let b_asym_1 = barnard_exact_2x2_with_ordering(25, 0, 0, 40, BarnardOrdering::Boschloo).unwrap();
    assert_rel_close(b_asym_1.p_value, 1.555_645_477_983_303_6e-19, 1e-8, "Boschloo (25,0,0,40) P");
    assert_close(b_asym_1.nuisance_pi, 0.384_817_95, 1e-4, "Boschloo (25,0,0,40) skewed pi");

    let b_asym_2 = barnard_exact_2x2_with_ordering(30, 0, 1, 15, BarnardOrdering::Boschloo).unwrap();
    assert_rel_close(b_asym_2.p_value, 5.846_276_880_745_333e-12, 1e-8, "Boschloo (30,0,1,15) P");
    assert_close(b_asym_2.nuisance_pi, 0.659_205_58, 1e-4, "Boschloo (30,0,1,15) skewed pi");

    // 4. Extreme-tail CSM tables where additive + 1e-12 previously merged unrelated frontier pairs (~3.6x-10.3x inflation):
    //    D1 Python exact CSM:
    //    - (30, 5, 0, 35): P = 2.1503794838533810e-15 (additive gave ~2.215e-14)
    //    - (28, 3, 0, 31): P = 4.7462034302724164e-15 (additive gave ~1.725e-14)
    let csm_1 = barnard_exact_2x2_with_ordering(30, 5, 0, 35, BarnardOrdering::Csm).unwrap();
    assert!(!csm_1.used_asymptotic_fallback);
    assert_rel_close(csm_1.p_value, 2.150_379_483_853_381e-15, 1e-8, "CSM (30,5,0,35) P");

    let csm_2 = barnard_exact_2x2_with_ordering(28, 3, 0, 31, BarnardOrdering::Csm).unwrap();
    assert!(!csm_2.used_asymptotic_fallback);
    assert_rel_close(csm_2.p_value, 4.746_203_430_272_416e-15, 1e-8, "CSM (28,3,0,31) P");
}
