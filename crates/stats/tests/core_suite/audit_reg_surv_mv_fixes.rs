//! Consolidated regression & D1 reference-oracle tests for regression (MLR, Logistic, Poisson,
//! Passing-Bablok), survival (Kaplan-Meier, Log-rank, Cox PH), and multivariate (Correlation,
//! PCA, Clustering, Multiple Imputation) analyses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, Cell, ChecklistStatus, CiBound, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, GlmFamily,
    GlmSpec, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, PassingBablokSpec, ResultCell, ResultTabKind, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::SummaryMoments;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::StatsError;
use curvebook_stats::multiplicity::{adjust_holm_sidak, adjust_sidak};
use curvebook_stats::multivariate::{
    MAX_CLUSTERING_N, MultipleImputationConfig, cophenetic_correlation, cophenetic_distance_matrix, correlation_matrix, cut_dendrogram,
    hierarchical_clustering_with_metrics, kendall_tau_b, kmeans_pp, kmeans_pp_with_distance, mice_predictive_mean_matching, pairwise_distance_matrix,
    pca_analysis, rubin_pool_estimates, spearman_correlation,
};
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression, passing_bablok_regression};
use curvebook_stats::survival::{CoxSubject, compare_survival_curves, cox_ph_regression, kaplan_meier_curve, log_rank_trend_test};
use curvebook_testkit::assert_approx_eq;

use crate::common::{
    assert_close, cell_num, cell_p, mv_cells_sheet, mv_num_sheet, mv_opt_sheet, obs, presum_col_sheet, run_kind, run_kind_sel, scalar, scalar_opt,
    survival_sheet, tab_by_id, xy_multi_sheet, xy_sheet,
};

fn glm_kind(family: GlmFamily, resp: usize, preds: Vec<usize>, refs: Vec<(usize, String)>, vif: bool) -> AnalysisKind {
    AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family,
        response_column: resp,
        predictor_columns: preds,
        reference_levels: refs,
        compute_vif: vif,
    })
}

fn cox_kind(time: usize, status: usize, preds: Vec<usize>, refs: Vec<(usize, String)>, tie: CoxTieMethod) -> AnalysisKind {
    AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
        time_column: time,
        status_column: status,
        predictor_columns: preds,
        reference_levels: refs,
        tie_method: tie,
    })
}

#[test]
fn audit_19_20_survival_rank_df_censored_status_times_and_pairwise() {
    // 1. Covariance rank df drops zero-expected cohorts regardless of position (#19).
    let (c0, c1, c2) = (
        obs(&[(1.0, false), (2.0, false), (2.5, false)]),
        obs(&[(4.0, true), (6.0, true), (9.0, false), (12.0, true)]),
        obs(&[(5.0, false), (7.0, true), (10.0, true), (14.0, false)]),
    );
    for gehan in [false, true] {
        let r2 = compare_survival_curves(&[&c1, &c2], gehan).unwrap();
        for group in [[c0.as_slice(), c1.as_slice(), c2.as_slice()], [c1.as_slice(), c0.as_slice(), c2.as_slice()]] {
            let r3 = compare_survival_curves(&group, gehan).unwrap();
            assert_close(r3.df, 1.0, 1e-12, "rank df");
            assert_close(r3.chi_square, r2.chi_square, 1e-12, "chi2");
            assert_close(r3.p_value, r2.p_value, 1e-12, "P");
        }
    }

    // 2. All-censored comparisons report unavailable and are excluded from Bonferroni family (#20).
    let (a, b, c) = (obs(&[(1.0, false), (2.0, false)]), obs(&[(3.0, false), (4.0, false)]), obs(&[(5.0, false), (6.0, false)]));
    for res in [
        compare_survival_curves(&[&a, &b], false).unwrap(),
        compare_survival_curves(&[&a, &b], true).unwrap(),
        log_rank_trend_test(&[&a, &b, &c]).unwrap(),
    ] {
        assert!(res.chi_square.is_nan() && res.df.is_nan() && res.p_value.is_nan());
    }
    let sh0 = survival_sheet(
        "s0",
        &[
            ("CensA", &[(1.0, false), (2.0, false), (2.5, false)]),
            ("CensB", &[(3.0, false), (4.0, false), (4.5, false)]),
            ("EvC", &[(2.0, true), (5.0, true), (7.0, false)]),
        ],
    );
    let km = |mult: MultiplicitySpec, trend: bool| {
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: !trend,
            trend_test: trend,
            asymmetrical_greenwood_ci: true,
            multiplicity: mult,
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        })
    };
    let r01 = run_kind_sel(&sh0, km(MultiplicitySpec::default(), false), vec![0, 1]).unwrap();
    assert!(scalar_opt(&r01, "Log-rank chi-square", None).is_none());
    assert!(tab_by_id(&r01, "curve-comparison").rows.iter().all(|r| r.iter().all(|c| matches!(c, ResultCell::Empty))));
    let r_all =
        run_kind(&sh0, km(MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true }, true))
            .unwrap();
    let pw = tab_by_id(&r_all, "survival-pairwise");
    assert!(matches!(pw.rows[0][0], ResultCell::Empty) && r_all.pairwise_comparisons.len() == 2);
    assert_close(cell_p(&pw.rows[1][2]), (2.0 * cell_p(&pw.rows[1][1])).min(1.0), 1e-12, "Bonferroni m=2");

    // 3. KM rejects NaN/Inf/negative times (RV-31); Cox & MV KM enforce strict {0, 1} status (R1-S02-09).
    for bad_t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e-9, -5.0] {
        let bad_obs = obs(&[(2.0, true), (bad_t, false), (6.0, true)]);
        assert!(matches!(
            kaplan_meier_curve("C", &bad_obs, true, ConfidenceLevel::DEFAULT_95),
            Err(StatsError::InvalidParameter { name: "survival_time", .. })
        ));
    }
    for bad_st in [2.0, -1.0, 0.5] {
        let sh = mv_num_sheet("mvs", &[("T", &[5.0, 10.0, 15.0, 20.0]), ("S", &[0.0, bad_st, 1.0, 0.0]), ("D", &[1.0, 2.0, 3.0, 4.0])]);
        assert!(
            matches!(run_kind(&sh, cox_kind(0, 1, vec![2], vec![], CoxTieMethod::Efron)), Err(StatsError::InvalidParameter { name: "survival_status", value }) if (value - bad_st).abs() < 1e-12)
        );
        let mv_km = AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: None,
        });
        assert!(
            matches!(run_kind(&sh, mv_km), Err(StatsError::InvalidParameter { name: "survival_status", value }) if (value - bad_st).abs() < 1e-12)
        );
    }

    // 4. 4-cohort pairwise log-rank Sidak, Dunnett control-only, & ANOVA fallback footnotes (RV-67).
    let cohorts: [(&str, &[(f64, bool)]); 4] = [
        ("Control", &[(2.0, true), (3.0, true), (4.0, true), (5.0, true), (6.0, true), (8.0, false)]),
        ("Low", &[(4.0, true), (6.0, true), (7.0, true), (9.0, true), (11.0, false), (12.0, true)]),
        ("Mid", &[(7.0, true), (9.0, true), (12.0, true), (14.0, false), (15.0, true), (18.0, false)]),
        ("High", &[(11.0, true), (14.0, false), (16.0, true), (19.0, false), (22.0, true), (25.0, false)]),
    ];
    let obs4: Vec<_> = cohorts.iter().map(|(_, p)| obs(p)).collect();
    let mut raw_p = Vec::new();
    for i in 0..4 {
        for j in (i + 1)..4 {
            raw_p.push(compare_survival_curves(&[&obs4[i], &obs4[j]], false).unwrap().p_value);
        }
    }
    let (exp_s, exp_hs) = (adjust_sidak(&raw_p), adjust_holm_sidak(&raw_p));
    assert!(exp_s.iter().zip(&exp_hs).any(|(x, y)| (x - y).abs() > 1e-4));
    let sh4 = survival_sheet("rv67", &cohorts);
    let r_s = run_kind(&sh4, km(MultiplicitySpec { method: MultiplicityMethod::Sidak, threshold: 0.05, report_confidence_intervals: true }, false))
        .unwrap();
    for (rec, &ep) in r_s.pairwise_comparisons.iter().zip(&exp_s) {
        assert_close(rec.raw_p_or_q, ep, 1e-12, "Sidak pairwise");
    }
    let r_d = run_kind(
        &sh4,
        km(MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 0 }, threshold: 0.05, report_confidence_intervals: true }, false),
    )
    .unwrap();
    assert_eq!(r_d.pairwise_comparisons.len(), 3);
    assert!(tab_by_id(&r_d, "survival-pairwise").footnotes.iter().any(|f| f.contains("Dunnett") && f.contains("Bonferroni")));
    assert!(matches!(
        run_kind(
            &sh4,
            km(
                MultiplicitySpec { method: MultiplicityMethod::Dunnett { control_column: 9 }, threshold: 0.05, report_confidence_intervals: true },
                false
            )
        ),
        Err(StatsError::InvalidParameter { name: "control_column", .. })
    ));
    for (m, name) in [(MultiplicityMethod::TukeyHsd, "Tukey-Kramer"), (MultiplicityMethod::GamesHowell, "Games-Howell")] {
        let rf = run_kind(&sh4, km(MultiplicitySpec { method: m, threshold: 0.05, report_confidence_intervals: true }, false)).unwrap();
        assert!(tab_by_id(&rf, "survival-pairwise").footnotes.iter().any(|f| f.contains(name) && f.contains("Bonferroni")));
    }
}

