//! Stage 3A regression tests for S02 silent-wrong-statistics fixes in `lumencraft-stats`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable,
    CoxTieMethod, DataSheet, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, OneWayMethod, ResultCell, RocCiMethod,
    RocSpec, SheetId, SurvivalAnalysisSpec, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_stats::categorical::{cochran_armitage_trend, fisher_exact_rxc};
use lumencraft_stats::column::one_sample_wilcoxon;
use lumencraft_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_p_values, compare_dunn_ranks, compare_groups_with_procedure,
};
use lumencraft_stats::roc::roc_curve_analysis;
use lumencraft_stats::survival::{CoxSubject, cox_ph_regression};
use lumencraft_stats::two_group::{mann_whitney_u, wilcoxon_matched_pairs};
use lumencraft_stats::{StatsError, run_analysis};

fn contingency_sheet(rows: &[&[u64]]) -> DataSheet {
    let n_rows = rows.len();
    let n_cols = rows[0].len();
    let mut tab = ContingencyTable::new(n_cols, n_rows).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(val as f64);
        }
    }
    DataSheet::new(SheetId::new("ct-s"), "Contingency", TableArchetype::Contingency(tab)).unwrap()
}

fn column_sheet(cols: &[(&str, &[f64])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new("col-s"), "Columns", TableArchetype::Column(tab)).unwrap()
}

fn scalar_by_name(sheet: &lumencraft_doc::ResultSheet, name: &str) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == name).map(|s| s.value)
}

#[test]
fn r1_s02_02_categorical_no_silent_chi_square_fallback() {
    // Cochran-Armitage: non-2xR/Rx2 (3x3) and insufficient ordered levels (2x2) return Err.
    assert!(matches!(cochran_armitage_trend(&[vec![10, 5, 2], vec![4, 8, 6], vec![1, 3, 9]]), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(cochran_armitage_trend(&[vec![10, 5], vec![4, 8]]), Err(StatsError::InvalidParameter { .. })));

    let bad_3x3: &[&[u64]] = &[&[10, 5, 2], &[4, 8, 6], &[1, 3, 9]];
    let bad_2x2: &[&[u64]] = &[&[10, 5], &[4, 8]];
    for bad_rows in [bad_3x3, bad_2x2] {
        let sheet = contingency_sheet(bad_rows);
        let spec = AnalysisSpec::new(
            SheetId::new("r-ca"),
            sheet.id.clone(),
            AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CochranArmitageTrend, ..Default::default() }),
        )
        .unwrap();
        assert!(matches!(run_analysis(&sheet, &spec), Err(StatsError::InvalidParameter { .. })));
    }

    // Valid 3x2 Cochran-Armitage succeeds.
    let valid_ca = contingency_sheet(&[&[5, 15], &[10, 10], &[16, 4]]);
    let spec_ca = AnalysisSpec::new(
        SheetId::new("r-ca-ok"),
        valid_ca.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CochranArmitageTrend, ..Default::default() }),
    )
    .unwrap();
    let res_ca = run_analysis(&valid_ca, &spec_ca).unwrap();
    assert_eq!(res_ca.tabs[0].row_labels[0], "Cochran-Armitage trend");

    // BarnardExact: non-2x2 rejected and > cap 2x2 rejected (never silent asymptotic or chi_square_p).
    let non_2x2 = contingency_sheet(&[&[5, 10, 15], &[12, 8, 5]]);
    let spec_b1 = AnalysisSpec::new(
        SheetId::new("r-b1"),
        non_2x2.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::BarnardExact, ..Default::default() }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&non_2x2, &spec_b1), Err(StatsError::InvalidParameter { .. })));

    let big_2x2_sym = contingency_sheet(&[&[1_500, 1_500], &[1_500, 1_500]]);
    let big_2x2_asym = contingency_sheet(&[&[1_600, 1_400], &[1_400, 1_600]]);
    for big in [&big_2x2_sym, &big_2x2_asym] {
        let spec_b2 = AnalysisSpec::new(
            SheetId::new("r-b2"),
            big.id.clone(),
            AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::BarnardExact, ..Default::default() }),
        )
        .unwrap();
        assert!(matches!(run_analysis(big, &spec_b2), Err(StatsError::InvalidParameter { .. })));
    }

    // FisherExact >6x6 within Monte Carlo cap (<= 50,000) labels explicitly as Monte Carlo with footnote.
    let big_7x7_row = [2_u64; 7];
    let big_7x7_rows = [&big_7x7_row[..]; 7];
    let s7 = contingency_sheet(&big_7x7_rows);
    let spec_f_mc = AnalysisSpec::new(
        SheetId::new("r-f-mc"),
        s7.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FisherExact, ..Default::default() }),
    )
    .unwrap();
    let res_f_mc = run_analysis(&s7, &spec_f_mc).unwrap();
    assert_eq!(res_f_mc.tabs[0].row_labels[0], "Fisher's exact test (Monte Carlo)");
    assert!(res_f_mc.tabs[0].footnotes.iter().any(|f| f.contains("Monte Carlo")));
    assert_eq!(scalar_by_name(&res_f_mc, "Fisher's exact Monte Carlo draws"), Some(10_000.0));
    assert!(scalar_by_name(&res_f_mc, "Fisher's exact P").is_some());

    // FisherExact exceeding Monte Carlo cap (> 50,000) returns structured error, never chi_square_p.
    let huge_3x3 = &[vec![10_000_u64, 10_000, 10_000], vec![10_000, 10_000, 10_000], vec![1_000, 1_000, 1_000]];
    assert!(matches!(fisher_exact_rxc(huge_3x3), Err(StatsError::InvalidParameter { .. })));

    let huge_3x3_sheet = contingency_sheet(&[&[10_000, 10_000, 10_000], &[10_000, 10_000, 10_000], &[1_000, 1_000, 1_000]]);
    let spec_f_err = AnalysisSpec::new(
        SheetId::new("r-f-err"),
        huge_3x3_sheet.id.clone(),
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::FisherExact, ..Default::default() }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&huge_3x3_sheet, &spec_f_err), Err(StatsError::InvalidParameter { .. })));
}

