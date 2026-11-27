//! Comprehensive integration & smoke tests covering every statistical family in `curvebook-stats`,
//! end-to-end `run_analysis` `ResultSheet` builders, multiplicity procedures, and `ChecklistItem` diagnostics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec,
    MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec, RepeatedMeasuresDesign,
    RocCiMethod, RocSpec, SheetId, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec,
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
use curvebook_stats::multiplicity::{MultiplicityProcedure, adjust_p_values, compare_dunn_ranks, compare_groups_with_procedure};
use curvebook_stats::multivariate::{LinkageMethod, hierarchical_clustering, kmeans_pp};
use curvebook_stats::regression::{multiple_logistic_regression, multiple_poisson_regression};
use curvebook_stats::roc::{compare_roc_delong_paired, compare_roc_unpaired, roc_curve_analysis};
use curvebook_stats::survival::{CoxSubject, compare_survival_curves, cox_ph_regression, kaplan_meier_curve};
use curvebook_stats::two_group::{bland_altman, lognormal_unpaired_t_test, ratio_paired_t_test, welch_t_test, wilcoxon_matched_pairs};
use curvebook_testkit::assert_approx_eq;

use crate::common::{col_sheet, contingency_sheet, group_input, grouped_sheet_fill, mv_num_sheet, nested_table, obs, run_kind, survival_sheet};

#[test]
fn column_descriptive_normality_outliers_and_median_ci() {
    let normal_like = [10.2, 9.8, 10.5, 10.1, 9.9, 10.4, 9.7, 10.0, 10.3, 9.6, 10.6, 10.1];
    let med_ci = exact_binomial_median_ci(&normal_like, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(med_ci.lower <= med_ci.upper && med_ci.actual_confidence_percent >= 95.0);

    for t in [
        shapiro_wilk(&normal_like, 0.05).unwrap(),
        anderson_darling(&normal_like, 0.05).unwrap(),
        lilliefors_ks(&normal_like, 0.05).unwrap(),
        dagostino_pearson_k2(&normal_like, 0.05).unwrap(),
    ] {
        assert!((0.0..=1.0).contains(&t.p_value) && t.passed, "{} failed on normal-like sample", t.test_name);
    }

    let with_outlier = [10.0, 10.2, 9.9, 10.1, 10.3, 9.8, 10.0, 10.1, 9.9, 45.0];
    let gr = grubbs_outliers(&with_outlier, 0.05, true).unwrap();
    assert_eq!(gr.outliers.len(), 1);
    assert_approx_eq(gr.outliers[0].value, 45.0, 1e-12, 1e-12, "Grubbs catches 45.0");

    let rt = rout_outliers(&with_outlier, 1.0).unwrap();
    assert_eq!(rt.outliers.len(), 1);
    assert_approx_eq(rt.outliers[0].value, 45.0, 1e-12, 1e-12, "ROUT catches 45.0");

    let sheet = col_sheet("s-col", &[("Ctrl", &with_outlier), ("Trt", &normal_like)]);
    for kind in [AnalysisKind::DescriptiveStats(DescriptiveSpec::default()), AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 })]
    {
        let res = run_kind(&sheet, kind).unwrap();
        assert!(!res.tabs.is_empty() && !res.checklist.is_empty());
    }
}

#[test]
fn two_group_and_bland_altman_suite() {
    let a = [12.0, 14.0, 13.5, 15.0, 14.5, 13.0];
    let b = [10.0, 11.0, 10.5, 11.5, 10.2, 9.8];
    let conf = ConfidenceLevel::DEFAULT_95;

    let w = welch_t_test(&a, &b, true, conf).unwrap();
    assert!(w.p_value < 0.01 && w.effect_sizes.hedges_g_ci.is_some());
    assert!(ratio_paired_t_test(&a, &b, true, conf).unwrap().geometric_mean_ratio.unwrap() > 1.1);
    assert!(lognormal_unpaired_t_test(&a, &b, true, conf).unwrap().geometric_mean_ratio.unwrap() > 1.1);

    let wil = wilcoxon_matched_pairs(&[12.1, 14.3, 13.8, 15.4, 14.9, 13.4], &[10.0, 11.0, 10.7, 11.5, 10.2, 9.8], true, conf, false).unwrap();
    assert!(wil.exact_p_used && wil.p_value < 0.05);

    let ba = bland_altman(&a, &b, false, conf).unwrap();
    assert!(ba.bias > 2.0 && ba.loa_lower < ba.bias && ba.bias < ba.loa_upper);

    let sheet = col_sheet("s-2g", &[("A", &a), ("B", &b)]);
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
        let res = run_kind(&sheet, AnalysisKind::TwoGroupComparison(sp)).unwrap();
        assert!(!res.tabs.is_empty() && !res.checklist.is_empty());
    }
}

