//! Regression and D1 reference-oracle tests for audit findings #17, #18, #22, #38, #41, #88, #89, #90.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::f64::consts::PI;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable,
    DataSheet, PartsOfWholeTable, ResultCell, ResultSheet, ResultTab, RocCiMethod, RocSpec, SheetId, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::rng::DeterministicRng;
use curvebook_stats::categorical::{analyze_2x2_ratios, analyze_contingency, cochran_armitage_trend, fleiss_kappa};
use curvebook_stats::power::{
    OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode, TwoProportionsMethod, solve_power,
};
use curvebook_stats::roc::roc_curve_analysis;
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected}, got {actual} (diff = {})", (actual - expected).abs());
}

fn scalar_opt(sheet: &ResultSheet, key: &str) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index.is_none()).map(|s| s.value)
}

fn scalar(sheet: &ResultSheet, key: &str) -> f64 {
    scalar_opt(sheet, key).unwrap_or_else(|| panic!("missing scalar `{key}` in {:?}", sheet.scalars))
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab `{id}`"))
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected numeric cell, got {other:?}"),
    }
}

fn cell_p(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected PValue cell, got {other:?}"),
    }
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

fn contingency_spec(src: &SheetId, method: ContingencyMethod) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new("res-ct"), src.clone(), AnalysisKind::Contingency(ContingencySpec { method, ..Default::default() })).unwrap()
}

fn parts_sheet_multi(series: &[(&str, &[(&str, f64)])]) -> DataSheet {
    let n_series = series.len();
    let n_rows = series[0].1.len();
    let mut pow = PartsOfWholeTable::new(n_series, n_rows).unwrap();
    pow.slice_labels = series[0].1.iter().map(|(lbl, _)| (*lbl).to_owned()).collect();
    for (c, (title, rows)) in series.iter().enumerate() {
        pow.data_sets[c].title = (*title).to_owned();
        for (r, (_, val)) in rows.iter().enumerate() {
            pow.data_sets[c].cells[r][0] = Cell::number(*val);
        }
    }
    DataSheet::new(SheetId::new("pow"), "PartsOfWhole", TableArchetype::PartsOfWhole(pow)).unwrap()
}

// ============================================================================
// #17: Empty rows or columns must be dropped before df / Cramer's V / CIs,
//      reduced tables < 2x2 rejected, and a visible note emitted on sheets.
// ============================================================================

