#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Audit findings #4, #7, #8, #13, #15, and #16 (`crates/stats/*`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, GlmFamily, GlmSpec,
    MultipleRowTestsSpec, MultipleVariablesTable, MultiplicitySpec, ResultCell, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::column::one_sample_t_test;
use curvebook_stats::survival::{CoxSubject, cox_ph_regression};
use curvebook_stats::two_group::{bland_altman, one_sample_t_test_from_summary, paired_t_test, ratio_paired_t_test};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_approx_eq;

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

fn tab_by_id<'a>(sheet: &'a curvebook_doc::ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected Number cell, got {other:?}"),
    }
}

// ============================================================================
// Audit #4: Bland-Altman explicit column validation on XY tables
// ============================================================================

#[test]
fn audit_04_bland_altman_xy_rejects_invalid_and_equal_indices() {
    let mut xy1 = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy1.x_title = String::from("Ref");
    xy1.data_sets[0].title = String::from("Cand");
    let a = [10.0, 20.0, 30.0, 40.0, 50.0];
    let b = [10.4, 19.6, 30.8, 39.2, 50.5];
    for r in 0..5 {
        xy1.set_x_cell(r, Cell::number(a[r])).unwrap();
        xy1.data_sets[0].cells[r][0] = Cell::number(b[r]);
    }
    let sheet1 = DataSheet::new(SheetId::new("xy1"), "XY1", TableArchetype::Xy(xy1)).unwrap();

    // Valid default (0, 1) on 1-dataset XY compares X vs Y[0].
    let direct = bland_altman(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let ok_res = run_analysis(
        &sheet1,
        &spec_for("r-ok", "xy1", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false })),
    )
    .unwrap();
    let bias = ok_res.scalars.iter().find(|s| s.key == "Bias").unwrap().value;
    assert_approx_eq(bias, direct.bias, 1e-12, 1e-12, "XY1 default (0,1) matches direct");

    // Out-of-range explicit indices must fail cleanly with MissingColumn instead of falling back to Y[0].
    let err_99_98 = run_analysis(
        &sheet1,
        &spec_for("r-bad1", "xy1", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 99, column_b: 98, percent_difference: false })),
    )
    .unwrap_err();
    assert!(matches!(err_99_98, StatsError::MissingColumn { .. }), "got {err_99_98:?}");

    let err_0_98 = run_analysis(
        &sheet1,
        &spec_for("r-bad2", "xy1", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 98, percent_difference: false })),
    )
    .unwrap_err();
    assert!(matches!(err_0_98, StatsError::MissingColumn { index: 98 }), "got {err_0_98:?}");

    let err_same_1ds = run_analysis(
        &sheet1,
        &spec_for("r-same1", "xy1", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 0, percent_difference: false })),
    )
    .unwrap_err();
    assert!(matches!(err_same_1ds, StatsError::InvalidParameter { name: "column_b", .. }), "got {err_same_1ds:?}");

    // 2-dataset XY table compares Y[column_a] vs Y[column_b], rejecting equal and out-of-range indices.
    let mut xy2 = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
    xy2.data_sets[0].title = String::from("Y0");
    xy2.data_sets[1].title = String::from("Y1");
    for r in 0..5 {
        xy2.set_x_cell(r, Cell::number(r as f64)).unwrap();
        xy2.data_sets[0].cells[r][0] = Cell::number(a[r]);
        xy2.data_sets[1].cells[r][0] = Cell::number(b[r]);
    }
    let sheet2 = DataSheet::new(SheetId::new("xy2"), "XY2", TableArchetype::Xy(xy2)).unwrap();

    let ok2 = run_analysis(
        &sheet2,
        &spec_for("r-ok2", "xy2", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false })),
    )
    .unwrap();
    let bias2 = ok2.scalars.iter().find(|s| s.key == "Bias").unwrap().value;
    assert_approx_eq(bias2, direct.bias, 1e-12, 1e-12, "XY2 (0,1) matches Y0 vs Y1");

    assert!(matches!(
        run_analysis(
            &sheet2,
            &spec_for("r-same2", "xy2", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 0, percent_difference: false })),
        ),
        Err(StatsError::InvalidParameter { name: "column_b", .. })
    ));
    assert!(matches!(
        run_analysis(
            &sheet2,
            &spec_for("r-oob2", "xy2", AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 5, percent_difference: false })),
        ),
        Err(StatsError::MissingColumn { index: 5 })
    ));
}