#[test]
fn audit_16_cox_concordance_separation_schoenfeld_time_and_complete_case() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Zero comparable pairs -> concordance None & scalar omitted (#16).
    let same_t: Vec<_> = (0..4).map(|i| CoxSubject { time: 10.0, event: true, x: vec![i as f64] }).collect();
    let fit0 = cox_ph_regression(&same_t, &["Dose".into()], CoxTieMethod::Breslow, conf).unwrap();
    assert_eq!((fit0.comparable_pairs, fit0.concordance), (0, None));
    let sh0 = mv_num_sheet("c0", &[("T", &[10.0; 4]), ("S", &[1.0; 4]), ("D", &[0.0, 1.0, 2.0, 3.0])]);
    let r0 = run_kind(&sh0, cox_kind(0, 1, vec![2], vec![], CoxTieMethod::Breslow)).unwrap();
    assert!(scalar_opt(&r0, "Concordance (C-index)", None).is_none());
    assert_approx_eq(scalar(&r0, "Comparable pairs"), 0.0, 1e-12, 1e-12, "0 pairs");
    assert!(matches!(tab_by_id(&r0, "cox-ph").kind, ResultTabKind::ParametersAndConfidenceIntervals));

    // 2. Monotone separation & collinearity -> converged=false, NaN SE/CI, Empty/Open cells (R1-S02-05).
    let sep: Vec<_> = (1..=6).map(|i| CoxSubject { time: i as f64, event: i <= 3, x: vec![(7 - i) as f64] }).collect();
    let fs = cox_ph_regression(&sep, &["X".into()], CoxTieMethod::Breslow, conf).unwrap();
    assert!(!fs.converged && fs.covariates[0].se.is_nan() && fs.covariates[0].hr_ci.0.is_nan() && fs.likelihood_ratio_p.is_finite());
    let col_s: Vec<_> = [(1.0, true, 1.0), (2.0, false, 2.0), (3.0, true, 4.0), (4.0, true, 1.5), (5.0, false, 3.0), (6.0, true, 0.5)]
        .into_iter()
        .map(|(t, e, x)| CoxSubject { time: t, event: e, x: vec![x, 2.0 * x] })
        .collect();
    let fc = cox_ph_regression(&col_s, &["X1".into(), "X2".into()], CoxTieMethod::Efron, conf).unwrap();
    assert!(!fc.converged && fc.covariates.iter().all(|c| c.se.is_nan()));
    let sh_sep = mv_num_sheet(
        "csep",
        &[("T", &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), ("S", &[1.0, 1.0, 1.0, 0.0, 0.0, 0.0]), ("X", &[6.0, 5.0, 4.0, 3.0, 2.0, 1.0])],
    );
    let r_sep = run_kind(&sh_sep, cox_kind(0, 1, vec![2], vec![], CoxTieMethod::Breslow)).unwrap();
    assert_eq!(scalar_opt(&r_sep, "Converged", None), Some(0.0));
    let ct = tab_by_id(&r_sep, "cox-ph");
    assert!(
        matches!(ct.rows[0][1], ResultCell::Empty)
            && matches!(ct.rows[0][5], ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open })
    );

    // 3. Schoenfeld PH non-estimable when events < 3 or all events tied (RV-32) + time == 0 ok / < 0 err (FA-SEC06-11).
    let two_ev = vec![
        CoxSubject { time: 2.0, event: true, x: vec![0.6] },
        CoxSubject { time: 4.0, event: false, x: vec![1.5] },
        CoxSubject { time: 6.0, event: true, x: vec![0.1] },
        CoxSubject { time: 9.0, event: false, x: vec![-0.8] },
        CoxSubject { time: 11.0, event: false, x: vec![0.9] },
        CoxSubject { time: 14.0, event: false, x: vec![-0.3] },
    ];
    let c2 = cox_ph_regression(&two_ev, &["x".into()], CoxTieMethod::Efron, conf).unwrap();
    assert!(c2.converged && c2.global_schoenfeld_p.is_nan() && c2.covariates[0].schoenfeld_p.is_nan());
    let sh2 = mv_num_sheet(
        "c2ev",
        &[("T", &[2.0, 4.0, 6.0, 9.0, 11.0, 14.0]), ("S", &[1.0, 0.0, 1.0, 0.0, 0.0, 0.0]), ("X", &[0.6, 1.5, 0.1, -0.8, 0.9, -0.3])],
    );
    let r2 = run_kind(&sh2, cox_kind(0, 1, vec![2], vec![], CoxTieMethod::Efron)).unwrap();
    assert!(scalar_opt(&r2, "Global Schoenfeld P", None).is_none() && tab_by_id(&r2, "cox-ph").rows[0][6] == ResultCell::Empty);
    assert!(r2.checklist.iter().any(|c| c.assumption.contains("Schoenfeld") && c.status == ChecklistStatus::Warn));
    let tied3: Vec<_> = [(5.0, true, 0.8), (5.0, true, -0.3), (5.0, true, 0.2), (10.0, false, 1.4), (12.0, false, -1.1), (15.0, false, 0.1)]
        .into_iter()
        .map(|(t, e, x)| CoxSubject { time: t, event: e, x: vec![x] })
        .collect();
    assert!(cox_ph_regression(&tied3, &["x".into()], CoxTieMethod::Efron, conf).unwrap().global_schoenfeld_p.is_nan());

    let mut t0 = vec![
        CoxSubject { time: 0.0, event: false, x: vec![0.5] },
        CoxSubject { time: 2.0, event: true, x: vec![1.0] },
        CoxSubject { time: 4.0, event: true, x: vec![2.0] },
        CoxSubject { time: 6.0, event: false, x: vec![0.2] },
        CoxSubject { time: 8.0, event: true, x: vec![1.8] },
    ];
    assert!(cox_ph_regression(&t0, &["X".into()], CoxTieMethod::Breslow, conf).is_ok());
    t0[0].time = -1.0;
    assert!(matches!(
        cox_ph_regression(&t0, &["X".into()], CoxTieMethod::Breslow, conf),
        Err(StatsError::InvalidParameter { name: "cox_subject", .. })
    ));

    // 4. Categorical complete-case filtering R oracle & finite curve overlays (R2-S02-14).
    let t_cox = [4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 5.0, 7.0];
    let e_cox = [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0];
    let g_cox: Vec<Cell> = ["A", "A", "A", "B", "B", "B", "A", "B", "A", "B", "C", "C"].into_iter().map(Cell::text).collect();
    let x_cox: Vec<Cell> = [Some(1.2), Some(2.1), Some(0.8), Some(3.0), Some(2.5), Some(1.8), Some(0.9), Some(3.4), Some(1.1), Some(2.9), None, None]
        .into_iter()
        .map(|v| v.map_or_else(Cell::empty, Cell::number))
        .collect();
    let t_cells: Vec<Cell> = t_cox.into_iter().map(Cell::number).collect();
    let e_cells: Vec<Cell> = e_cox.into_iter().map(Cell::number).collect();
    let sh_cc = mv_cells_sheet(
        "cox-cc",
        &[("Time", None, &t_cells), ("Event", None, &e_cells), ("Arm", Some(VariableRole::Categorical), &g_cox), ("X", None, &x_cox)],
    );
    let r_cc = run_kind(&sh_cc, cox_kind(0, 1, vec![2, 3], vec![(2, "A".into())], CoxTieMethod::Efron)).unwrap();
    let ph = tab_by_id(&r_cc, "cox-ph");
    assert_eq!(ph.row_labels, vec!["Arm[B vs A]", "X"]);
    assert_close(cell_num(&ph.rows[0][0]), 1.105_599_189_738_33, 1e-7, "Cox coef Arm");
    assert_close(cell_num(&ph.rows[0][1]), 1.922_725_378_395_87, 1e-7, "Cox SE Arm");
    assert_close(cell_num(&ph.rows[1][0]), -1.319_374_615_050_9, 1e-7, "Cox coef X");
    assert_close(cell_num(&ph.rows[1][1]), 1.186_728_572_958_91, 1e-7, "Cox SE X");
    assert_close(scalar(&r_cc, "AIC"), 21.294_461_191_473_5, 1e-7, "Cox AIC");
    assert!(r_cc.fitted_curves.iter().all(|c| c.points.iter().all(|p| p.y.is_finite() && (0.0..=1.0).contains(&p.y))));
}