#[test]
fn audit_17_contingency_drops_empty_margins_and_rejects_under_2x2() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. All-zero row [[10,2],[3,9],[0,0]] reduces to 2x2 [[10,2],[3,9]].
    // D1 R 4.5.2 reference: chisq.test(matrix(c(10,2,3,9), nrow=2, byrow=TRUE), correct=FALSE)
    //   chi2 = 8.223776223776223, df = 1, p = 0.004134507669588785.
    let padded_row = vec![vec![10, 2], vec![3, 9], vec![0, 0]];
    let exact_2x2 = vec![vec![10, 2], vec![3, 9]];
    let res_pad_row = analyze_contingency(&padded_row, false, conf).unwrap();
    let res_2x2 = analyze_contingency(&exact_2x2, false, conf).unwrap();
    assert_eq!(res_pad_row.n_rows, 2);
    assert_eq!(res_pad_row.n_cols, 2);
    assert_eq!(res_pad_row.dropped_empty_rows, 1);
    assert_eq!(res_pad_row.dropped_empty_cols, 0);
    assert_close(res_pad_row.df, 1.0, 1e-12, "padded row df");
    assert_close(res_pad_row.chi_square, 8.223_776_223_776_223, 1e-12, "padded row chi2");
    assert_close(res_pad_row.chi_square_p, 0.004_134_507_669_588_785, 1e-12, "padded row P");
    assert_close(res_pad_row.cramers_v, res_2x2.cramers_v, 1e-12, "padded row V");
    assert_eq!(res_pad_row.cramers_v_ci, res_2x2.cramers_v_ci);

    // 2. All-zero column [[10,2,0],[3,9,0],[4,4,0]] reduces to 3x2 [[10,2],[3,9],[4,4]].
    // D1 R 4.5.2 reference: chi2 = 8.240522875816994, df = 2, p = 0.01624026806517574, V = 0.5074606781508111.
    let padded_col = vec![vec![10, 2, 0], vec![3, 9, 0], vec![4, 4, 0]];
    let exact_3x2 = vec![vec![10, 2], vec![3, 9], vec![4, 4]];
    let res_pad_col = analyze_contingency(&padded_col, false, conf).unwrap();
    let res_3x2 = analyze_contingency(&exact_3x2, false, conf).unwrap();
    assert_eq!(res_pad_col.n_rows, 3);
    assert_eq!(res_pad_col.n_cols, 2);
    assert_eq!(res_pad_col.dropped_empty_rows, 0);
    assert_eq!(res_pad_col.dropped_empty_cols, 1);
    assert_close(res_pad_col.df, 2.0, 1e-12, "padded col df");
    assert_close(res_pad_col.chi_square, 8.240_522_875_816_994, 1e-12, "padded col chi2");
    assert_close(res_pad_col.chi_square_p, 0.016_240_268_065_175_74, 1e-12, "padded col P");
    assert_close(res_pad_col.cramers_v, 0.507_460_678_150_811_1, 1e-12, "padded col V");
    assert_eq!(res_pad_col.cramers_v_ci, res_3x2.cramers_v_ci);

    // 3. Sheet subset selection introducing an empty row emits a visible footnote & checklist item,
    //    and treats the reduced 2x2 table as 2x2 (applying Yates when requested).
    let sheet_sub = contingency_sheet(&[&[10, 2, 1], &[3, 9, 1], &[0, 0, 8]]);
    let mut spec_sub = contingency_spec(&sheet_sub.id, ContingencyMethod::PearsonChiSquare { yates_continuity: true });
    spec_sub.selected_data_sets = vec![0, 1];
    let out_sub = run_analysis(&sheet_sub, &spec_sub).unwrap();
    let main_tab = tab_by_id(&out_sub, "contingency-main");
    assert_eq!(main_tab.row_labels[0], "Chi-square (Yates)");
    assert_close(cell_num(&main_tab.rows[0][1]), 1.0, 1e-12, "reduced 2x2 df");
    assert!(
        main_tab.footnotes.iter().any(|f| f.contains("empty (all-zero) row(s)") && f.contains("2 x 2")),
        "missing footnote: {:?}",
        main_tab.footnotes
    );
    assert!(out_sub.checklist.iter().any(|c| c.assumption == "Empty margin reduction" && c.status == ChecklistStatus::Info));

    // 4. Tables reducing below 2x2 are rejected cleanly.
    for bad in [vec![vec![5_u64, 0], vec![3, 0]], vec![vec![5, 3], vec![0, 0]], vec![vec![0, 0, 0], vec![0, 5, 0], vec![0, 0, 0]]] {
        assert!(
            matches!(analyze_contingency(&bad, false, conf), Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })),
            "expected <2x2 rejection for {bad:?}"
        );
        let rows_ref: Vec<&[u64]> = bad.iter().map(Vec::as_slice).collect();
        let bad_sheet = contingency_sheet(&rows_ref);
        let bad_spec = contingency_spec(&bad_sheet.id, ContingencyMethod::PearsonChiSquare { yates_continuity: false });
        assert!(matches!(run_analysis(&bad_sheet, &bad_spec), Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })));
    }
    assert!(matches!(cochran_armitage_trend(&[vec![5, 0], vec![3, 0], vec![2, 0]]), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(cochran_armitage_trend(&[vec![10, 2], vec![3, 9], vec![0, 0]]), Err(StatsError::InvalidParameter { .. })));
}

// ============================================================================
// #18: Undefined 0/0 2x2 quantities must be None rather than Inf / 1 / 0 / P = 1.
// ============================================================================

