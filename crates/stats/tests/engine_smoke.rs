//! Comprehensive integration & smoke tests covering every statistical family in `curvebook-stats`,
//! end-to-end `run_analysis` `ResultSheet` builders, multiplicity procedures, and `ChecklistItem` diagnostics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable, CoxTieMethod,
    DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod, OutlierSpec, RepeatedMeasuresDesign, RocCiMethod, RocSpec,
    SheetId, SubcolumnMode, SumOfSquaresType, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype,
    TwoGroupMethod, TwoGroupSpec,
};
use curvebook_math::{ConfidenceLevel, DenseMatrix};
use curvebook_stats::anova::{
    FactorialObservations, fit_factorial_anova, fit_random_intercept_reml, fit_rm_two_way_anova, friedman_test, kruskal_wallis_test,
    nested_one_way_anova, rm_one_way_anova, unequal_variance_one_way_anova,
};
use curvebook_stats::categorical::{
    KappaWeighting, baptista_pike_or_ci, cochran_armitage_trend, cohens_kappa, fleiss_kappa, mcnemar_test, wilson_score_ci,
};
use curvebook_stats::column::{
    anderson_darling, dagostino_pearson_k2, exact_binomial_median_ci, grubbs_outliers, lilliefors_ks, rout_outliers, shapiro_wilk,
};
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_p_values, compare_dunn_ranks, compare_groups_with_procedure,
};
use curvebook_stats::multivariate::{LinkageMethod, hierarchical_clustering, kmeans_pp};
use curvebook_stats::regression::{multiple_logistic_regression, multiple_poisson_regression};
use curvebook_stats::roc::{compare_roc_delong_paired, compare_roc_unpaired, roc_curve_analysis};
use curvebook_stats::run_analysis;
use curvebook_stats::survival::{CoxSubject, compare_survival_curves, cox_ph_regression, kaplan_meier_curve};
use curvebook_stats::two_group::{bland_altman, lognormal_unpaired_t_test, ratio_paired_t_test, welch_t_test, wilcoxon_matched_pairs};
use curvebook_testkit::assert_approx_eq;

fn col_sheet(id: &str, cols: &[(&str, &[f64])], hint: ColumnDesignHint) -> DataSheet {
    let max_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1);
    let mut table = ColumnTable::new(hint, cols.len(), max_rows).unwrap();
    for (c, &(name, vals)) in cols.iter().enumerate() {
        table.data_sets[c].title = name.to_string();
        for (r, &v) in vals.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell { value: CellValue::Number(v), excluded: false };
        }
    }
    DataSheet::new(SheetId::new(id), "Test Sheet", TableArchetype::Column(table)).unwrap()
}

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

#[test]
fn column_descriptive_normality_outliers_and_median_ci() {
    let normal_like = [10.2, 9.8, 10.5, 10.1, 9.9, 10.4, 9.7, 10.0, 10.3, 9.6, 10.6, 10.1];
    let med_ci = exact_binomial_median_ci(&normal_like, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(med_ci.lower <= med_ci.upper);
    assert!(med_ci.actual_confidence_percent >= 95.0);

    let sw = shapiro_wilk(&normal_like, 0.05).unwrap();
    let ad = anderson_darling(&normal_like, 0.05).unwrap();
    let ks = lilliefors_ks(&normal_like, 0.05).unwrap();
    let dp = dagostino_pearson_k2(&normal_like, 0.05).unwrap();
    for t in [&sw, &ad, &ks, &dp] {
        assert!((0.0..=1.0).contains(&t.p_value), "{} p out of bounds", t.test_name);
        assert!(t.passed, "{} unexpectedly failed on normal-like sample", t.test_name);
    }

    // Outliers: Grubbs + standalone ROUT (K=1)
    let with_outlier = [10.0, 10.2, 9.9, 10.1, 10.3, 9.8, 10.0, 10.1, 9.9, 45.0];
    let gr = grubbs_outliers(&with_outlier, 0.05, true).unwrap();
    assert_eq!(gr.outliers.len(), 1);
    assert_approx_eq(gr.outliers[0].value, 45.0, 1e-12, 1e-12, "Grubbs catches 45.0");

    let rt = rout_outliers(&with_outlier, 1.0).unwrap();
    assert_eq!(rt.outliers.len(), 1);
    assert_approx_eq(rt.outliers[0].value, 45.0, 1e-12, 1e-12, "ROUT catches 45.0");

    let sheet = col_sheet("s-col", &[("Ctrl", &with_outlier), ("Trt", &normal_like)], ColumnDesignHint::UnpairedReplicates);
    let desc_res = run_analysis(&sheet, &spec_for("r-desc", "s-col", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))).unwrap();
    assert!(!desc_res.tabs.is_empty());
    assert!(!desc_res.checklist.is_empty());

    let rout_res =
        run_analysis(&sheet, &spec_for("r-rout", "s-col", AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }))).unwrap();
    assert!(!rout_res.tabs.is_empty());
    assert!(!rout_res.checklist.is_empty());
}