#[test]
fn audit_13_15_64_86_96_120_glm_linear_reference_levels_and_wampler() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. XY GLM selection & validation (#13) + source row indices (#15) + XY RawReplicates extraction (#120).
    let xs6 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let xy_sh = xy_multi_sheet("xyg", &xs6, &[("Null", &[1.5, 2.0, 2.5, 3.0, 3.5, 4.0]), ("Steep", &[15.0, 25.0, 35.0, 45.0, 55.0, 65.0])]);
    assert_approx_eq(
        cell_num(
            &tab_by_id(&run_kind_sel(&xy_sh, glm_kind(GlmFamily::Linear, 0, vec![0], vec![], false), vec![1]).unwrap(), "glm-linear-params").rows[1]
                [0],
        ),
        10.0,
        1e-10,
        1e-10,
        "sel=[1]",
    );
    assert_approx_eq(
        cell_num(&tab_by_id(&run_kind(&xy_sh, glm_kind(GlmFamily::Linear, 1, vec![0], vec![], false)).unwrap(), "glm-linear-params").rows[1][0]),
        10.0,
        1e-10,
        1e-10,
        "resp=1",
    );
    assert!(matches!(run_kind(&xy_sh, glm_kind(GlmFamily::Linear, 9, vec![0], vec![], false)), Err(StatsError::MissingColumn { index: 9 })));
    assert!(matches!(
        run_kind_sel(&xy_sh, glm_kind(GlmFamily::Linear, 0, vec![0], vec![], false), vec![9]),
        Err(StatsError::MissingColumn { index: 9 })
    ));
    assert!(matches!(run_kind(&xy_sh, glm_kind(GlmFamily::Linear, 0, vec![], vec![], false)), Err(StatsError::InsufficientSampleSize { .. })));
    assert!(matches!(run_kind(&xy_sh, glm_kind(GlmFamily::Linear, 0, vec![4], vec![], false)), Err(StatsError::MissingColumn { index: 4 })));
    assert!(matches!(
        run_kind(&xy_sh, glm_kind(GlmFamily::Linear, 0, vec![0, 1], vec![], false)),
        Err(StatsError::InvalidParameter { name: "predictor_columns", .. })
    ));

    let x_cells: Vec<Cell> = xs6.iter().enumerate().map(|(r, &x)| if r == 4 { Cell::excluded_number(x) } else { Cell::number(x) }).collect();
    for (fam, ys, tid) in [
        (GlmFamily::Linear, [2.0, 999.0, 6.1, 7.9, 10.2, 12.1], "glm-linear-residuals"),
        (GlmFamily::Logistic, [0.0, 1.0, 0.0, 1.0, 0.0, 1.0], "glm-logistic-predictions"),
        (GlmFamily::Poisson, [1.0, 9.0, 3.0, 6.0, 8.0, 14.0], "glm-poisson-residuals"),
    ] {
        let y_cells: Vec<Cell> = ys.iter().enumerate().map(|(r, &y)| if r == 1 { Cell::empty() } else { Cell::number(y) }).collect();
        let sh = mv_cells_sheet("mvr", &[("Y", None, &y_cells), ("X", None, &x_cells)]);
        assert_eq!(
            tab_by_id(&run_kind(&sh, glm_kind(fam, 0, vec![1], vec![], false)).unwrap(), tid).row_labels,
            vec!["Row 1", "Row 3", "Row 4", "Row 6"]
        );
    }

    let reps_all = [
        (GlmFamily::Linear, [[2.0, 2.4, 2.8], [4.1, 4.5, 4.9], [6.0, 6.4, 6.8], [8.2, 8.6, 9.0]], "glm-linear-residuals", "glm-linear-params"),
        (
            GlmFamily::Logistic,
            [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]],
            "glm-logistic-predictions",
            "glm-logistic-params",
        ),
        (GlmFamily::Poisson, [[1.0, 2.0, 1.0], [3.0, 4.0, 3.0], [6.0, 8.0, 7.0], [14.0, 16.0, 15.0]], "glm-poisson-residuals", "glm-poisson-params"),
    ];
    for (fam, reps, rid, pid) in reps_all {
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 4).unwrap();
        let (mut fx, mut fy) = (Vec::new(), Vec::new());
        for r in 0..4 {
            let xv = (r + 1) as f64;
            xy.set_x_cell(r, Cell::number(xv)).unwrap();
            for s in 0..3 {
                if r == 1 && s == 1 {
                    xy.data_sets[0].cells[r][s] = Cell::excluded_number(999.0);
                } else {
                    xy.data_sets[0].cells[r][s] = Cell::number(reps[r][s]);
                    fx.extend_from_slice(&[1.0, xv]);
                    fy.push(reps[r][s]);
                }
            }
        }
        let res = run_kind(&DataSheet::new("xyr", "XYR", TableArchetype::Xy(xy)).unwrap(), glm_kind(fam, 0, vec![0], vec![], false)).unwrap();
        let rt = tab_by_id(&res, rid);
        assert_eq!((rt.rows.len(), rt.row_labels[0].as_str(), rt.row_labels[4].as_str()), (11, "Row 1 (rep 1)", "Row 2 (rep 3)"));
        let xm = DenseMatrix::from_row_major(11, 2, fx).unwrap();
        let nm = ["Intercept".into(), "Dose".into()];
        let exp_s = match fam {
            GlmFamily::Linear => multiple_linear_regression(&xm, &fy, &nm, true, false, conf).unwrap().terms[1].estimate,
            GlmFamily::Logistic => multiple_logistic_regression(&xm, &fy, &nm, false, conf).unwrap().terms[1].beta,
            GlmFamily::Poisson => multiple_poisson_regression(&xm, &fy, &nm, false, conf).unwrap().terms[1].beta,
        };
        assert_close(cell_num(&tab_by_id(&res, pid).rows[1][0]), exp_s, 1e-10, "XY replicate slope");
    }
}