#[test]
fn audit_18_undefined_2x2_quantities_unavailable() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // (5, 0, 3, 0): c2 == 0 -> OR = 0/0 (None), specificity = 0/0 (None) + CI None.
    let r_c2_zero = analyze_2x2_ratios(5, 0, 3, 0, conf).unwrap();
    assert_eq!(r_c2_zero.odds_ratio, None);
    assert_eq!(r_c2_zero.or_baptista_pike_ci, None);
    assert_close(r_c2_zero.relative_risk.unwrap(), 1.0, 1e-12, "5,0,3,0 RR");
    assert_close(r_c2_zero.sensitivity.unwrap(), 5.0 / 8.0, 1e-12, "5,0,3,0 sens");
    assert!(r_c2_zero.sensitivity_wilson_ci.is_some());
    assert_eq!(r_c2_zero.specificity, None);
    assert_eq!(r_c2_zero.specificity_wilson_ci, None);

    // (0, 5, 0, 3): c1 == 0 -> OR = 0/0 (None), RR = 0/0 (None), sensitivity = 0/0 (None) + CI None.
    let r_c1_zero = analyze_2x2_ratios(0, 5, 0, 3, conf).unwrap();
    assert_eq!(r_c1_zero.odds_ratio, None);
    assert_eq!(r_c1_zero.or_baptista_pike_ci, None);
    assert_eq!(r_c1_zero.relative_risk, None);
    assert_eq!(r_c1_zero.rr_koopman_ci, None);
    assert_eq!(r_c1_zero.sensitivity, None);
    assert_eq!(r_c1_zero.sensitivity_wilson_ci, None);
    assert_close(r_c1_zero.specificity.unwrap(), 3.0 / 8.0, 1e-12, "0,5,0,3 spec");
    assert!(r_c1_zero.specificity_wilson_ci.is_some());

    // Positive numerator / 0 denominator (non-0/0) stays +Infinity.
    let r_inf_or = analyze_2x2_ratios(5, 0, 1, 3, conf).unwrap();
    assert_eq!(r_inf_or.odds_ratio, Some(f64::INFINITY));
    assert_close(r_inf_or.relative_risk.unwrap(), 4.0, 1e-12, "5,0,1,3 RR");

    let r_inf_both = analyze_2x2_ratios(5, 5, 0, 3, conf).unwrap();
    assert_eq!(r_inf_both.odds_ratio, Some(f64::INFINITY));
    assert_eq!(r_inf_both.relative_risk, Some(f64::INFINITY));

    // Zero-margin contingency table rejected instead of returning chi2=0, P=1, Fisher P=1, phi=0.
    assert!(analyze_contingency(&[vec![5, 0], vec![3, 0]], false, conf).is_err());
}

// ============================================================================
// #22: Method-specific df and conditional "Chi-square (Yates)" label.
// ============================================================================