#[test]
fn r1_s02_03_roc_perfect_separation_valid_p_and_ci() {
    let ctrl = [1.0, 2.0, 3.0, 4.0, 5.0];
    let case = [10.0, 20.0, 30.0, 40.0, 50.0];
    let expected_mw_p = 2.0 / 252.0; // exact two-sided Mann-Whitney for n0=5, n1=5 separated

    for method in [RocCiMethod::DeLong, RocCiMethod::HanleyMcNeil] {
        let roc = roc_curve_analysis(&ctrl, &case, method, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!((roc.auc - 1.0).abs() < 1e-12);
        assert!(roc.p_value_vs_05 < 0.05 && (roc.p_value_vs_05 - expected_mw_p).abs() < 1e-12, "p={}", roc.p_value_vs_05);
        assert!(roc.ci.0 > 0.3 && roc.ci.0 < 1.0 && roc.ci.1 <= 1.0, "non-collapsed CI expected, got {:?}", roc.ci);

        // Reverse complete separation (AUC == 0.0).
        let rev = roc_curve_analysis(&case, &ctrl, method, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(rev.auc.abs() < 1e-12);
        assert!((rev.p_value_vs_05 - expected_mw_p).abs() < 1e-12);
        assert!(rev.ci.0 >= 0.0 && rev.ci.1 > 0.0 && rev.ci.1 < 0.7, "non-collapsed CI expected, got {:?}", rev.ci);
    }

    // All identical observations (AUC == 0.5, se == 0.0): P == 1.0, non-collapsed CI.
    let tied = roc_curve_analysis(&[5.0; 5], &[5.0; 5], RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!((tied.auc - 0.5).abs() < 1e-12);
    assert!((tied.p_value_vs_05 - 1.0).abs() < 1e-12);
    assert!(tied.ci.0 < 0.5 && tied.ci.1 > 0.5);

    // run_analysis smoke for separated ROC.
    let roc_sheet = column_sheet(&[("Ctrl", &ctrl), ("Case", &case)]);
    let spec = AnalysisSpec::new(SheetId::new("r-roc"), roc_sheet.id.clone(), AnalysisKind::RocCurve(RocSpec::default())).unwrap();
    let res = run_analysis(&roc_sheet, &spec).unwrap();
    assert!(scalar_by_name(&res, "P value (vs. 0.5)").unwrap() < 0.05);
}

#[test]
fn r1_s02_04_multiplicity_exhaustive_and_dunn_fwer() {
    let raw = [0.01, 0.02, 0.04];

    // Only None and FisherLsd leave raw P unadjusted.
    for unadj_proc in [MultiplicityProcedure::None, MultiplicityProcedure::FisherLsd] {
        let (adj, is_fdr) = adjust_p_values(&raw, unadj_proc, 0.05);
        assert!(!is_fdr);
        assert_eq!(adj, raw);
    }

    // Raw-P FWER methods apply Bonferroni / Sidak instead of raw pass-through.
    for bonf_proc in [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::DunnRankSum,
    ] {
        let (adj, is_fdr) = adjust_p_values(&raw, bonf_proc, 0.05);
        assert!(!is_fdr);
        for (a, r) in adj.iter().zip(raw.iter()) {
            assert!((*a - (r * 3.0).min(1.0)).abs() < 1e-12, "proc={bonf_proc:?} got {a} vs raw {r}");
        }
    }
    let (sidak_t2, is_fdr_t2) = adjust_p_values(&raw, MultiplicityProcedure::TamhaneT2, 0.05);
    assert!(!is_fdr_t2);
    for (a, r) in sidak_t2.iter().zip(raw.iter()) {
        let exp = 1.0 - (1.0 - r).powi(3);
        assert!((*a - exp).abs() < 1e-12);
    }

    // Parametric ANOVA rejects DunnRankSum.
    let groups = [
        GroupComparisonInput { index: 0, label: "A".into(), n: 5.0, mean: 1.0, sd: 1.0 },
        GroupComparisonInput { index: 1, label: "B".into(), n: 5.0, mean: 2.0, sd: 1.0 },
        GroupComparisonInput { index: 2, label: "C".into(), n: 5.0, mean: 4.0, sd: 1.0 },
    ];
    assert!(matches!(
        compare_groups_with_procedure(&groups, 1.0, 12.0, MultiplicityProcedure::DunnRankSum, 0.05, true),
        Err(StatsError::InvalidParameter { .. })
    ));

    // Dunn after Kruskal-Wallis with TukeyHsd must apply Dunn-Bonferroni!
    let labels = [(0, "G1".into()), (1, "G2".into()), (2, "G3".into())];
    let ns = [5, 5, 5];
    let mean_ranks = [3.0, 8.0, 13.0];
    let tukey_spec = MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false };
    let dunn_tukey = compare_dunn_ranks(&labels, &ns, &mean_ranks, 15, 0.0, &tukey_spec).unwrap();
    assert_eq!(dunn_tukey.len(), 3);
    let sig_pairs: Vec<_> = dunn_tukey.iter().filter(|d| d.raw_p < 0.99 && d.raw_p > 0.0).collect();
    assert!(!sig_pairs.is_empty());
    for d in &sig_pairs {
        assert!(d.adjusted_p_or_q > d.raw_p, "expected Dunn-Bonferroni adjustment for TukeyHsd: {:?}", d);
    }

    // Dunn with TamhaneT2 uses Sidak.
    let dunn_t2 = compare_dunn_ranks(
        &labels,
        &ns,
        &mean_ranks,
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::TamhaneT2, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    for d in &dunn_t2 {
        let exp = 1.0 - (1.0 - d.raw_p).powi(3);
        assert!((d.adjusted_p_or_q - exp).abs() < 1e-12);
    }

    // Dunn with Dunnett validates control_column and compares k - 1 pairs vs control with Bonferroni.
    let dunn_dunnett = compare_dunn_ranks(
        &labels,
        &ns,
        &mean_ranks,
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert_eq!(dunn_dunnett.len(), 2);
    for d in &dunn_dunnett {
        assert_eq!(d.group_b, 0);
        assert!((d.adjusted_p_or_q - (d.raw_p * 2.0).min(1.0)).abs() < 1e-12);
    }
    assert!(matches!(
        compare_dunn_ranks(
            &labels,
            &ns,
            &mean_ranks,
            15,
            0.0,
            &MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 3 }, threshold: 0.05, report_confidence_intervals: false },
        ),
        Err(StatsError::InvalidParameter { .. })
    ));

    // Only None and FisherLsd remain uncorrected when explicitly requested.
    for m in [MultiplicityMethod::None, MultiplicityMethod::FisherLsd] {
        let dunn_un = compare_dunn_ranks(
            &labels,
            &ns,
            &mean_ranks,
            15,
            0.0,
            &MultiplicitySpec { method: m, threshold: 0.05, report_confidence_intervals: false },
        )
        .unwrap();
        for d in &dunn_un {
            assert!((d.adjusted_p_or_q - d.raw_p).abs() < 1e-12);
        }
    }

    // anova.rs Kruskal-Wallis tab title/footnote documents Bonferroni fallback.
    let kw_sheet = column_sheet(&[("G1", &[1.0, 2.0, 3.0, 4.0, 5.0]), ("G2", &[6.0, 7.0, 8.0, 9.0, 10.0]), ("G3", &[20.0, 21.0, 22.0, 23.0, 24.0])]);
    let kw_spec = AnalysisSpec::new(
        SheetId::new("r-kw"),
        kw_sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::KruskalWallis,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
            compute_effect_sizes: true,
        }),
    )
    .unwrap();
    let kw_res = run_analysis(&kw_sheet, &kw_spec).unwrap();
    let mc_tab = kw_res.tabs.iter().find(|t| t.id == "multiple-comparisons").unwrap();
    assert!(mc_tab.title.contains("Bonferroni"));
    assert!(mc_tab.footnotes.iter().any(|f| f.contains("Bonferroni")));
}