#[test]
fn audit_64_86_96_glm_aliased_exact_fit_reference_levels_and_wampler() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let xs6 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    // 2. Aliased columns (#64), exact linear fit (#86), absent reference level (#96), no-intercept VIF (FA-SEC06-08), joint multi-df Type III (R2-S02-15), Wampler Dot2/DW.
    let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y_a: Vec<f64> = x1.iter().zip([0.05, -0.08, 0.04, -0.02, 0.07, -0.06, 0.03, -0.03]).map(|(&x, e)| 1.0 + 2.0 * x + e).collect();
    let mlr = multiple_linear_regression(
        &DenseMatrix::from_row_major(8, 3, (0..8).flat_map(|i| [1.0, x1[i], x1[i]]).collect()).unwrap(),
        &y_a,
        &["Int".into(), "X1".into(), "X2".into()],
        true,
        true,
        conf,
    )
    .unwrap();
    let ai = mlr.aliased_columns.iter().position(|&a| a).unwrap();
    assert_eq!(mlr.rank, 2);
    assert!(mlr.terms[ai].estimate.is_nan() && mlr.terms[ai].vif.is_none());
    assert_close(mlr.terms[3 - ai].estimate, 2.0, 0.05, "unhalved retained slope");
    let sh_al = mv_num_sheet("al", &[("Y", &y_a), ("X1", &x1), ("X2", &x1)]);
    let r_al = run_kind(&sh_al, glm_kind(GlmFamily::Linear, 0, vec![1, 2], vec![], true)).unwrap();
    assert!(matches!(&tab_by_id(&r_al, "glm-linear-params").rows[ai][0], ResultCell::Text(s) if s == "Aliased"));
    let y_mi: Vec<Option<f64>> = y_a.iter().enumerate().map(|(i, &v)| if i == 3 { None } else { Some(v) }).collect();
    let x_mi: Vec<Option<f64>> = x1.iter().copied().map(Some).collect();
    let r_mi = run_kind(
        &mv_opt_sheet("mial", &[("Y", &y_mi), ("X1", &x_mi), ("X2", &x_mi)]),
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 3,
            max_iters: 5,
            seed: 19,
            response_column: Some(0),
            predictor_columns: vec![1, 2],
        }),
    )
    .unwrap();
    assert!(tab_by_id(&r_mi, "mi-pooled-regression").rows.iter().any(|r| matches!(&r[0], ResultCell::Text(s) if s == "Aliased")));

    let ey: Vec<f64> = xs6.iter().map(|&x| 1.0 + 2.0 * x).collect();
    let ex = multiple_linear_regression(
        &DenseMatrix::from_row_major(6, 2, xs6.iter().flat_map(|&x| [1.0, x]).collect()).unwrap(),
        &ey,
        &["Int".into(), "x".into()],
        true,
        false,
        conf,
    )
    .unwrap();
    assert!(ex.rss.abs() < 1e-24 && ex.terms[1].t_statistic.is_nan() && ex.terms[1].p_value.is_nan());
    let r_ex = run_kind(&xy_sheet("ex", &xs6, &ey), glm_kind(GlmFamily::Linear, 0, vec![0], vec![], false)).unwrap();
    assert!(matches!(tab_by_id(&r_ex, "glm-linear-params").rows[1][2], ResultCell::Empty));

    let arms: Vec<Cell> = (0..8).map(|r| Cell::text(if r % 2 == 0 { "Control" } else { "Treated" })).collect();
    let sh_ref = mv_cells_sheet(
        "ref",
        &[
            ("T", None, &(0..8).map(|r| Cell::number((r + 2) as f64)).collect::<Vec<_>>()),
            ("E", None, &(0..8).map(|r| Cell::number(if r % 3 == 0 { 0.0 } else { 1.0 })).collect::<Vec<_>>()),
            ("D", None, &(0..8).map(|r| Cell::number(r as f64 * 0.5)).collect::<Vec<_>>()),
            ("Arm", Some(VariableRole::Categorical), &arms),
        ],
    );
    assert!(matches!(
        run_kind(&sh_ref, glm_kind(GlmFamily::Linear, 0, vec![2, 3], vec![(3, "ctrl".into())], false)),
        Err(StatsError::InvalidParameter { name: "reference_levels", .. })
    ));
    assert!(matches!(
        run_kind(&sh_ref, cox_kind(0, 1, vec![2, 3], vec![(3, "ctrl".into())], CoxTieMethod::Breslow)),
        Err(StatsError::InvalidParameter { name: "reference_levels", .. })
    ));
    assert!(matches!(
        run_kind(&sh_ref, glm_kind(GlmFamily::Linear, 0, vec![2, 3], vec![(9, "Control".into())], false)),
        Err(StatsError::MissingColumn { index: 9 })
    ));

    let x2_v: Vec<f64> = xs6.iter().zip([0.001, -0.001, 0.002, -0.002, 0.001, -0.001]).map(|(&x, j)| 1007.0 - x + j).collect();
    let y_v: Vec<f64> = xs6.iter().zip(&x2_v).map(|(&a, &b)| 2.0 * a + 0.5 * b + 1.0).collect();
    let vif_i = multiple_linear_regression(
        &DenseMatrix::from_row_major(6, 3, (0..6).flat_map(|i| [1.0, xs6[i], x2_v[i]]).collect()).unwrap(),
        &y_v,
        &["I".into(), "X1".into(), "X2".into()],
        true,
        true,
        conf,
    )
    .unwrap()
    .terms[1]
        .vif
        .unwrap();
    let mlr_ni = multiple_linear_regression(
        &DenseMatrix::from_row_major(6, 2, (0..6).flat_map(|i| [xs6[i], x2_v[i]]).collect()).unwrap(),
        &y_v,
        &["X1".into(), "X2".into()],
        false,
        true,
        conf,
    )
    .unwrap();
    assert!(mlr_ni.terms[0].vif.unwrap() > 1e5 && (mlr_ni.terms[0].vif.unwrap() - vif_i).abs() / vif_i < 1e-6);

    let y14: Vec<Cell> = [12.1, 13.4, 11.8, 15.2, 16.0, 14.5, 22.3, 24.1, 21.8, 20.5, 23.0, 25.4, 26.2, 27.1].into_iter().map(Cell::number).collect();
    let d14: Vec<Cell> =
        ["Low", "Low", "Low", "Low", "Med", "Med", "Med", "Med", "Med", "High", "High", "High", "High", "High"].into_iter().map(Cell::text).collect();
    let a14: Vec<Cell> = [20.0, 25.0, 22.0, 30.0, 28.0, 32.0, 40.0, 45.0, 38.0, 35.0, 42.0, 48.0, 50.0, 52.0].into_iter().map(Cell::number).collect();
    let r_t3 = run_kind(
        &mv_cells_sheet("t3", &[("Y", None, &y14), ("Dose", Some(VariableRole::Categorical), &d14), ("Age", None, &a14)]),
        glm_kind(GlmFamily::Linear, 0, vec![1, 2], vec![(1, "Low".into())], true),
    )
    .unwrap();
    let t3 = tab_by_id(&r_t3, "glm-linear-type-iii");
    assert_eq!(t3.row_labels, vec!["Dose", "Age"]);
    assert_close(cell_num(&t3.rows[0][0]), 1.529_160_527_705_57, 1e-10, "Dose Type III SS");
    assert_close(cell_num(&t3.rows[0][1]), 2.0, 1e-12, "Dose Type III df");
    assert_close(cell_num(&t3.rows[0][2]), 1.529_160_527_705_57 / 2.0, 1e-10, "Dose Type III MS");
    assert_close(cell_num(&t3.rows[0][3]), 0.497_311_575_846_308, 1e-10, "Dose Type III F");
    assert_close(cell_p(&t3.rows[0][4]), 0.622_441_096_295_923, 1e-10, "Dose Type III P");
    assert_close(cell_num(&t3.rows[1][0]), 91.757_229_603_183_9, 1e-10, "Age Type III SS");
    assert_close(cell_num(&t3.rows[1][1]), 1.0, 1e-12, "Age Type III df");
    assert_close(cell_num(&t3.rows[1][3]), 59.682_331_086_219_3, 1e-10, "Age Type III F");
    assert_close(cell_p(&t3.rows[1][4]), 1.596_720_368_339_24e-5, 1e-12, "Age Type III P");

    let w_names: Vec<String> = (0..6).map(|j| format!("B{j}")).collect();
    for scale in [1.0_f64, 1.0 + f64::EPSILON, 3.0, 1.0e-12] {
        let mut des = Vec::with_capacity(21 * 6);
        let (mut yw1, mut yw2) = (Vec::with_capacity(21), Vec::with_capacity(21));
        for i in 0..=20 {
            let x = (i as f64) * scale;
            let row = [1.0, x, x.powi(2), x.powi(3), x.powi(4), x.powi(5)];
            des.extend_from_slice(&row);
            yw1.push(row.iter().sum());
            yw2.push(1.0 + 0.1 * x + 0.01 * x.powi(2) + 0.001 * x.powi(3) + 0.0001 * x.powi(4) + 0.00001 * x.powi(5));
        }
        let xm = DenseMatrix::from_row_major(21, 6, des).unwrap();
        for y in [&yw1, &yw2] {
            let r = multiple_linear_regression(&xm, y, &w_names, true, true, conf).unwrap();
            assert_close(r.durbin_watson, 2.0, 1e-12, "Wampler DW == 2");
            for (&yi, (&yh, &ri)) in y.iter().zip(r.fitted.iter().zip(r.residuals.iter())) {
                assert!(ri.abs() <= 1e-8 * yi.abs().max(1.0) && (yh + ri - yi).abs() <= 1e-9 * yi.abs().max(1.0));
            }
        }
    }
}