#[test]
fn two_group_and_bland_altman_suite() {
    let a = [12.0, 14.0, 13.5, 15.0, 14.5, 13.0];
    let b = [10.0, 11.0, 10.5, 11.5, 10.2, 9.8];

    let w = welch_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(w.p_value < 0.01);
    assert!(w.effect_sizes.hedges_g_ci.is_some());

    let rp = ratio_paired_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(rp.geometric_mean_ratio.unwrap() > 1.1);

    let lu = lognormal_unpaired_t_test(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(lu.geometric_mean_ratio.unwrap() > 1.1);

    let a_notie = [12.1, 14.3, 13.8, 15.4, 14.9, 13.4];
    let b_notie = [10.0, 11.0, 10.7, 11.5, 10.2, 9.8];
    let wil = wilcoxon_matched_pairs(&a_notie, &b_notie, true, ConfidenceLevel::DEFAULT_95, false).unwrap();
    assert!(wil.exact_p_used);
    assert!(wil.p_value < 0.05);

    let ba = bland_altman(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(ba.bias > 2.0);
    assert!(ba.loa_lower < ba.bias && ba.bias < ba.loa_upper);

    let sheet = col_sheet("s-2g", &[("A", &a), ("B", &b)], ColumnDesignHint::UnpairedReplicates);
    for method in [
        TwoGroupMethod::UnpairedT,
        TwoGroupMethod::WelchT,
        TwoGroupMethod::PairedT,
        TwoGroupMethod::RatioPairedT,
        TwoGroupMethod::LognormalUnpairedT,
        TwoGroupMethod::MannWhitneyU,
        TwoGroupMethod::WilcoxonSignedRank,
        TwoGroupMethod::KolmogorovSmirnov,
        TwoGroupMethod::OneSampleT { hypothetical: 12.0 },
        TwoGroupMethod::OneSampleWilcoxon { hypothetical: 12.0 },
    ] {
        let sp = TwoGroupSpec { method, two_tailed: true, compute_effect_sizes: true, mv_group_column: None, mv_response_column: None };
        let res = run_analysis(&sheet, &spec_for("r-2g", "s-2g", AnalysisKind::TwoGroupComparison(sp))).unwrap();
        assert!(!res.tabs.is_empty());
        assert!(!res.checklist.is_empty());
    }
}

#[test]
fn all_14_multiplicity_procedures_and_anova_family() {
    let raw = [0.001, 0.012, 0.039, 0.045, 0.21, 0.85];
    let non_adaptive = [
        MultiplicityProcedure::None,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
    ];
    for p in non_adaptive {
        let (adj, _) = adjust_p_values(&raw, p, 0.05);
        assert_eq!(adj.len(), raw.len());
        assert!(raw.iter().zip(adj.iter()).all(|(&r, &a)| a >= r - 1e-12 && a <= 1.0));
    }
    // BKY (2006) Def. 6 scales Stage-2 decision values by m0_hat * (1 + q) / i, which can fall below raw_p.
    let (bky_adj, bky_is_fdr) = adjust_p_values(&raw, MultiplicityProcedure::TwoStageBkyFdr, 0.05);
    assert!(bky_is_fdr && bky_adj.len() == raw.len() && bky_adj.iter().all(|&a| (0.0..=1.0).contains(&a)));
    assert!(bky_adj.iter().take(4).all(|&a| a <= 0.05));

    let groups = vec![
        GroupComparisonInput { index: 0, label: "Ctrl".into(), n: 6.0, mean: 10.0, sd: 1.1 },
        GroupComparisonInput { index: 1, label: "Low".into(), n: 6.0, mean: 12.5, sd: 1.2 },
        GroupComparisonInput { index: 2, label: "High".into(), n: 6.0, mean: 16.0, sd: 1.3 },
    ];
    let all_pairwise_procs = [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::TamhaneT2,
        MultiplicityProcedure::FisherLsd,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
        MultiplicityProcedure::TwoStageBkyFdr,
    ];
    for proc in all_pairwise_procs {
        let pw = compare_groups_with_procedure(&groups, 1.5, 15.0, proc, 0.05, true).unwrap();
        assert!(!pw.is_empty());
    }

    let g1 = [10.0, 11.0, 9.5, 10.5, 10.2];
    let g2 = [13.0, 12.5, 14.0, 13.5, 12.8];
    let g3 = [17.0, 16.5, 18.0, 17.2, 16.8];
    let dunn = compare_dunn_ranks(
        &[(0, "G1".into()), (1, "G2".into()), (2, "G3".into())],
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::BenjaminiHochbergFdr, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert_eq!(dunn.len(), 3);

    let wb = unequal_variance_one_way_anova(&groups).unwrap();
    assert!(wb.welch_p < 0.001);
    let kw = kruskal_wallis_test(&[&g1, &g2, &g3]).unwrap();
    assert!(kw.p_value < 0.01);

    let blocks = vec![vec![10.0, 12.0, 16.0], vec![11.0, 13.0, 17.0], vec![9.5, 11.5, 15.5], vec![10.5, 12.5, 16.5], vec![10.2, 12.2, 16.2]];
    let rm1 = rm_one_way_anova(&blocks).unwrap();
    assert!(rm1.p_greenhouse_geisser < 0.001);
    let fr = friedman_test(&blocks).unwrap();
    assert!(fr.p_value < 0.01);

    let fact = FactorialObservations {
        factor_names: vec!["Drug".into(), "Dose".into()],
        factor_levels: vec![vec!["Placebo".into(), "Active".into()], vec!["Low".into(), "High".into()]],
        rows: vec![
            (vec![0, 0], 10.0),
            (vec![0, 0], 10.5),
            (vec![0, 0], 9.8),
            (vec![0, 1], 11.0),
            (vec![0, 1], 11.2),
            (vec![1, 0], 14.0),
            (vec![1, 0], 14.5),
            (vec![1, 0], 13.8),
            (vec![1, 1], 19.0),
            (vec![1, 1], 19.4),
            (vec![1, 1], 18.8),
        ],
    };
    let f_res = fit_factorial_anova(&fact, 2, SumOfSquaresType::TypeIII).unwrap();
    assert_eq!(f_res.terms.len(), 3);
    assert!(f_res.terms[0].p_value.unwrap() < 0.01);

    // Mixed-effects REML with Satterthwaite & Kenward-Roger dfs
    let x_reml = DenseMatrix::from_row_major(
        10,
        3,
        vec![
            1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
        ],
    )
    .unwrap();
    let y_reml = [10.0, 12.0, 15.0, 10.5, 15.8, 9.8, 11.9, 14.9, 10.2, 12.4];
    let cids = [0, 0, 0, 1, 1, 2, 2, 2, 3, 3];
    let reml = fit_random_intercept_reml(&x_reml, &y_reml, &cids, &[("Condition", 1, 2)]).unwrap();
    assert_eq!(reml.fixed_terms.len(), 1);
    assert!(reml.fixed_terms[0].p_value.unwrap() < 0.01);
    assert!(reml.satterthwaite_df[0] > 0.0);
    assert!(reml.kenward_roger_df[0] > 0.0);

    // 2-way RM ANOVA (RowFactorMatched)
    let rm2_subj = vec![
        vec![vec![10.0, 14.0], vec![11.0, 16.0], vec![12.0, 19.0]],
        vec![vec![10.2, 14.3], vec![11.3, 16.2], vec![12.1, 19.4]],
        vec![vec![9.8, 13.9], vec![10.9, 15.8], vec![11.9, 18.8]],
    ];
    let (rm2_fac, rm2_sph) = fit_rm_two_way_anova(&rm2_subj, "Time", "Group", RepeatedMeasuresDesign::BothFactorsMatched).unwrap();
    assert_eq!(rm2_fac.terms.len(), 3);
    assert!(rm2_sph.is_some());

    // Nested ANOVA & Nested t via run_analysis + direct helper
    let mut nt = NestedTable::new(2, 3, 2).unwrap();
    for g in 0..2 {
        for b in 0..3 {
            for r in 0..2 {
                let v = (g as f64) * 5.0 + (b as f64) * 0.5 + (r as f64) * 0.1 + 10.0;
                nt.groups[g].cells[r][b] = Cell { value: CellValue::Number(v), excluded: false };
            }
        }
    }
    let nested_direct = nested_one_way_anova(&nt, true).unwrap();
    assert!(nested_direct.p_among_groups < 0.01);

    let n_sheet = DataSheet::new(SheetId::new("s-nest"), "Nested", TableArchetype::Nested(nt)).unwrap();
    let nest_res = run_analysis(
        &n_sheet,
        &spec_for("r-nest", "s-nest", AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, ..NestedAnalysisSpec::default() })),
    )
    .unwrap();
    assert!(!nest_res.tabs.is_empty());

    let csheet = col_sheet("s-aov", &[("G1", &g1), ("G2", &g2), ("G3", &g3)], ColumnDesignHint::UnpairedReplicates);
    let aov_sheet = run_analysis(
        &csheet,
        &spec_for(
            "r-aov",
            "s-aov",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::OrdinaryAnova,
                compute_effect_sizes: true,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
                ..OneWayAnovaSpec::default()
            }),
        ),
    )
    .unwrap();
    assert!(!aov_sheet.pairwise_comparisons.is_empty());

    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    for c in 0..2 {
        for r in 0..3 {
            let base = 10.0 + (c as f64) * 3.0 + (r as f64) * 2.0;
            gt.data_sets[c].cells[r][0] = Cell { value: CellValue::Number(base), excluded: false };
            gt.data_sets[c].cells[r][1] = Cell { value: CellValue::Number(base + 0.3), excluded: false };
        }
    }
    let gsheet = DataSheet::new(SheetId::new("s-grp"), "Grouped", TableArchetype::Grouped(gt)).unwrap();
    let f_sheet = run_analysis(&gsheet, &spec_for("r-fact", "s-grp", AnalysisKind::FactorialAnova(FactorialAnovaSpec::default()))).unwrap();
    assert!(!f_sheet.tabs.is_empty());
}