#[test]
fn r1_s02_05_cox_monotone_likelihood_and_collinear_non_estimable() {
    // Monotone likelihood separation: all early events have high x, later events/censored have lower x.
    let subjects = vec![
        CoxSubject { time: 1.0, event: true, x: vec![6.0] },
        CoxSubject { time: 2.0, event: true, x: vec![5.0] },
        CoxSubject { time: 3.0, event: true, x: vec![4.0] },
        CoxSubject { time: 4.0, event: false, x: vec![3.0] },
        CoxSubject { time: 5.0, event: false, x: vec![2.0] },
        CoxSubject { time: 6.0, event: false, x: vec![1.0] },
    ];
    let res = cox_ph_regression(&subjects, &["X".into()], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!res.converged, "monotone separation must report converged=false");
    let cov = &res.covariates[0];
    assert!(cov.se.is_nan(), "expected NaN SE instead of {}, got {}", "regularized/identity", cov.se);
    assert!(cov.z_statistic.is_nan());
    assert!(cov.p_value.is_nan());
    assert!(cov.hr_ci.0.is_nan() && cov.hr_ci.1.is_nan());
    assert!(res.likelihood_ratio_p.is_finite());

    // Exact collinearity (x2 = 2 * x1) also yields non-estimable SE/CI without regularized/identity fallback.
    let col_subjects = vec![
        CoxSubject { time: 1.0, event: true, x: vec![1.0, 2.0] },
        CoxSubject { time: 2.0, event: false, x: vec![2.0, 4.0] },
        CoxSubject { time: 3.0, event: true, x: vec![4.0, 8.0] },
        CoxSubject { time: 4.0, event: true, x: vec![1.5, 3.0] },
        CoxSubject { time: 5.0, event: false, x: vec![3.0, 6.0] },
        CoxSubject { time: 6.0, event: true, x: vec![0.5, 1.0] },
    ];
    let col_res = cox_ph_regression(&col_subjects, &["X1".into(), "X2".into()], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!col_res.converged);
    for c in &col_res.covariates {
        assert!(c.se.is_nan());
        assert!(c.hr_ci.0.is_nan() && c.hr_ci.1.is_nan());
    }

    // run_analysis result sheet surfaces Converged/Iterations, Empty/Open cells, and Warning checklist.
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Marker");
    for (r, s) in subjects.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(s.time);
        mv.variables[1].cells[r] = Cell::number(if s.event { 1.0 } else { 0.0 });
        mv.variables[2].cells[r] = Cell::number(s.x[0]);
    }
    let cox_sheet = DataSheet::new(SheetId::new("mv-sep-cox"), "Separated Cox", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-cox-sep"),
        cox_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Breslow,
        }),
    )
    .unwrap();
    let sheet_res = run_analysis(&cox_sheet, &spec).unwrap();
    assert_eq!(scalar_by_name(&sheet_res, "Converged"), Some(0.0));
    assert!(scalar_by_name(&sheet_res, "Iterations").is_some_and(|v| v >= 1.0));
    let cox_tab = sheet_res.tabs.iter().find(|t| t.id == "cox-ph").unwrap();
    assert!(matches!(cox_tab.rows[0][1], ResultCell::Empty)); // SE
    assert!(matches!(cox_tab.rows[0][2], ResultCell::Empty)); // Wald z
    assert!(matches!(cox_tab.rows[0][3], ResultCell::Empty)); // P value
    assert!(matches!(cox_tab.rows[0][5], ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open, .. }));
    let conv_warn = sheet_res.checklist.iter().find(|c| c.assumption == "Cox PH convergence & estimability").unwrap();
    assert_eq!(conv_warn.status, ChecklistStatus::Warn);
    assert!(conv_warn.remediation.is_none());
    let lower_expl = conv_warn.explanation.to_ascii_lowercase();
    assert!(!lower_expl.contains("is recommended") && !lower_expl.contains("use "));
}