#[test]
fn audit_22_contingency_method_specific_df_and_yates_label() {
    // 1. Cochran-Armitage trend on 4x2 table has df = 1 (not full-table df = 3).
    // D1 R 4.5.2 reference: prop.trend.test(c(15,10,8,3), c(20,20,20,20)) -> z^2 = 14.58585858585859, df = 1, p = 0.0001339157382673294.
    let ca_sheet = contingency_sheet(&[&[15, 5], &[10, 10], &[8, 12], &[3, 17]]);
    let ca_res = run_analysis(&ca_sheet, &contingency_spec(&ca_sheet.id, ContingencyMethod::CochranArmitageTrend)).unwrap();
    let ca_tab = tab_by_id(&ca_res, "contingency-main");
    assert_eq!(ca_tab.row_labels[0], "Cochran-Armitage trend");
    assert_close(cell_num(&ca_tab.rows[0][0]).abs(), 3.819_143_697_985_006, 1e-10, "CA |z|");
    assert_close(cell_num(&ca_tab.rows[0][1]), 1.0, 1e-12, "CA df = 1");
    assert_close(cell_p(&ca_tab.rows[0][2]), 0.000_133_915_738_267_329_4, 1e-12, "CA P");

    // 2. Fisher exact (2x2 & RxC) and Barnard exact emit Empty df.
    let s2 = contingency_sheet(&[&[14, 6], &[5, 15]]);
    let f2_res = run_analysis(&s2, &contingency_spec(&s2.id, ContingencyMethod::FisherExact)).unwrap();
    assert_eq!(tab_by_id(&f2_res, "contingency-main").rows[0][1], ResultCell::Empty);

    let b2_res = run_analysis(&s2, &contingency_spec(&s2.id, ContingencyMethod::BarnardExact)).unwrap();
    assert_eq!(tab_by_id(&b2_res, "contingency-main").rows[0][1], ResultCell::Empty);

    let s3 = contingency_sheet(&[&[10, 4, 2], &[3, 12, 5], &[1, 3, 14]]);
    let f3_res = run_analysis(&s3, &contingency_spec(&s3.id, ContingencyMethod::FisherExact)).unwrap();
    assert_eq!(tab_by_id(&f3_res, "contingency-main").rows[0][1], ResultCell::Empty);

    // 3. "Chi-square (Yates)" emitted ONLY for 2x2 tables; 3x3 with yates_continuity=true emits "Pearson chi-square" and df = 4.
    let y2_res = run_analysis(&s2, &contingency_spec(&s2.id, ContingencyMethod::PearsonChiSquare { yates_continuity: true })).unwrap();
    let y2_tab = tab_by_id(&y2_res, "contingency-main");
    assert_eq!(y2_tab.row_labels[0], "Chi-square (Yates)");
    assert_close(cell_num(&y2_tab.rows[0][1]), 1.0, 1e-12, "Yates 2x2 df");

    let y3_res = run_analysis(&s3, &contingency_spec(&s3.id, ContingencyMethod::PearsonChiSquare { yates_continuity: true })).unwrap();
    let y3_tab = tab_by_id(&y3_res, "contingency-main");
    assert_eq!(y3_tab.row_labels[0], "Pearson chi-square");
    assert_close(cell_num(&y3_tab.rows[0][1]), 4.0, 1e-12, "3x3 df");
    assert!(y3_tab.footnotes.iter().any(|f| f.contains("2 x 2")));
}

// ============================================================================
// #38: DeLong ROC with n_pos == 1 or n_neg == 1 must not treat 0/0 variance as 0
//      or produce P < 0.05 on single-subject groups.
// ============================================================================

#[test]
fn audit_38_delong_roc_single_subject_guard() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let controls_10: Vec<f64> = (1..=10).map(f64::from).collect();
    let single_case = [9.5_f64];

    // DeLong refuses single-subject groups (S10 or S01 divides by n - 1 = 0).
    assert!(matches!(
        roc_curve_analysis(&controls_10, &single_case, RocCiMethod::DeLong, conf),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));
    assert!(matches!(
        roc_curve_analysis(&single_case, &controls_10, RocCiMethod::DeLong, conf),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));

    // Sheet path with DeLong also fails cleanly.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 10).unwrap();
    for (r, &v) in controls_10.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(v);
    }
    col.data_sets[1].cells[0][0] = Cell::number(9.5);
    let roc_sheet = DataSheet::new(SheetId::new("roc1"), "ROC1", TableArchetype::Column(col)).unwrap();
    let dl_spec = AnalysisSpec::new(SheetId::new("r-dl"), roc_sheet.id.clone(), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
    assert!(matches!(run_analysis(&roc_sheet, &dl_spec), Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })));

    // Hanley-McNeil on single-subject group uses exact Mann-Whitney U P (= 4/11 = 0.3636...) and Wilson CI, never P < 0.05.
    let hm_case1 = roc_curve_analysis(&controls_10, &single_case, RocCiMethod::HanleyMcNeil, conf).unwrap();
    assert_close(hm_case1.auc, 0.9, 1e-12, "single case AUC");
    assert!(hm_case1.se > 0.2, "Hanley-McNeil SE expected ~0.218, got {}", hm_case1.se);
    assert_close(hm_case1.p_value_vs_05, 4.0 / 11.0, 1e-12, "single case exact MW P");
    assert!(hm_case1.p_value_vs_05 >= 0.05);
    assert!(hm_case1.ci.0 < 0.5 && hm_case1.ci.1 > 0.5);

    let hm_ctrl1 = roc_curve_analysis(&[10.0], &controls_10, RocCiMethod::HanleyMcNeil, conf).unwrap();
    assert!(hm_ctrl1.p_value_vs_05 >= 0.05);

    // Even with n0 = 100 and 1 case near the top, Hanley-McNeil must never use Wald Z to yield P < 0.05.
    let controls_100: Vec<f64> = (1..=100).map(f64::from).collect();
    let hm_100 = roc_curve_analysis(&controls_100, &[99.5], RocCiMethod::HanleyMcNeil, conf).unwrap();
    assert!(hm_100.p_value_vs_05 >= 0.05, "expected non-significant P for n1=1, got {}", hm_100.p_value_vs_05);

    let hm_spec =
        AnalysisSpec::new(SheetId::new("r-hm"), roc_sheet.id.clone(), AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::HanleyMcNeil }))
            .unwrap();
    let hm_out = run_analysis(&roc_sheet, &hm_spec).unwrap();
    assert!(scalar(&hm_out, "P value (vs. 0.5)") >= 0.05);
    assert!(hm_out.checklist.iter().any(|c| c.assumption == "ROC small-sample inference" && c.status == ChecklistStatus::Warn));
}