#[test]
fn audit_84_85_101_103_logistic_poisson_and_singular_fisher() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Scale-invariant logistic separation (#84).
    let ys40 = [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
        1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ];
    let mut ref_pair = None;
    for scale in [100.0_f64, 1.0, 0.25, 0.1] {
        let xs: Vec<f64> = (1..=40).map(|i| (i as f64 / 40.0) * scale).collect();
        let fit = multiple_logistic_regression(
            &DenseMatrix::from_row_major(40, 2, xs.iter().flat_map(|&x| [1.0, x]).collect()).unwrap(),
            &ys40,
            &["Int".into(), "X".into()],
            false,
            conf,
        )
        .unwrap();
        assert!(fit.converged && !fit.separation_detected);
        let cur = (fit.terms[1].beta * scale, fit.deviance_residual);
        if let Some((rb, rd)) = ref_pair {
            assert_close(cur.0, rb, 1e-8, "logistic scale slope");
            assert_close(cur.1, rd, 1e-8, "logistic scale dev");
        } else {
            ref_pair = Some(cur);
        }
    }
    let xs_sep: Vec<f64> = (1..=10).map(|i| (i as f64) * 100.0).collect();
    let ys_sep: Vec<f64> = xs_sep.iter().map(|&x| if x > 500.0 { 1.0 } else { 0.0 }).collect();
    let r_sep = run_kind(&mv_num_sheet("sep", &[("Y", &ys_sep), ("X", &xs_sep)]), glm_kind(GlmFamily::Logistic, 0, vec![1], vec![], false)).unwrap();
    assert_eq!(scalar_opt(&r_sep, "Converged", None), Some(0.0));
    assert!(r_sep.checklist.iter().any(|c| c.assumption == "Logistic separation & calibration" && c.status == ChecklistStatus::Warn));

    // 2. Hosmer-Lemeshow ties & < 3 groups unavailable (#85).
    let mut y_f = vec![1.0; 5];
    y_f.extend(vec![0.0; 15]);
    y_f.extend(vec![1.0; 15]);
    y_f.extend(vec![0.0; 5]);
    let mut x_f = vec![0.0; 20];
    x_f.extend(vec![1.0; 20]);
    let r_hl = run_kind(&mv_num_sheet("hl", &[("Y", &y_f), ("X", &x_f)]), glm_kind(GlmFamily::Logistic, 0, vec![1], vec![], false)).unwrap();
    assert!(scalar_opt(&r_hl, "Hosmer-Lemeshow chi2", None).is_none());
    let mut rows4 = Vec::new();
    for xl in [0.0, 1.0, 2.0, 3.0] {
        for k in 0..10 {
            rows4.push((if (k as f64) < 2.0 + 2.0 * xl { 1.0 } else { 0.0 }, xl));
        }
    }
    let mut rows4_rev = rows4.clone();
    rows4_rev.reverse();
    let fit_a = multiple_logistic_regression(
        &DenseMatrix::from_row_major(40, 2, rows4.iter().flat_map(|&(_, x)| [1.0, x]).collect()).unwrap(),
        &rows4.iter().map(|&(y, _)| y).collect::<Vec<_>>(),
        &["I".into(), "L".into()],
        false,
        conf,
    )
    .unwrap();
    let fit_b = multiple_logistic_regression(
        &DenseMatrix::from_row_major(40, 2, rows4_rev.iter().flat_map(|&(_, x)| [1.0, x]).collect()).unwrap(),
        &rows4_rev.iter().map(|&(y, _)| y).collect::<Vec<_>>(),
        &["I".into(), "L".into()],
        false,
        conf,
    )
    .unwrap();
    assert_close(fit_a.hosmer_lemeshow_df, 2.0, 1e-12, "HL df=2");
    assert_close(fit_a.hosmer_lemeshow_chi2, fit_b.hosmer_lemeshow_chi2, 1e-12, "HL row-order inv");

    // 3. Poisson unclamped 1e11 counts (#101), non-integer rejection (#103), & singular Fisher info (RV-26).
    let mut r = 2.0_f64;
    for _ in 0..30 {
        r -= (0.74 * r.powi(3) - 0.26 * r.powi(2) - 1.26 * r - 2.26) / (2.22 * r.powi(2) - 0.52 * r - 1.26);
    }
    let p_lg = multiple_poisson_regression(
        &DenseMatrix::from_row_major(4, 2, vec![1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 3.0]).unwrap(),
        &[1e11, 2.1e11, 3.9e11, 8e11],
        &["I".into(), "x".into()],
        false,
        conf,
    )
    .unwrap();
    assert!(p_lg.converged && p_lg.terms[0].beta > 25.0);
    assert_close(p_lg.terms[1].beta, r.ln(), 1e-8, "Poisson 1e11 b1");
    assert_close(p_lg.terms[0].beta, (1.5e12_f64 / (1.0 + r + r.powi(2) + r.powi(3))).ln(), 1e-8, "Poisson 1e11 b0");
    assert!(matches!(
        run_kind(
            &mv_num_sheet("pf", &[("R", &[1.0, 2.5, 4.0, 6.0, 9.0]), ("X", &[0.0, 1.0, 2.0, 3.0, 4.0])]),
            glm_kind(GlmFamily::Poisson, 0, vec![1], vec![], false)
        ),
        Err(StatsError::InvalidParameter { name: "poisson_y", .. })
    ));

    let x10 = [0.2, 0.5, 0.9, 1.3, 1.8, 2.2, 2.7, 3.1, 3.6, 4.0];
    let xm_col = DenseMatrix::from_row_major(10, 3, x10.iter().flat_map(|&v| [1.0, v, 2.0 * v + 1.0]).collect()).unwrap();
    let yb = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let yc = [2.0, 3.0, 5.0, 4.0, 8.0, 11.0, 14.0, 19.0, 23.0, 30.0];
    assert!(matches!(
        multiple_logistic_regression(&xm_col, &yb, &["I".into(), "x1".into(), "x2".into()], true, conf),
        Err(StatsError::Math(MathError::SingularMatrix { rank: 2, expected_rank: 3 }))
    ));
    assert!(matches!(
        multiple_poisson_regression(&xm_col, &yc, &["I".into(), "x1".into(), "x2".into()], true, conf),
        Err(StatsError::Math(MathError::SingularMatrix { rank: 2, expected_rank: 3 }))
    ));
    let x_sat = DenseMatrix::from_row_major(8, 2, [-1e4, -1e4, -1e4, -1e4, 1e4, 1e4, 1e4, 1e4].iter().flat_map(|&v| [1.0, v]).collect()).unwrap();
    assert!(matches!(
        multiple_logistic_regression(&x_sat, &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], &["I".into(), "x".into()], false, conf),
        Err(StatsError::Math(MathError::SingularMatrix { .. }))
    ));
    let x_psep =
        DenseMatrix::from_row_major(10, 2, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0].iter().flat_map(|&g| [1.0, g]).collect()).unwrap();
    assert!(matches!(
        multiple_poisson_regression(&x_psep, &[6.0, 8.0, 5.0, 7.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0], &["I".into(), "g".into()], false, conf),
        Err(StatsError::Math(MathError::SingularMatrix { .. }))
    ));
    assert!(matches!(
        run_kind(&mv_num_sheet("sgl", &[("Y", &yb), ("X1", &x10), ("X2", &x10)]), glm_kind(GlmFamily::Logistic, 0, vec![1, 2], vec![], true)),
        Err(StatsError::Math(MathError::SingularMatrix { .. }))
    ));
}