#[test]
fn r1_s02_06_one_tailed_rank_tests_respect_both_directions() {
    // Exact Mann-Whitney (n1=5, n2=5 -> exact 1/252 ~= 0.003968 in both directions).
    let low = [1.0, 2.0, 3.0, 4.0, 5.0];
    let high = [10.0, 20.0, 30.0, 40.0, 50.0];
    let mw_lo = mann_whitney_u(&low, &high, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let mw_hi = mann_whitney_u(&high, &low, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(mw_lo.p_value < 0.01 && (mw_lo.p_value - mw_hi.p_value).abs() < 1e-12);

    // Asymptotic normal-approx Mann-Whitney (n1=15, n2=15 -> total=30 > 20).
    let low_big: Vec<f64> = (1..=15).map(|i| i as f64).collect();
    let high_big: Vec<f64> = (101..=115).map(|i| i as f64).collect();
    let mw_big_lo = mann_whitney_u(&low_big, &high_big, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let mw_big_hi = mann_whitney_u(&high_big, &low_big, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(mw_big_lo.p_value < 1e-4 && (mw_big_lo.p_value - mw_big_hi.p_value).abs() < 1e-12);

    // Exact paired Wilcoxon (untied diffs, n=6 -> 1/64 = 0.015625 in both directions).
    let p_lo = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let p_hi = [10.0, 12.0, 15.0, 19.0, 24.0, 30.0];
    let wp_lo = wilcoxon_matched_pairs(&p_lo, &p_hi, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    let wp_hi = wilcoxon_matched_pairs(&p_hi, &p_lo, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    assert!((wp_lo.p_value - 1.0 / 64.0).abs() < 1e-12);
    assert!((wp_hi.p_value - 1.0 / 64.0).abs() < 1e-12);

    // Tied paired Wilcoxon (n=6, all diffs equal -10 -> tie-corrected normal approx) in both directions.
    let p_tie_hi = [11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
    let wp_tie_lo = wilcoxon_matched_pairs(&p_lo, &p_tie_hi, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    let wp_tie_hi = wilcoxon_matched_pairs(&p_tie_hi, &p_lo, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    assert!(wp_tie_lo.p_value < 0.05 && (wp_tie_lo.p_value - wp_tie_hi.p_value).abs() < 1e-12);

    // Asymptotic normal-approx paired Wilcoxon (n=25 > 20).
    let a25: Vec<f64> = (1..=25).map(|i| i as f64).collect();
    let b25: Vec<f64> = (101..=125).map(|i| i as f64).collect();
    let wp_big_lo = wilcoxon_matched_pairs(&a25, &b25, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    let wp_big_hi = wilcoxon_matched_pairs(&b25, &a25, false, ConfidenceLevel::DEFAULT_95, false).unwrap();
    assert!(wp_big_lo.p_value < 1e-4 && (wp_big_lo.p_value - wp_big_hi.p_value).abs() < 1e-12);

    // Exact one-sample Wilcoxon (n=6) in both directions around hypothetical median.
    let os_lo = one_sample_wilcoxon(&p_lo, 50.0, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let os_hi = one_sample_wilcoxon(&p_hi, 0.0, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!((os_lo.p_value - 1.0 / 64.0).abs() < 1e-12);
    assert!((os_hi.p_value - 1.0 / 64.0).abs() < 1e-12);

    // Asymptotic normal-approx one-sample Wilcoxon (n=25 > 20).
    let os_big_lo = one_sample_wilcoxon(&a25, 100.0, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let os_big_hi = one_sample_wilcoxon(&b25, 0.0, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(os_big_lo.p_value < 1e-4 && (os_big_lo.p_value - os_big_hi.p_value).abs() < 1e-12);
}