#[test]
fn categorical_survival_roc_glm_and_multivariate() {
    let wci = wilson_score_ci(18, 25, ConfidenceLevel::DEFAULT_95);
    assert!(wci.0 < 0.72 && 0.72 < wci.1);
    let bp = baptista_pike_or_ci(18, 7, 6, 19, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(bp.0 < bp.1);
    let (ca_chi2, ca_p) = cochran_armitage_trend(&[vec![5, 15], vec![10, 10], vec![16, 4]]).unwrap();
    assert!(ca_chi2 > 0.0 && ca_p < 0.01);
    let mc = mcnemar_test(2, 14).unwrap();
    assert!(mc.1 < 0.01);
    let ck = cohens_kappa(&[vec![20, 2], vec![3, 25]], KappaWeighting::Unweighted, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(ck.kappa > 0.7);
    let fk = fleiss_kappa(&[vec![3, 0], vec![0, 3], vec![2, 1], vec![0, 3]], ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(fk.kappa > 0.3);

    let mut ct = ContingencyTable::new(2, 2).unwrap();
    let vals = [[20.0, 5.0], [4.0, 21.0]];
    for (r, row) in vals.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell { value: CellValue::Number(v), excluded: false };
        }
    }
    let csheet = DataSheet::new(SheetId::new("s-cat"), "Cont", TableArchetype::Contingency(ct)).unwrap();
    let cat_res = run_analysis(
        &csheet,
        &spec_for(
            "r-cat",
            "s-cat",
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::FisherExact,
                compute_odds_and_risk_ratios: true,
                compute_sensitivity_specificity: true,
                compute_phi_and_cramers_v: true,
                expected_proportions: None,
                null_proportion: None,
            }),
        ),
    )
    .unwrap();
    assert!(!cat_res.tabs.is_empty());

    // Survival KM + Cox PH
    let mut st = SurvivalTable::new(2, 6).unwrap();
    let times = [2.0, 4.0, 6.0, 8.0, 10.0, 12.0];
    let ev0 = [1.0, 1.0, 1.0, 0.0, 1.0, 0.0];
    let ev1 = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    for r in 0..6 {
        st.times[r] = Cell { value: CellValue::Number(times[r]), excluded: false };
        st.cohorts[0].cells[r][0] = Cell { value: CellValue::Number(ev0[r]), excluded: false };
        st.cohorts[1].cells[r][0] = Cell { value: CellValue::Number(ev1[r]), excluded: false };
    }
    let ssheet = DataSheet::new(SheetId::new("s-surv"), "Surv", TableArchetype::Survival(st)).unwrap();
    let km_res = run_analysis(
        &ssheet,
        &spec_for(
            "r-km",
            "s-surv",
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: true,
                trend_test: true,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        ),
    )
    .unwrap();
    assert!(!km_res.fitted_curves.is_empty());

    let obs_a = [
        SurvivalObservation { row: 0, time: 2.0, status: SurvivalStatus::Event },
        SurvivalObservation { row: 1, time: 4.0, status: SurvivalStatus::Event },
        SurvivalObservation { row: 2, time: 6.0, status: SurvivalStatus::Censored },
        SurvivalObservation { row: 3, time: 8.0, status: SurvivalStatus::Event },
    ];
    let obs_b = [
        SurvivalObservation { row: 0, time: 5.0, status: SurvivalStatus::Event },
        SurvivalObservation { row: 1, time: 8.0, status: SurvivalStatus::Censored },
        SurvivalObservation { row: 2, time: 10.0, status: SurvivalStatus::Event },
    ];
    let km = kaplan_meier_curve("Cohort A", &obs_a, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!km.steps.is_empty());
    let lr = compare_survival_curves(&[&obs_a, &obs_b], false).unwrap();
    assert!((0.0..=1.0).contains(&lr.p_value));

    let cox_subjects = [
        CoxSubject { time: 2.0, event: true, x: vec![1.0] },
        CoxSubject { time: 4.0, event: true, x: vec![1.2] },
        CoxSubject { time: 5.0, event: false, x: vec![0.2] },
        CoxSubject { time: 7.0, event: true, x: vec![0.8] },
        CoxSubject { time: 9.0, event: true, x: vec![-0.5] },
        CoxSubject { time: 11.0, event: false, x: vec![-1.0] },
        CoxSubject { time: 13.0, event: true, x: vec![-0.8] },
        CoxSubject { time: 15.0, event: false, x: vec![-1.5] },
    ];
    let cox = cox_ph_regression(&cox_subjects, &["Score".into()], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(cox.covariates.len(), 1);

    // ROC DeLong (paired & unpaired) + run_analysis
    let ctrl = [1.0, 2.0, 2.5, 3.0, 3.5, 4.0];
    let cases = [3.8, 4.5, 5.0, 6.0, 7.0, 8.0];
    let roc_res = roc_curve_analysis(&ctrl, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(roc_res.auc > 0.9);
    let roc_b =
        roc_curve_analysis(&[1.5, 2.8, 3.2, 4.0, 4.1, 4.2], &[3.0, 4.0, 4.5, 5.5, 6.0, 6.5], RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95)
            .unwrap();
    let cmp_paired = compare_roc_delong_paired(&roc_res, &roc_b).unwrap();
    assert!((0.0..=1.0).contains(&cmp_paired.p_value));
    let cmp_unpaired = compare_roc_unpaired(&roc_res, &roc_b);
    assert!((0.0..=1.0).contains(&cmp_unpaired.p_value));

    let roc_sheet = col_sheet("s-roc", &[("Ctrl", &ctrl), ("Cases", &cases)], ColumnDesignHint::UnpairedReplicates);
    let roc_sheet_out =
        run_analysis(&roc_sheet, &spec_for("r-roc", "s-roc", AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }))).unwrap();
    assert!(!roc_sheet_out.fitted_curves.is_empty());

    // Logistic + Poisson GLM + Multivariate (PCA, k-means++, hierarchical)
    let x_glm = DenseMatrix::from_row_major(8, 2, vec![1.0, -2.0, 1.0, -1.5, 1.0, -1.0, 1.0, -0.2, 1.0, 0.4, 1.0, 1.0, 1.0, 1.6, 1.0, 2.2]).unwrap();
    let logi = multiple_logistic_regression(
        &x_glm,
        &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        &["Intercept".into(), "Dose".into()],
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert!(logi.roc_auc.unwrap() > 0.9);

    let pois = multiple_poisson_regression(
        &x_glm,
        &[1.0, 2.0, 2.0, 4.0, 6.0, 9.0, 14.0, 22.0],
        &["Intercept".into(), "Dose".into()],
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert!(pois.terms[1].beta > 0.0);

    let mut mvt = MultipleVariablesTable::new(3, 8).unwrap();
    let mv_x = [-2.0, -1.5, -1.0, -0.2, 0.4, 1.0, 1.6, 2.2];
    let mv_z = [1.0, 0.5, 1.5, 0.8, 2.1, 1.9, 2.8, 3.1];
    let mv_y = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    for r in 0..8 {
        mvt.variables[0].cells[r] = Cell { value: CellValue::Number(mv_x[r]), excluded: false };
        mvt.variables[1].cells[r] = Cell { value: CellValue::Number(mv_z[r]), excluded: false };
        mvt.variables[2].cells[r] = Cell { value: CellValue::Number(mv_y[r]), excluded: false };
    }
    let msheet = DataSheet::new(SheetId::new("s-mv"), "MV", TableArchetype::MultipleVariables(mvt)).unwrap();
    let glm_out = run_analysis(
        &msheet,
        &spec_for(
            "r-glm",
            "s-mv",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 2,
                predictor_columns: vec![0, 1],
                reference_levels: Vec::new(),
                compute_vif: true,
            }),
        ),
    )
    .unwrap();
    assert!(!glm_out.tabs.is_empty());

    let pca_out = run_analysis(
        &msheet,
        &spec_for(
            "r-pca",
            "s-mv",
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42), pcr_outcome_column: None }),
        ),
    )
    .unwrap();
    assert!(!pca_out.tabs.is_empty());

    let km_cl = kmeans_pp(&[vec![0.0, 0.0], vec![0.2, 0.1], vec![5.0, 5.0], vec![5.2, 4.9]], 2, 123).unwrap();
    assert_eq!(km_cl.assignments.len(), 4);
    let hc = hierarchical_clustering(&[vec![0.0, 0.0], vec![0.2, 0.1], vec![5.0, 5.0], vec![5.2, 4.9]], LinkageMethod::WardD2).unwrap();
    assert_eq!(hc.len(), 3);
}