#[test]
fn all_14_multiplicity_procedures_and_anova_family() {
    let raw = [0.001, 0.012, 0.039, 0.045, 0.21, 0.85];
    for p in [
        MultiplicityProcedure::None,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
    ] {
        let (adj, _) = adjust_p_values(&raw, p, 0.05);
        assert_eq!(adj.len(), raw.len());
        assert!(raw.iter().zip(adj.iter()).all(|(&r, &a)| a >= r - 1e-12 && a <= 1.0));
    }
    let (bky_adj, bky_is_fdr) = adjust_p_values(&raw, MultiplicityProcedure::TwoStageBkyFdr, 0.05);
    assert!(bky_is_fdr && bky_adj.len() == raw.len() && bky_adj.iter().all(|&a| (0.0..=1.0).contains(&a)));
    assert!(bky_adj.iter().take(4).all(|&a| a <= 0.05));

    let groups = vec![group_input(0, "Ctrl", 6.0, 10.0, 1.1), group_input(1, "Low", 6.0, 12.5, 1.2), group_input(2, "High", 6.0, 16.0, 1.3)];
    for proc in [
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
    ] {
        assert!(!compare_groups_with_procedure(&groups, 1.5, 15.0, proc, 0.05, true).unwrap().is_empty());
    }

    let (g1, g2, g3) = ([10.0, 11.0, 9.5, 10.5, 10.2], [13.0, 12.5, 14.0, 13.5, 12.8], [17.0, 16.5, 18.0, 17.2, 16.8]);
    let dunn = compare_dunn_ranks(
        &[(0, "G1".into()), (1, "G2".into()), (2, "G3".into())],
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec {
            method: MultiplicityMethod::BenjaminiHochbergFdr,
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
            selected_pairs: None,
        },
    )
    .unwrap();
    assert_eq!(dunn.len(), 3);
    assert!(unequal_variance_one_way_anova(&groups).unwrap().welch_p < 0.001);
    assert!(kruskal_wallis_test(&[&g1, &g2, &g3]).unwrap().p_value < 0.01);

    let blocks = vec![vec![10.0, 12.0, 16.0], vec![11.0, 13.0, 17.0], vec![9.5, 11.5, 15.5], vec![10.5, 12.5, 16.5], vec![10.2, 12.2, 16.2]];
    assert!(rm_one_way_anova(&blocks).unwrap().p_greenhouse_geisser < 0.001);
    assert!(friedman_test(&blocks).unwrap().p_value < 0.01);

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
    let f_res = fit_factorial_anova(&fact, 2, curvebook_doc::SumOfSquaresType::TypeIII).unwrap();
    assert_eq!(f_res.terms.len(), 3);
    assert!(f_res.terms[0].p_value.unwrap() < 0.01);

    let x_reml = DenseMatrix::from_row_major(
        10,
        3,
        vec![
            1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0,
            1.0, 1.0, 0.0,
        ],
    )
    .unwrap();
    let reml = fit_random_intercept_reml(
        &x_reml,
        &[10.0, 12.0, 15.0, 10.5, 15.8, 9.8, 11.9, 14.9, 10.2, 12.4],
        &[0, 0, 0, 1, 1, 2, 2, 2, 3, 3],
        &[("Condition", 1, 2)],
    )
    .unwrap();
    assert_eq!(reml.fixed_terms.len(), 1);
    assert!(reml.fixed_terms[0].p_value.unwrap() < 0.01 && reml.satterthwaite_df[0] > 0.0 && reml.kenward_roger_df[0] > 0.0);

    let rm2_subj = vec![
        vec![vec![10.0, 14.0], vec![11.0, 16.0], vec![12.0, 19.0]],
        vec![vec![10.2, 14.3], vec![11.3, 16.2], vec![12.1, 19.4]],
        vec![vec![9.8, 13.9], vec![10.9, 15.8], vec![11.9, 18.8]],
    ];
    let (rm2_fac, rm2_sph) = fit_rm_two_way_anova(&rm2_subj, "Time", "Group", RepeatedMeasuresDesign::BothFactorsMatched).unwrap();
    assert_eq!(rm2_fac.terms.len(), 3);
    assert!(rm2_sph.is_some());

    let nt = nested_table(&[("G0", &[&[10.0, 10.1], &[10.5, 10.6], &[11.0, 11.1]]), ("G1", &[&[15.0, 15.1], &[15.5, 15.6], &[16.0, 16.1]])]);
    assert!(nested_one_way_anova(&nt, true).unwrap().p_among_groups < 0.01);
    let n_sheet = DataSheet::new(SheetId::new("s-nest"), "Nested", TableArchetype::Nested(nt)).unwrap();
    assert!(
        !run_kind(&n_sheet, AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, ..Default::default() }))
            .unwrap()
            .tabs
            .is_empty()
    );

    let csheet = col_sheet("s-aov", &[("G1", &g1), ("G2", &g2), ("G3", &g3)]);
    let aov_sheet = run_kind(
        &csheet,
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::TukeyHsd,
                threshold: 0.05,
                report_confidence_intervals: true,
                control_column: None,
                selected_pairs: None,
            },
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(!aov_sheet.pairwise_comparisons.is_empty());

    let gsheet = grouped_sheet_fill("s-grp", 2, 3, 2, |c, r, s| Some(10.0 + (c as f64) * 3.0 + (r as f64) * 2.0 + (s as f64) * 0.3));
    assert!(!run_kind(&gsheet, AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap().tabs.is_empty());
}