// ============================================================================
// #41: Reject |h| > pi on input and reject unreachable DetectableEffect h > pi.
// ============================================================================

#[test]
fn audit_41_power_cohens_h_bounded_by_pi() {
    // 1. Standardized |h| > pi rejected for SampleSize, Power, and DetectableEffect.
    for bad_h in [5.0, -5.0, PI + 1e-6, -PI - 1e-6] {
        let req_n = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
            .with_two_proportions_mode(TwoProportionsMethod::CohensH)
            .with_effect(PowerEffectSpec::Standardized(bad_h));
        assert!(matches!(solve_power(&req_n), Err(StatsError::InvalidParameter { name: "cohens_h", .. })), "SampleSize h={bad_h}");

        let req_p = req_n.clone().with_target(PowerSolveTarget::Power).with_sample_size(20.0);
        assert!(matches!(solve_power(&req_p), Err(StatsError::InvalidParameter { name: "cohens_h", .. })), "Power h={bad_h}");
    }

    // Exact boundary |h| == pi is accepted.
    let req_pi = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
        .with_two_proportions_mode(TwoProportionsMethod::CohensH)
        .with_effect(PowerEffectSpec::Standardized(PI));
    let res_pi = solve_power(&req_pi).unwrap();
    assert_close(res_pi.effect_size, PI, 1e-12, "exact boundary h=pi");

    // 2. DetectableEffect with n=2, alpha=0.05, power=0.9 requires h = 3.2415 > pi -> must error.
    let req_unreachable = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_two_proportions_mode(TwoProportionsMethod::CohensH)
        .with_sample_size(2.0)
        .with_alpha(0.05)
        .with_target_power(0.90);
    assert!(matches!(solve_power(&req_unreachable), Err(StatsError::InvalidParameter { name: "target_power", .. })));

    // Also verify one-sided Greater and Less bounds at |h| = pi.
    let req_greater = req_unreachable.clone().with_tail(TailMode::OneSided(OneSidedDirection::Greater)).with_target_power(0.99);
    assert!(matches!(solve_power(&req_greater), Err(StatsError::InvalidParameter { name: "target_power", .. })));
    let req_less = req_unreachable.clone().with_tail(TailMode::OneSided(OneSidedDirection::Less)).with_target_power(0.99);
    assert!(matches!(solve_power(&req_less), Err(StatsError::InvalidParameter { name: "target_power", .. })));

    // Reachable DetectableEffect at n=10 succeeds with |h| <= pi.
    let req_ok = req_unreachable.with_sample_size(10.0);
    let res_ok = solve_power(&req_ok).unwrap();
    assert!(res_ok.effect_size.abs() <= PI && res_ok.achieved_power >= 0.90 - 1e-9);
}