#[test]
fn audit_102_117_passing_bablok_cusum_rows_replicates_vertical_and_alpha() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. CUSUM exact Passing & Bablok (1983, §3.3) H / sqrt(L + l) normalization & Type I error control (#102, AUD-05-03).
    let mut rng = DeterministicRng::from_seed(0x102_BABC);
    for (n, trials) in [(30_usize, 400_usize), (100_usize, 250_usize)] {
        let mut rej = 0_usize;
        for _ in 0..trials {
            let (mut xs, mut ys) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for _ in 0..n {
                let x = 1.0 + 9.0 * rng.next_open_f64();
                xs.push(x + 0.25 * rng.standard_normal());
                ys.push(1.5 + 1.2 * x + 0.25 * rng.standard_normal());
            }
            let pb = passing_bablok_regression(&xs, &ys, conf).unwrap();
            let scale_y = ys.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
            let scale_x = xs.iter().map(|v| v.abs()).fold(0.0_f64, f64::max);
            let tol = 1e-12 * (1.0 + scale_y + (pb.slope * scale_x).abs());
            let l_plus_l = pb.residuals.iter().filter(|&&r| r.abs() > tol).count();
            assert_close(pb.cusum_statistic, pb.cusum_h / (l_plus_l as f64).sqrt(), 1e-12, "PB CUSUM H/sqrt(L+l)");
            if pb.cusum_p_value < 0.05 {
                rej += 1;
            }
        }
        let rate = rej as f64 / trials as f64;
        assert!(rate <= 0.05, "PB CUSUM n={n} Type I error rate={rate:.3} exceeded nominal 0.05");
    }

    // 2. Source row indices across Column, MV, and XY (#117) + XY RawReplicates & overflow sanitization (FA-SEC06-07).
    let (xs, ys) = ([1.0, 2.0, 3.0, 4.0, 5.0, 6.0], [2.1, 4.0, 6.2, 7.9, 10.1, 12.0]);
    let xc: Vec<Cell> = xs.iter().enumerate().map(|(r, &x)| if r == 4 { Cell::excluded_number(x) } else { Cell::number(x) }).collect();
    let yc: Vec<Cell> = ys.iter().enumerate().map(|(r, &y)| if r == 1 { Cell::empty() } else { Cell::number(y) }).collect();
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 6).unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    for r in 0..6 {
        col.data_sets[0].cells[r][0] = xc[r].clone();
        col.data_sets[1].cells[r][0] = yc[r].clone();
        xy.set_x_cell(r, xc[r].clone()).unwrap();
        xy.data_sets[0].cells[r][0] = yc[r].clone();
    }
    let pb_k = AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 });
    for sh in [
        DataSheet::new("pbc", "PBC", TableArchetype::Column(col)).unwrap(),
        mv_cells_sheet("pbm", &[("X", None, &xc), ("Y", None, &yc)]),
        DataSheet::new("pbx", "PBX", TableArchetype::Xy(xy)).unwrap(),
    ] {
        assert_eq!(tab_by_id(&run_kind(&sh, pb_k.clone()).unwrap(), "pb-residuals").row_labels, vec!["Row 1", "Row 3", "Row 4", "Row 6"]);
    }

    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 5).unwrap();
    let (mut fx, mut fy) = (Vec::new(), Vec::new());
    for r in 0..5 {
        let xv = (r + 1) as f64;
        xy_rep.set_x_cell(r, Cell::number(xv)).unwrap();
        for (s, yv) in [xv + 0.1, 1.5 * xv + 0.2, 2.0 * xv - 0.1].into_iter().enumerate() {
            xy_rep.data_sets[0].cells[r][s] = Cell::number(yv);
            fx.push(xv);
            fy.push(yv);
        }
    }
    let r_rep = run_kind(&DataSheet::new("pbr", "PBR", TableArchetype::Xy(xy_rep)).unwrap(), pb_k.clone()).unwrap();
    let r_flt = run_kind(&xy_sheet("pbf", &fx, &fy), pb_k.clone()).unwrap();
    assert_eq!((scalar(&r_rep, "N"), scalar(&r_rep, "Slope")), (15.0, scalar(&r_flt, "Slope")));
    assert!(matches!(
        run_kind(&xy_sheet("pbb", &[0.0, 1.0, 2.0, 3.0, 1e307], &[0.0, 1e10, 2e10, 3e10, 1e307]), pb_k),
        Err(StatsError::Math(MathError::DomainError { name: "non_finite_curve_point", .. }))
    ));

    // 3. Vertical slopes +inf included in N (order-invariant K; AUD-05-34) + X<->Y equivariance (RV-34) + linearity alpha (FA-SEC06-11).
    let pb_v = passing_bablok_regression(&[1.0, 2.0, 2.0, 4.0, 6.0], &[2.0, 5.0, 20.0, 8.0, 11.0], conf).unwrap();
    let pb_s = passing_bablok_regression(&[1.0, 2.0, 2.0, 4.0, 6.0], &[2.0, 20.0, 5.0, 8.0, 11.0], conf).unwrap();
    assert_eq!((pb_v.valid_slopes, pb_v.k_offset, pb_s.valid_slopes, pb_s.k_offset), (10, 2, 10, 2));
    assert_close(pb_v.slope, 2.5, 1e-12, "PB vertical slope");
    assert_close(pb_s.slope, pb_v.slope, 1e-12, "PB swap slope");
    assert_eq!(passing_bablok_regression(&[1.0, 2.0, 2.0, 4.0, 6.0], &[2.0, 5.0, 5.0, 8.0, 11.0], conf).unwrap().valid_slopes, 9);
    let (hx, hy) = ([1.0, 2.0, 3.0, 4.5, 6.0, 8.0, 10.0], [3.0, 5.2, 5.2, 10.1, 13.0, 17.4, 21.0]);
    let (pxy, pyx) = (passing_bablok_regression(&hx, &hy, conf).unwrap(), passing_bablok_regression(&hy, &hx, conf).unwrap());
    assert_close(pyx.slope, 1.0 / pxy.slope, 1e-12, "PB X<->Y slope");
    assert_close(pyx.intercept, -pxy.intercept / pxy.slope, 1e-11, "PB X<->Y intercept");
    let xs36: Vec<f64> = (1..=36).map(f64::from).collect();
    let ys36: Vec<f64> = xs36.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    assert!(!passing_bablok_regression(&xs36, &ys36, conf).unwrap().linearity_passed);
    assert!(passing_bablok_regression(&xs36, &ys36, ConfidenceLevel::PERCENT_99).unwrap().linearity_passed);
}