#[test]
fn categorical_survival_roc_glm_and_multivariate() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let wci = wilson_score_ci(18, 25, conf);
    assert!(wci.0 < 0.72 && 0.72 < wci.1);
    let bp = baptista_pike_or_ci(18, 7, 6, 19, conf).unwrap();
    assert!(bp.0 < bp.1);
    let (ca_chi2, ca_p) = cochran_armitage_trend(&[vec![5, 15], vec![10, 10], vec![16, 4]]).unwrap();
    assert!(ca_chi2 > 0.0 && ca_p < 0.01);
    assert!(mcnemar_test(2, 14).unwrap().1 < 0.01);
    assert!(cohens_kappa(&[vec![20, 2], vec![3, 25]], KappaWeighting::Unweighted, conf).unwrap().kappa > 0.7);
    assert!(fleiss_kappa(&[vec![3, 0], vec![0, 3], vec![2, 1], vec![0, 3]], conf).unwrap().kappa > 0.3);

    let csheet = contingency_sheet("s-cat", &[&[20, 5], &[4, 21]]);
    let cat_res = run_kind(
        &csheet,
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            expected_proportions: None,
            null_proportion: None,
        }),
    )
    .unwrap();
    assert!(!cat_res.tabs.is_empty());

    let ssheet = survival_sheet(
        "s-surv",
        &[
            ("C0", &[(2.0, true), (4.0, true), (6.0, true), (8.0, false), (10.0, true), (12.0, false)]),
            ("C1", &[(2.0, false), (4.0, true), (6.0, false), (8.0, false), (10.0, true), (12.0, false)]),
        ],
    );
    let km_res = run_kind(
        &ssheet,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: true,
            trend_test: true,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        }),
    )
    .unwrap();
    assert!(!km_res.fitted_curves.is_empty());

    let obs_a = obs(&[(2.0, true), (4.0, true), (6.0, false), (8.0, true)]);
    let obs_b = obs(&[(5.0, true), (8.0, false), (10.0, true)]);
    assert!(!kaplan_meier_curve("Cohort A", &obs_a, true, conf).unwrap().steps.is_empty());
    assert!((0.0..=1.0).contains(&compare_survival_curves(&[&obs_a, &obs_b], false).unwrap().p_value));

    let cox_subjects: Vec<CoxSubject> = [
        (2.0, true, 1.0),
        (4.0, true, 1.2),
        (5.0, false, 0.2),
        (7.0, true, 0.8),
        (9.0, true, -0.5),
        (11.0, false, -1.0),
        (13.0, true, -0.8),
        (15.0, false, -1.5),
    ]
    .into_iter()
    .map(|(time, event, v)| CoxSubject { time, event, x: vec![v] })
    .collect();
    assert_eq!(cox_ph_regression(&cox_subjects, &["Score".into()], CoxTieMethod::Efron, conf).unwrap().covariates.len(), 1);

    let (ctrl, cases) = ([1.0, 2.0, 2.5, 3.0, 3.5, 4.0], [3.8, 4.5, 5.0, 6.0, 7.0, 8.0]);
    let roc_res = roc_curve_analysis(&ctrl, &cases, RocCiMethod::DeLong, conf).unwrap();
    assert!(roc_res.auc > 0.9);
    let roc_b = roc_curve_analysis(&[1.5, 2.8, 3.2, 4.0, 4.1, 4.2], &[3.0, 4.0, 4.5, 5.5, 6.0, 6.5], RocCiMethod::DeLong, conf).unwrap();
    assert!((0.0..=1.0).contains(&compare_roc_delong_paired(&roc_res, &roc_b).unwrap().p_value));
    assert!((0.0..=1.0).contains(&compare_roc_unpaired(&roc_res, &roc_b).p_value));
    assert!(
        !run_kind(&col_sheet("s-roc", &[("Ctrl", &ctrl), ("Cases", &cases)]), AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong }))
            .unwrap()
            .fitted_curves
            .is_empty()
    );

    let x_glm = DenseMatrix::from_row_major(8, 2, vec![1.0, -2.0, 1.0, -1.5, 1.0, -1.0, 1.0, -0.2, 1.0, 0.4, 1.0, 1.0, 1.0, 1.6, 1.0, 2.2]).unwrap();
    let terms = ["Intercept".into(), "Dose".into()];
    assert!(multiple_logistic_regression(&x_glm, &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], &terms, true, conf).unwrap().roc_auc.unwrap() > 0.9);
    assert!(multiple_poisson_regression(&x_glm, &[1.0, 2.0, 2.0, 4.0, 6.0, 9.0, 14.0, 22.0], &terms, true, conf).unwrap().terms[1].beta > 0.0);

    let msheet = mv_num_sheet(
        "s-mv",
        &[
            ("X", &[-2.0, -1.5, -1.0, -0.2, 0.4, 1.0, 1.6, 2.2]),
            ("Z", &[1.0, 0.5, 1.5, 0.8, 2.1, 1.9, 2.8, 3.1]),
            ("Y", &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
        ],
    );
    let glm_out = run_kind(
        &msheet,
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 2,
            predictor_columns: vec![0, 1],
            reference_levels: Vec::new(),
            compute_vif: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(!glm_out.tabs.is_empty());
    let pca_out = run_kind(
        &msheet,
        AnalysisKind::Multivariate(MultivariateSpec::Pca {
            standardize: true,
            parallel_analysis_seed: Some(42),
            pcr_outcome_column: None,
            retention_rule: Default::default(),
            emit_extra_matrices: false,
        }),
    )
    .unwrap();
    assert!(!pca_out.tabs.is_empty());

    let pts = [vec![0.0, 0.0], vec![0.2, 0.1], vec![5.0, 5.0], vec![5.2, 4.9]];
    assert_eq!(kmeans_pp(&pts, 2, 123).unwrap().assignments.len(), 4);
    assert_eq!(hierarchical_clustering(&pts, LinkageMethod::WardD2).unwrap().len(), 3);
}