// ============================================================================
// Audit #7: Odd column counts rejected cleanly on paired multiple row t-tests
// ============================================================================

#[test]
fn audit_07_paired_multiple_row_t_tests_rejects_odd_column_counts() {
    // 1. Column table with 7 columns (3 pairs + trailing unmatched column).
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 7, 3).unwrap();
    for c in 0..7 {
        col.data_sets[c].title = format!("C{c}");
        for r in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number((c + r) as f64);
        }
    }
    let col_sheet = DataSheet::new(SheetId::new("col-odd"), "ColOdd", TableArchetype::Column(col)).unwrap();
    let paired_kind = AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
        paired: true,
        nonparametric: false,
        assume_equal_sd: false,
        multiplicity: MultiplicitySpec::default(),
    });
    let err_col = run_analysis(&col_sheet, &spec_for("r-col-odd", "col-odd", paired_kind.clone())).unwrap_err();
    assert!(matches!(err_col, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - 7.0).abs() < 1e-12));

    // Also reject odd subset on an 8-column table when 5 columns are explicitly selected.
    let mut col8 = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 8, 3).unwrap();
    for c in 0..8 {
        for r in 0..3 {
            col8.data_sets[c].cells[r][0] = Cell::number((c * 2 + r) as f64);
        }
    }
    let col8_sheet = DataSheet::new(SheetId::new("col8"), "Col8", TableArchetype::Column(col8)).unwrap();
    let mut spec5 = spec_for("r-col5", "col8", paired_kind.clone());
    spec5.selected_data_sets = vec![0, 1, 2, 3, 4];
    let err_sel = run_analysis(&col8_sheet, &spec5).unwrap_err();
    assert!(matches!(err_sel, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - 5.0).abs() < 1e-12));

    // 2. MultipleVariables table with 5 numeric columns.
    let mut mv = MultipleVariablesTable::new(5, 3).unwrap();
    for c in 0..5 {
        mv.variables[c].title = format!("V{c}");
        for r in 0..3 {
            mv.variables[c].cells[r] = Cell::number((c + r + 1) as f64);
        }
    }
    let mv_sheet = DataSheet::new(SheetId::new("mv-odd"), "MvOdd", TableArchetype::MultipleVariables(mv)).unwrap();
    let err_mv = run_analysis(&mv_sheet, &spec_for("r-mv-odd", "mv-odd", paired_kind)).unwrap_err();
    assert!(matches!(err_mv, StatsError::InvalidParameter { name: "paired_selected_columns", value } if (value - 5.0).abs() < 1e-12));
}

// ============================================================================
// Audit #8: df = 1 paired & one-sample t-tests succeed with Hedges' g None
// ============================================================================