#[test]
fn audit_39_87_91_92_113_multivariate_corr_pca_clustering_and_mi() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Undefined correlations unavailable & excluded from BH FDR (#39) + Spearman/Kendall error contexts (FA-SEC06-11).
    let (a, b, k, d) = (
        [Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)],
        [Some(2.0), Some(1.0), Some(4.0), Some(3.0), Some(6.0)],
        [Some(7.0); 5],
        [Some(1.0), Some(3.0), None, None, None],
    );
    let cm4 = correlation_matrix(&[("A", &a), ("B", &b), ("K", &k), ("D", &d)], false).unwrap();
    assert_close(cm4.fdr_q_matrix[0][1], cm4.p_matrix[0][1], 1e-12, "q==p excluding undef");
    for (i, j) in [(0, 2), (1, 2), (0, 3), (1, 3), (2, 3)] {
        assert!(cm4.r_matrix[i][j].is_nan() && cm4.p_matrix[i][j].is_nan() && cm4.fdr_q_matrix[i][j].is_nan());
    }
    let r_cm = run_kind(
        &mv_opt_sheet("cm", &[("A", &a), ("B", &b), ("C", &k), ("D", &d)]),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
    )
    .unwrap();
    assert!(matches!(tab_by_id(&r_cm, "corr-matrix-r").rows[0][2], ResultCell::Empty));
    assert_close(cell_p(&tab_by_id(&r_cm, "corr-matrix-fdr").rows[0][1]), cell_p(&tab_by_id(&r_cm, "corr-matrix-p").rows[0][1]), 1e-12, "sheet q==p");
    assert!(matches!(
        spearman_correlation(&[1.0, 2.0], &[2.0, 3.0]),
        Err(StatsError::InsufficientSampleSize { context: "Spearman correlation", required: 3, actual: 2 })
    ));
    assert!(matches!(
        kendall_tau_b(&[1.0, 2.0], &[2.0, 3.0]),
        Err(StatsError::InsufficientSampleSize { context: "Kendall tau-b correlation", required: 3, actual: 2 })
    ));

    // 2. Rubin zero-variance (#87), MI bounds (#113), & 1-var ABB coverage (#91).
    let p0 = rubin_pool_estimates("C", &[5.0; 3], &[0.0; 3], Some(9.0), conf).unwrap();
    assert!(p0.t_statistic.is_nan() && p0.p_value.is_nan() && p0.ci.0.is_nan());
    let cmi_sh = mv_opt_sheet("cmi", &[("C", &[Some(5.0), Some(5.0), None, Some(5.0), None, Some(5.0)])]);
    let r_cmi = run_kind(
        &cmi_sh,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 4,
            max_iters: 3,
            seed: 1,
            response_column: None,
            predictor_columns: vec![],
        }),
    )
    .unwrap();
    assert!(matches!(tab_by_id(&r_cmi, "mi-pooled-means").rows[0][5], ResultCell::Empty));
    let mi_cols = vec![vec![Some(1.0), Some(2.0), Some(3.0), None, Some(5.0)]];
    for bad_m in [0, 1, 51, 200] {
        assert!(matches!(
            mice_predictive_mean_matching(
                &["X".into()],
                &mi_cols,
                &MultipleImputationConfig { imputations_m: bad_m, max_iters: 5, ..Default::default() }
            ),
            Err(StatsError::InvalidParameter { name: "imputations_m", .. })
        ));
    }
    for bad_it in [0, 101, 500] {
        assert!(matches!(
            mice_predictive_mean_matching(
                &["X".into()],
                &mi_cols,
                &MultipleImputationConfig { imputations_m: 5, max_iters: bad_it, ..Default::default() }
            ),
            Err(StatsError::InvalidParameter { name: "max_iters", .. })
        ));
    }
    let mut rng_mi = DeterministicRng::from_seed(0x91_CAFE);
    let mut covered = 0_usize;
    for trial in 0..250_usize {
        let col: Vec<Option<f64>> = (0..40).map(|r| if r < 20 { Some(10.0 + 3.0 * rng_mi.standard_normal()) } else { None }).collect();
        let res = mice_predictive_mean_matching(
            &["Y".into()],
            &[col],
            &MultipleImputationConfig {
                imputations_m: 10,
                max_iters: 1,
                seed: 5000 + trial as u64,
                response_col: None,
                predictor_cols: vec![],
                confidence: conf,
            },
        )
        .unwrap();
        covered += usize::from(res.pooled_means[0].ci.0 <= 10.0 && 10.0 <= res.pooled_means[0].ci.1);
    }
    assert!((0.91..=0.985).contains(&(covered as f64 / 250.0)));
}