// ============================================================================
// #88 & #89: Parts-of-whole GOF integer check + selected_data_sets handling.
// ============================================================================

#[test]
fn audit_88_89_parts_of_whole_integers_and_selected_series() {
    // 1. Non-integer slice values in GOF rejected with InvalidParameter.
    let non_int = parts_sheet_multi(&[("Frac", &[("A", 0.5), ("B", 0.3), ("C", 0.2)])]);
    let spec_gof = contingency_spec(&non_int.id, ContingencyMethod::PearsonChiSquare { yates_continuity: false });
    assert!(matches!(run_analysis(&non_int, &spec_gof), Err(StatsError::InvalidParameter { name: "goodness_of_fit_count", .. })));

    let partial_frac = parts_sheet_multi(&[("Mixed", &[("A", 10.0), ("B", 20.5), ("C", 30.0)])]);
    let spec_g2 = contingency_spec(&partial_frac.id, ContingencyMethod::LikelihoodRatioG2);
    assert!(matches!(run_analysis(&partial_frac, &spec_g2), Err(StatsError::InvalidParameter { name: "goodness_of_fit_count", .. })));

    // 2. Multi-series GOF honors selected_data_sets and rejects multi-series / empty multi-series / OOB.
    // D1 R 4.5.2: chisq.test(c(50,30,20)) -> chi2=14.0, p=0.0009118819655545154;
    //             chisq.test(c(34,33,33)) -> chi2=0.02, p=0.990049833749168.
    let multi_gof =
        parts_sheet_multi(&[("Series 0", &[("A", 50.0), ("B", 30.0), ("C", 20.0)]), ("Series 1", &[("A", 34.0), ("B", 33.0), ("C", 33.0)])]);

    let mut s0_spec = contingency_spec(&multi_gof.id, ContingencyMethod::PearsonChiSquare { yates_continuity: false });
    s0_spec.selected_data_sets = vec![0];
    let r0 = run_analysis(&multi_gof, &s0_spec).unwrap();
    assert_close(scalar(&r0, "Goodness-of-fit chi-square"), 14.0, 1e-12, "GOF series 0 chi2");
    assert_close(scalar(&r0, "P value"), 0.000_911_881_965_554_515_4, 1e-12, "GOF series 0 P");

    let mut s1_spec = s0_spec.clone();
    s1_spec.selected_data_sets = vec![1];
    let r1 = run_analysis(&multi_gof, &s1_spec).unwrap();
    assert_close(scalar(&r1, "Goodness-of-fit chi-square"), 0.02, 1e-12, "GOF series 1 chi2");
    assert_close(scalar(&r1, "P value"), 0.990_049_833_749_168, 1e-12, "GOF series 1 P");

    let mut multi_sel = s0_spec.clone();
    multi_sel.selected_data_sets = vec![0, 1];
    assert!(matches!(run_analysis(&multi_gof, &multi_sel), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut empty_sel = s0_spec.clone();
    empty_sel.selected_data_sets = vec![];
    assert!(matches!(run_analysis(&multi_gof, &empty_sel), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut oob_sel = s0_spec;
    oob_sel.selected_data_sets = vec![5];
    assert!(matches!(run_analysis(&multi_gof, &oob_sel), Err(StatsError::MissingColumn { index: 5 })));

    // 3. Multi-series Exact Binomial honors selected_data_sets and rejects multi-series / empty multi-series / OOB.
    let multi_bin = parts_sheet_multi(&[("Skewed", &[("Success", 9.0), ("Failure", 1.0)]), ("Balanced", &[("Success", 5.0), ("Failure", 5.0)])]);
    let mut b0_spec = contingency_spec(&multi_bin.id, ContingencyMethod::ExactBinomial);
    b0_spec.selected_data_sets = vec![0];
    let rb0 = run_analysis(&multi_bin, &b0_spec).unwrap();
    assert_close(scalar(&rb0, "successes"), 9.0, 1e-12, "Binomial series 0 k");
    assert_close(scalar(&rb0, "p_value"), 0.021_484_375, 1e-12, "Binomial series 0 P");

    let mut b1_spec = b0_spec.clone();
    b1_spec.selected_data_sets = vec![1];
    let rb1 = run_analysis(&multi_bin, &b1_spec).unwrap();
    assert_close(scalar(&rb1, "successes"), 5.0, 1e-12, "Binomial series 1 k");
    assert_close(scalar(&rb1, "p_value"), 1.0, 1e-12, "Binomial series 1 P");

    let mut b_multi = b0_spec.clone();
    b_multi.selected_data_sets = vec![0, 1];
    assert!(matches!(run_analysis(&multi_bin, &b_multi), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut b_empty = b0_spec.clone();
    b_empty.selected_data_sets = vec![];
    assert!(matches!(run_analysis(&multi_bin, &b_empty), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));

    let mut b_oob = b0_spec;
    b_oob.selected_data_sets = vec![3];
    assert!(matches!(run_analysis(&multi_bin, &b_oob), Err(StatsError::MissingColumn { index: 3 })));
}

// ============================================================================
// #90: Fleiss's kappa non-null asymptotic SE & CI (Fleiss, Levin & Paik 2003)
//      with Fleiss, Nee & Landis (1979) null SE retained strictly for z-test.
// ============================================================================

fn null_fleiss_se(counts: &[Vec<u64>]) -> f64 {
    let n = counts.len() as f64;
    let k = counts[0].len();
    let m: u64 = counts[0].iter().sum();
    let mf = m as f64;
    let mut col_sums = vec![0.0_f64; k];
    for row in counts {
        for (j, &c) in row.iter().enumerate() {
            col_sums[j] += c as f64;
        }
    }
    let p_j: Vec<f64> = col_sums.iter().map(|&s| s / (n * mf)).collect();
    let pe: f64 = p_j.iter().map(|&p| p * p).sum();
    let sum_pj3: f64 = p_j.iter().map(|&p| p * p * p).sum();
    let denom = n * mf * (mf - 1.0) * (1.0 - pe).powi(2);
    (2.0 * (pe - (2.0 * mf - 3.0) * pe * pe + 2.0 * (mf - 2.0) * sum_pj3) / denom).max(0.0).sqrt()
}

#[test]
fn audit_90_fleiss_kappa_non_null_se_ci_and_coverage() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let z_95 = 1.959_963_984_540_054;

    // 1. Fixed 30-subject, 4-rater, 3-category table: verify z_statistic uses SE_0 while displayed SE & CI use non-null SE.
    let fixed_table: Vec<Vec<u64>> = vec![
        vec![4, 0, 0],
        vec![3, 1, 0],
        vec![4, 0, 0],
        vec![2, 2, 0],
        vec![4, 0, 0],
        vec![3, 0, 1],
        vec![4, 0, 0],
        vec![3, 1, 0],
        vec![2, 1, 1],
        vec![4, 0, 0],
        vec![0, 4, 0],
        vec![0, 3, 1],
        vec![1, 3, 0],
        vec![0, 4, 0],
        vec![0, 2, 2],
        vec![1, 2, 1],
        vec![0, 4, 0],
        vec![0, 3, 1],
        vec![1, 3, 0],
        vec![0, 4, 0],
        vec![0, 0, 4],
        vec![0, 1, 3],
        vec![1, 0, 3],
        vec![0, 0, 4],
        vec![0, 2, 2],
        vec![1, 1, 2],
        vec![0, 0, 4],
        vec![0, 1, 3],
        vec![1, 0, 3],
        vec![0, 0, 4],
    ];
    let res_fixed = fleiss_kappa(&fixed_table, conf).unwrap();
    let se0_fixed = null_fleiss_se(&fixed_table);
    assert_close(res_fixed.z_statistic, res_fixed.kappa / se0_fixed, 1e-12, "z_statistic must use null SE_0");
    assert!(res_fixed.se > se0_fixed, "non-null SE ({}) must exceed null SE ({se0_fixed}) on clustered agreement", res_fixed.se);
    assert_close(res_fixed.ci.0, (res_fixed.kappa - z_95 * res_fixed.se).clamp(-1.0, 1.0), 1e-12, "CI lower uses non-null SE");
    assert_close(res_fixed.ci.1, (res_fixed.kappa + z_95 * res_fixed.se).clamp(-1.0, 1.0), 1e-12, "CI upper uses non-null SE");

    // 2. Deterministic Monte Carlo coverage check (3,000 replicates, N=60, m=4, K=3, unequal class priors [0.5, 0.3, 0.2]).
    //    Latent class j drawn from [0.5, 0.3, 0.2]; each rater selects j with prob q=0.80 and each other class with prob 0.10.
    //    Exact marginals: p = [0.45, 0.31, 0.24], Pe = 0.3562, E[P_i] = 0.80^2 + 2 * 0.10^2 = 0.66,
    //    exact population kappa = (0.66 - 0.3562) / (1 - 0.3562) = 0.4718856787822305.
    let pop_kappa = (0.66 - 0.3562) / (1.0 - 0.3562);
    let reps = 3_000_usize;
    let n_subj = 60_usize;
    let m_raters = 4_usize;
    let mut rng = DeterministicRng::from_seed(0xF1E1_55CA_99A0_2026);

    let mut kappas = Vec::with_capacity(reps);
    let mut se_non_null_sum = 0.0_f64;
    let mut se_null_sum = 0.0_f64;
    let mut covered_non_null = 0_usize;
    let mut covered_null = 0_usize;

    for _ in 0..reps {
        let mut table = Vec::with_capacity(n_subj);
        for _ in 0..n_subj {
            let u_cls = rng.next_f64();
            let true_c = if u_cls < 0.5 {
                0
            } else if u_cls < 0.8 {
                1
            } else {
                2
            };
            let mut row = vec![0_u64; 3];
            for _ in 0..m_raters {
                let u_r = rng.next_f64();
                let assigned = if u_r < 0.80 {
                    true_c
                } else if u_r < 0.90 {
                    (true_c + 1) % 3
                } else {
                    (true_c + 2) % 3
                };
                row[assigned] += 1;
            }
            table.push(row);
        }
        let fk = fleiss_kappa(&table, conf).unwrap();
        let se0 = null_fleiss_se(&table);
        kappas.push(fk.kappa);
        se_non_null_sum += fk.se;
        se_null_sum += se0;
        if fk.ci.0 <= pop_kappa && pop_kappa <= fk.ci.1 {
            covered_non_null += 1;
        }
        if (fk.kappa - z_95 * se0) <= pop_kappa && pop_kappa <= (fk.kappa + z_95 * se0) {
            covered_null += 1;
        }
    }

    let mean_k = kappas.iter().sum::<f64>() / (reps as f64);
    let emp_sd = (kappas.iter().map(|&k| (k - mean_k).powi(2)).sum::<f64>() / ((reps - 1) as f64)).sqrt();
    let mean_se = se_non_null_sum / (reps as f64);
    let mean_se0 = se_null_sum / (reps as f64);
    let cov = (covered_non_null as f64) / (reps as f64);
    let cov0 = (covered_null as f64) / (reps as f64);

    assert!((mean_se - emp_sd).abs() < 0.1 * emp_sd, "mean non-null SE {mean_se:.4} must match empirical SD {emp_sd:.4}");
    assert!((0.925..=0.975).contains(&cov), "95% CI coverage must be ~95%, got {cov:.4} (null cov0 = {cov0:.4}, se0 = {mean_se0:.4})");
    assert!(cov0 < 0.88, "null SE must noticeably undercover when kappa != 0, got cov0 = {cov0:.4}");
}