#[test]
fn audit_08_two_pair_df1_paired_and_one_sample_t_tests_return_primary_results() {
    let a = [10.0, 12.0];
    let b = [7.0, 11.0];

    // Direct paired_t_test (df = 1): d = [3, 1], mean = 2, sd = sqrt(2), sem = 1, t = 2, df = 1.
    let pt = paired_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pt.df, 1.0, 1e-12, 1e-12, "paired df=1");
    assert_approx_eq(pt.mean_of_differences, 2.0, 1e-12, 1e-12, "paired mean diff");
    assert_approx_eq(pt.t_statistic, 2.0, 1e-12, 1e-12, "paired t");
    assert!(pt.p_value.is_finite() && pt.ci.0.is_finite() && pt.ci.1.is_finite());
    assert!(pt.cohens_dz.is_finite());
    assert_eq!(pt.hedges_gz, None);
    assert_eq!(pt.hedges_gz_ci, None);

    // Direct ratio_paired_t_test (df = 1): log ratios differ across pairs.
    let rpt = ratio_paired_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(rpt.df, 1.0, 1e-12, 1e-12, "ratio paired df=1");
    assert!(rpt.t_statistic.is_finite() && rpt.p_value.is_finite());
    assert_eq!(rpt.hedges_gz, None);
    assert_eq!(rpt.hedges_gz_ci, None);

    // Direct one_sample_t_test & summary variant (df = 1).
    let os = one_sample_t_test(&[3.0, 1.0], 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(os.df, 1.0, 1e-12, 1e-12, "one-sample df=1");
    assert_approx_eq(os.t_statistic, 2.0, 1e-12, 1e-12, "one-sample t");
    assert_eq!(os.hedges_g, None);
    assert_eq!(os.hedges_g_ci, None);

    let os_sum = one_sample_t_test_from_summary(2.0, 2.0_f64.sqrt(), 2.0, 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(os_sum.p_value, os.p_value, 1e-12, 1e-12, "summary df=1 matches raw");
    assert_eq!(os_sum.hedges_g, None);

    // Single two-group sheet invocation (both compute_effect_sizes = false and true) succeeds cleanly.
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 2).unwrap();
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("After");
    for r in 0..2 {
        col.data_sets[0].cells[r][0] = Cell::number(a[r]);
        col.data_sets[1].cells[r][0] = Cell::number(b[r]);
    }
    let sheet = DataSheet::new(SheetId::new("col-2pair"), "TwoPair", TableArchetype::Column(col)).unwrap();
    for compute_es in [false, true] {
        let res = run_analysis(
            &sheet,
            &spec_for(
                "r-2p",
                "col-2pair",
                AnalysisKind::TwoGroupComparison(TwoGroupSpec {
                    method: TwoGroupMethod::PairedT,
                    two_tailed: true,
                    compute_effect_sizes: compute_es,
                    mv_group_column: None,
                    mv_response_column: None,
                }),
            ),
        )
        .unwrap();
        let summary_t = res.scalars.iter().find(|s| s.key == "t").unwrap().value;
        assert_approx_eq(summary_t, 2.0, 1e-12, 1e-12, "sheet paired t df=1");
        if compute_es {
            let gz_scalar = res.scalars.iter().find(|s| s.key == "Hedges g_z");
            assert!(gz_scalar.is_none(), "Hedges g_z scalar must be omitted when unavailable at df=1");
        }
    }

    // MultipleRowTTests paired row with 2 pairs must include the row rather than silently skipping it.
    let mut row_col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 1).unwrap();
    row_col.row_titles = vec![String::from("TwoPairRow")];
    for (c, &v) in [a[0], b[0], a[1], b[1]].iter().enumerate() {
        row_col.data_sets[c].cells[0][0] = Cell::number(v);
    }
    let row_sheet = DataSheet::new(SheetId::new("row-2p"), "RowTwoPair", TableArchetype::Column(row_col)).unwrap();
    let row_res = run_analysis(
        &row_sheet,
        &spec_for(
            "r-row-2p",
            "row-2p",
            AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec {
                paired: true,
                nonparametric: false,
                assume_equal_sd: false,
                multiplicity: MultiplicitySpec::default(),
            }),
        ),
    )
    .unwrap();
    let cmp_tab = row_res.tabs.iter().find(|t| t.kind == ResultTabKind::MultipleComparisons).expect("MultipleComparisons tab");
    assert_eq!(cmp_tab.rows.len(), 1);
    assert_eq!(cmp_tab.row_labels, vec![String::from("TwoPairRow")]);
}

// ============================================================================
// Audit #13 & #15: XY GLM response selection / validation + source row indices
// ============================================================================