#[test]
fn audit_fa_sec06_02_multivariate_presummarized_xy_parity_and_horn_pa() {
    // 3. Multivariate pre-summarized guards & XY RawReplicates parity (FA-SEC06-02).
    let mv_specs = [
        MultivariateSpec::CorrelationMatrix { spearman_rank: false },
        MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: None },
        MultivariateSpec::Clustering { k_means_k: Some(2), distance: ClusterDistance::Euclidean, linkage: ClusterLinkage::Ward, k_sweep_max: None },
        MultivariateSpec::MultipleImputation { imputations_m: 3, max_iters: 5, seed: 42, response_column: None, predictor_columns: vec![] },
    ];
    let col_pre = presum_col_sheet(
        "cpre",
        SubcolumnMode::MeanSdN,
        &[("G0", &[&[10.0, 1.5, 5.0], &[12.0, 1.8, 5.0]]), ("G1", &[&[13.0, 1.5, 5.0], &[15.0, 1.8, 5.0]])],
    );
    let mut xy_px = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::Single, 2, 5).unwrap();
    for r in 0..5 {
        xy_px.set_x_subcell(r, 0, Cell::number((r + 1) as f64)).unwrap();
        xy_px.set_x_subcell(r, 1, Cell::number(0.5)).unwrap();
        xy_px.set_x_subcell(r, 2, Cell::number(3.0)).unwrap();
        (xy_px.data_sets[0].cells[r][0], xy_px.data_sets[1].cells[r][0]) = (Cell::number(2.0 * r as f64 + 1.0), Cell::number(3.0 * r as f64 + 2.0));
    }
    let sh_xy_px = DataSheet::new("xypx", "XYPX", TableArchetype::Xy(xy_px)).unwrap();
    for sh in [&col_pre, &sh_xy_px] {
        for ms in &mv_specs {
            assert!(matches!(run_kind(sh, AnalysisKind::Multivariate(ms.clone())), Err(StatsError::RequiresRawReplicates)));
        }
    }
    let mut xy_r = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 5).unwrap();
    let mut xy_f = XyTable::new(SubcolumnMode::Single, 2, 15).unwrap();
    for r in 0..5 {
        let xv = (r + 1) as f64;
        xy_r.set_x_cell(r, Cell::number(xv)).unwrap();
        let (y1, y2) = ([xv, 10.0 * xv + 1.0, -5.0 * xv + 3.0], [2.0 * xv + 0.5, 4.0 * xv - 1.0, 0.5 * xv + 8.0]);
        for s in 0..3 {
            (xy_r.data_sets[0].cells[r][s], xy_r.data_sets[1].cells[r][s]) = (Cell::number(y1[s]), Cell::number(y2[s]));
            xy_f.set_x_cell(r * 3 + s, Cell::number(xv)).unwrap();
            (xy_f.data_sets[0].cells[r * 3 + s][0], xy_f.data_sets[1].cells[r * 3 + s][0]) = (Cell::number(y1[s]), Cell::number(y2[s]));
        }
    }
    let (sh_r, sh_f) =
        (DataSheet::new("xyr2", "R", TableArchetype::Xy(xy_r)).unwrap(), DataSheet::new("xyf2", "F", TableArchetype::Xy(xy_f)).unwrap());
    for ms in &mv_specs {
        let (rr, rf) =
            (run_kind(&sh_r, AnalysisKind::Multivariate(ms.clone())).unwrap(), run_kind(&sh_f, AnalysisKind::Multivariate(ms.clone())).unwrap());
        assert_eq!(rr.scalars, rf.scalars);
        assert_eq!(rr.tabs[0].rows, rf.tabs[0].rows);
    }

    // 4. Horn PA covariance scale invariance (RV-27) & PCA zero-variance + deterministic signs (RV-33).
    let mut rng_pa = DeterministicRng::from_seed(1965);
    let mut pa_raw = Vec::with_capacity(30 * 6);
    for _ in 0..30 {
        let (f1, f2) = (2.5 * rng_pa.standard_normal(), 1.8 * rng_pa.standard_normal());
        pa_raw.extend_from_slice(&[
            f1 + 0.25 * rng_pa.standard_normal(),
            0.9 * f1 + 0.25 * rng_pa.standard_normal(),
            -1.1 * f1 + 0.25 * rng_pa.standard_normal(),
            f2 + 0.25 * rng_pa.standard_normal(),
            0.85 * f2 + 0.25 * rng_pa.standard_normal(),
            -0.95 * f2 + 0.25 * rng_pa.standard_normal(),
        ]);
    }
    let lbl6: Vec<String> = (1..=6).map(|i| format!("V{i}")).collect();
    let pca1 = pca_analysis(&DenseMatrix::from_row_major(30, 6, pa_raw.clone()).unwrap(), &lbl6, false, Some(4242)).unwrap();
    let pca100 =
        pca_analysis(&DenseMatrix::from_row_major(30, 6, pa_raw.iter().map(|&v| v * 100.0).collect()).unwrap(), &lbl6, false, Some(4242)).unwrap();
    assert_eq!((pca1.parallel_analysis_components, pca100.parallel_analysis_components), (Some(2), Some(2)));
    assert_close(
        pca100.parallel_analysis_thresholds.as_ref().unwrap()[0] / pca1.parallel_analysis_thresholds.as_ref().unwrap()[0],
        10_000.0,
        1e-6,
        "PA scale^2",
    );

    let c_rows = [[1.0, 5.0, 2.1], [2.0, 5.0, 3.9], [3.0, 5.0, 6.2], [4.0, 5.0, 8.1], [5.0, 5.0, 9.8]];
    let m_c = DenseMatrix::from_row_major(5, 3, c_rows.iter().flatten().copied().collect()).unwrap();
    assert!(matches!(pca_analysis(&m_c, &["V1".into(), "V2".into(), "V3".into()], true, None), Err(StatsError::ZeroVariance { .. })));
    let p_cov = pca_analysis(&m_c, &["V1".into(), "V2".into(), "V3".into()], false, None).unwrap();
    let exp_tr = SummaryMoments::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap().sample_variance().unwrap()
        + SummaryMoments::from_slice(&[2.1, 3.9, 6.2, 8.1, 9.8]).unwrap().sample_variance().unwrap();
    assert_close(p_cov.eigenvalues.iter().sum(), exp_tr, 1e-12, "cov PCA trace");
    let pos_r = [[-2.5, -1.8, 3.1], [-1.9, -2.2, 2.4], [-0.8, -0.5, 1.1], [0.7, 0.9, -0.8], [1.8, 2.1, -2.6], [2.7, 1.5, -3.2]];
    let p_pos = pca_analysis(
        &DenseMatrix::from_row_major(6, 3, pos_r.iter().flatten().copied().collect()).unwrap(),
        &["V1".into(), "V2".into(), "V3".into()],
        true,
        None,
    )
    .unwrap();
    let p_neg = pca_analysis(
        &DenseMatrix::from_row_major(6, 3, pos_r.iter().flatten().map(|&v| -v).collect()).unwrap(),
        &["V1".into(), "V2".into(), "V3".into()],
        true,
        None,
    )
    .unwrap();
    for j in 0..3 {
        assert_close(p_pos.loadings[0][j], p_neg.loadings[0][j], 1e-12, "PCA loading sign inv");
        assert_close(p_pos.scores[0][j], -p_neg.scores[0][j], 1e-12, "PCA score sign eqv");
    }

    // 5. Clustering: k-means distinct cap / empty repair / tie-break (#92) + 7 R hclust oracles + MAX_CLUSTERING_N + n=550 tab cap (R2-S02-16).
    let dup = vec![vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.0], vec![5.0, 5.0], vec![5.0, 5.0], vec![5.0, 5.0], vec![5.0, 5.0]];
    for kb in [3, 4] {
        assert!(matches!(kmeans_pp(&dup, kb, 7), Err(StatsError::InvalidParameter { name: "k_means_k", .. })));
    }
    let sh_dup = mv_num_sheet("kmd", &[("X", &[0.0, 0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 5.0]), ("Y", &[0.0, 0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 5.0])]);
    assert_eq!(
        scalar(
            &run_kind(
                &sh_dup,
                AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                    k_means_k: None,
                    distance: ClusterDistance::Euclidean,
                    linkage: ClusterLinkage::Ward,
                    k_sweep_max: None
                })
            )
            .unwrap(),
            "Clusters (k)"
        ),
        2.0
    );
    let skewed = vec![vec![0.0, 0.0], vec![0.01, 0.0], vec![1.0, 0.0], vec![100.0, 0.0], vec![1000.0, 0.0]];
    for dist in [ClusterDistance::Euclidean, ClusterDistance::Manhattan] {
        for seed in 0..8 {
            let km = kmeans_pp_with_distance(&skewed, 4, dist, seed).unwrap();
            assert!((0..4).all(|c| km.assignments.contains(&c)));
        }
    }

    let hrows = vec![vec![0.0, 0.1], vec![0.2, 0.0], vec![5.0, 5.1], vec![5.3, 4.9], vec![2.0, 2.5], vec![2.2, 2.1], vec![8.0, 8.2], vec![8.4, 7.8]];
    let dmat = pairwise_distance_matrix(&hrows, ClusterDistance::Euclidean).unwrap();
    for (link, exp_r, exp_h) in [
        (
            ClusterLinkage::Single,
            0.737_496_810_905_61,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 2.9, 3.96988664826, 4.24499705536],
        ),
        (
            ClusterLinkage::Complete,
            0.804_033_950_247_816,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.12409987036, 4.34165866922, 11.3951744173],
        ),
        (
            ClusterLinkage::Upgma,
            0.813_323_509_880_609,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.01947449499, 4.29109694192, 7.71674937249],
        ),
        (
            ClusterLinkage::Wpgma,
            0.813_323_509_880_609,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.01947449499, 4.29109694192, 7.71674937249],
        ),
        (
            ClusterLinkage::Ward,
            0.805_636_760_560_721,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 4.25734659148, 6.05020660804, 15.4189818081],
        ),
        (
            ClusterLinkage::Centroid,
            0.813_322_928_679_129_5,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.0103986447, 4.27814212013, 7.70949090407],
        ),
        (
            ClusterLinkage::Median,
            0.813_322_928_679_129_5,
            [0.22360679775, 0.360555127546, 0.4472135955, 0.565685424949, 3.0103986447, 4.27814212013, 7.70949090407],
        ),
    ] {
        let m = hierarchical_clustering_with_metrics(&hrows, ClusterDistance::Euclidean, link).unwrap();
        let rc = cophenetic_correlation(&dmat, &cophenetic_distance_matrix(8, &m).unwrap()).unwrap();
        assert_close(rc, exp_r, 1e-10, "cophenetic r");
        for (i, (step, &eh)) in m.iter().zip(&exp_h).enumerate() {
            assert_close(step.distance, eh, 1e-9, &format!("{link:?}[{i}]"));
        }
        let cut = cut_dendrogram(8, &m, 4).unwrap();
        assert!(cut[0] == cut[1] && cut[2] == cut[3] && cut[4] == cut[5] && cut[6] == cut[7]);
    }
    let km_m = kmeans_pp_with_distance(&hrows, 4, ClusterDistance::Manhattan, 7).unwrap();
    let l1: f64 = hrows.iter().zip(&km_m.assignments).map(|(r, &c)| (r[0] - km_m.centroids[c][0]).abs() + (r[1] - km_m.centroids[c][1]).abs()).sum();
    assert_close(km_m.wcss, l1, 1e-12, "Manhattan L1 WCSS");
    assert!(matches!(
        pairwise_distance_matrix(&vec![vec![1.0, 2.0]; MAX_CLUSTERING_N + 1], ClusterDistance::Euclidean),
        Err(StatsError::InvalidParameter { name: "clustering_n", .. })
    ));
    let x550: Vec<f64> = (0..550).map(|i| (i % 3) as f64 * 10.0 + ((i % 7) as f64) * 0.1).collect();
    let y550: Vec<f64> = (0..550).map(|i| (i % 3) as f64 * 10.0 + ((i % 5) as f64) * 0.1).collect();
    let r550 = run_kind(
        &mv_num_sheet("c550", &[("X", &x550), ("Y", &y550)]),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: None,
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: Some(100),
        }),
    )
    .unwrap();
    assert!(scalar(&r550, "cophenetic_r").is_finite() && !r550.tabs.iter().any(|t| t.id == "clustering-distance-matrix"));
}