#[test]
fn audit_13_xy_glm_honors_selected_dataset_and_rejects_bad_indices() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).unwrap();
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("NullResp");
    xy.data_sets[1].title = String::from("SteepResp");
    for r in 0..6 {
        let x = (r + 1) as f64;
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(1.0 + 0.5 * x);
        xy.data_sets[1].cells[r][0] = Cell::number(5.0 + 10.0 * x);
    }
    let sheet = DataSheet::new(SheetId::new("xy-glm"), "XyGlm", TableArchetype::Xy(xy)).unwrap();

    // 1. Selecting dataset 1 via `selected_data_sets = [1]` must fit SteepResp (slope ~ 10.0).
    let mut spec_ds1 = spec_for(
        "r-xy-ds1",
        "xy-glm",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    );
    spec_ds1.selected_data_sets = vec![1];
    let res_ds1 = run_analysis(&sheet, &spec_ds1).unwrap();
    let slope_ds1 = cell_num(&tab_by_id(&res_ds1, "glm-linear-params").rows[1][0]);
    assert_approx_eq(slope_ds1, 10.0, 1e-10, 1e-10, "selected_data_sets=[1] slope");

    // 2. Explicit `response_column: 1` (with empty selected_data_sets) must also fit SteepResp (slope ~ 10.0).
    let spec_rc1 = spec_for(
        "r-xy-rc1",
        "xy-glm",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 1,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    );
    let res_rc1 = run_analysis(&sheet, &spec_rc1).unwrap();
    let slope_rc1 = cell_num(&tab_by_id(&res_rc1, "glm-linear-params").rows[1][0]);
    assert_approx_eq(slope_rc1, 10.0, 1e-10, 1e-10, "response_column=1 slope");

    // 3. Invalid response_column / selected_data_sets / predictor_columns must return typed errors.
    let err_bad_rc = run_analysis(
        &sheet,
        &spec_for(
            "r-xy-badrc",
            "xy-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 9,
                predictor_columns: vec![0],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_bad_rc, StatsError::MissingColumn { index: 9 }), "got {err_bad_rc:?}");

    let mut spec_bad_ds = spec_ds1.clone();
    spec_bad_ds.selected_data_sets = vec![9];
    assert!(matches!(run_analysis(&sheet, &spec_bad_ds), Err(StatsError::MissingColumn { index: 9 })));

    let err_empty_pred = run_analysis(
        &sheet,
        &spec_for(
            "r-xy-emptyp",
            "xy-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_empty_pred, StatsError::InsufficientSampleSize { .. }), "got {err_empty_pred:?}");

    let err_bad_pred = run_analysis(
        &sheet,
        &spec_for(
            "r-xy-badp",
            "xy-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![4],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_bad_pred, StatsError::MissingColumn { index: 4 }), "got {err_bad_pred:?}");

    let err_two_preds = run_analysis(
        &sheet,
        &spec_for(
            "r-xy-twop",
            "xy-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![0, 1],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_two_preds, StatsError::InvalidParameter { name: "predictor_columns", .. }), "got {err_two_preds:?}");
}

#[test]
fn audit_15_glm_residuals_and_predictions_preserve_source_row_indices() {
    // 6-row MV table where 1-based Row 2 (index 1) has missing Y and Row 5 (index 4) has excluded X.
    // Complete cases come from 1-based source rows [1, 3, 4, 6].
    let mut mv = MultipleVariablesTable::new(2, 6).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X");
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ys_lin = [2.0, 999.0, 6.1, 7.9, 10.2, 12.1];
    let ys_bin = [0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let ys_cnt = [1.0, 9.0, 3.0, 6.0, 8.0, 14.0];

    let expected_labels = vec![String::from("Row 1"), String::from("Row 3"), String::from("Row 4"), String::from("Row 6")];

    for (family, ys, tab_id) in [
        (GlmFamily::Linear, ys_lin, "glm-linear-residuals"),
        (GlmFamily::Logistic, ys_bin, "glm-logistic-predictions"),
        (GlmFamily::Poisson, ys_cnt, "glm-poisson-residuals"),
    ] {
        for r in 0..6 {
            mv.variables[0].cells[r] = if r == 1 { Cell::empty() } else { Cell::number(ys[r]) };
            mv.variables[1].cells[r] =
                if r == 4 { Cell { value: curvebook_doc::CellValue::Number(xs[r]), excluded: true } } else { Cell::number(xs[r]) };
        }
        let sheet = DataSheet::new(SheetId::new("mv-rows"), "MvRows", TableArchetype::MultipleVariables(mv.clone())).unwrap();
        let res = run_analysis(
            &sheet,
            &spec_for(
                "r-rows",
                "mv-rows",
                AnalysisKind::GeneralizedLinearModel(GlmSpec {
                    family,
                    response_column: 0,
                    predictor_columns: vec![1],
                    reference_levels: Vec::new(),
                    compute_vif: false,
                }),
            ),
        )
        .unwrap();
        let tab = tab_by_id(&res, tab_id);
        assert_eq!(tab.row_labels, expected_labels, "{family:?} should preserve true 1-based source row labels");
    }

    // Also verify XY table residuals preserve true 1-based source rows after skipping excluded row 2.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = if r == 1 { Cell::empty() } else { Cell::number((r + 1) as f64 * 2.5) };
    }
    let xy_sheet = DataSheet::new(SheetId::new("xy-rows"), "XyRows", TableArchetype::Xy(xy)).unwrap();
    let xy_res = run_analysis(
        &xy_sheet,
        &spec_for(
            "r-xyrows",
            "xy-rows",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![0],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap();
    assert_eq!(
        tab_by_id(&xy_res, "glm-linear-residuals").row_labels,
        vec![String::from("Row 1"), String::from("Row 3"), String::from("Row 4"), String::from("Row 5")]
    );
}

// ============================================================================
// Audit #16: Zero-comparable-pairs Cox PH concordance reported as unavailable
// ============================================================================

#[test]
fn audit_16_cox_ph_zero_comparable_pairs_reports_unavailable_concordance() {
    // All 4 subjects experience the event at the exact same time t = 10.0, so zero pairs are comparable.
    let subjects = vec![
        CoxSubject { time: 10.0, event: true, x: vec![0.0] },
        CoxSubject { time: 10.0, event: true, x: vec![1.0] },
        CoxSubject { time: 10.0, event: true, x: vec![2.0] },
        CoxSubject { time: 10.0, event: true, x: vec![3.0] },
    ];
    let fit = cox_ph_regression(&subjects, &[String::from("Dose")], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(fit.comparable_pairs, 0);
    assert_eq!(fit.concordance, None);

    // End-to-end sheet execution omits "Concordance (C-index)", keeps "Comparable pairs" = 0, and emits checklist note.
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Dose");
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::number(10.0);
        mv.variables[1].cells[r] = Cell::number(1.0);
        mv.variables[2].cells[r] = Cell::number(r as f64);
    }
    let sheet = DataSheet::new(SheetId::new("mv-cox0"), "Cox0", TableArchetype::MultipleVariables(mv)).unwrap();
    let res = run_analysis(
        &sheet,
        &spec_for(
            "r-cox0",
            "mv-cox0",
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: Vec::new(),
                tie_method: CoxTieMethod::Breslow,
            }),
        ),
    )
    .unwrap();
    assert!(res.scalars.iter().all(|s| s.key != "Concordance (C-index)"), "Concordance scalar must not emit pseudo-0.5 when comparable_pairs == 0");
    let pairs_scalar = res.scalars.iter().find(|s| s.key == "Comparable pairs").unwrap().value;
    assert_approx_eq(pairs_scalar, 0.0, 1e-12, 1e-12, "Comparable pairs == 0");

    let chk = res.checklist.iter().find(|c| c.assumption.contains("Harrell's C-index")).expect("discrimination checklist entry");
    assert_eq!(chk.status, ChecklistStatus::Info);
    assert!(chk.diagnostic_metric.as_deref().unwrap_or("").contains("— (0 comparable pairs)"), "got diagnostic_metric {:?}", chk.diagnostic_metric);
    assert!(matches!(tab_by_id(&res, "cox-ph").kind, ResultTabKind::ParametersAndConfidenceIntervals));
    assert!(tab_by_id(&res, "cox-ph").footnotes.iter().any(|f| f.contains("unavailable")));
}
