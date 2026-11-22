//! Regression and D1 reference-oracle tests for Wave 2 audit findings:
//! #19, #20, #32, #39, #64, #84, #85, #86, #87, #91, #92, #96, #101, #102, #103, #113, #117, #120.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet,
    GlmFamily, GlmSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, PassingBablokSpec, ResultCell, ResultSheet,
    ResultTab, SheetId, SubcolumnMode, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype, VariableRole,
    XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::rng::DeterministicRng;
use curvebook_stats::multivariate::{
    MultipleImputationConfig, correlation_matrix, kmeans_pp, kmeans_pp_with_distance, mice_predictive_mean_matching, rubin_pool_estimates,
};
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression, passing_bablok_regression};
use curvebook_stats::survival::{compare_survival_curves, log_rank_trend_test};
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected}, got {actual} (diff = {})", (actual - expected).abs());
}

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

fn scalar_opt(sheet: &ResultSheet, key: &str) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index.is_none()).map(|s| s.value)
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

fn obs(rows: &[(f64, bool)]) -> Vec<SurvivalObservation> {
    rows.iter()
        .enumerate()
        .map(|(row, &(time, ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect()
}

// ============================================================================
// #19 & #20: Survival df from covariance rank & 0-event unavailability
// ============================================================================

#[test]
fn audit_19_survival_comparison_df_matches_covariance_rank() {
    // Arm 0 is censored completely before any event in Arms 1 & 2, so Arm 0 has zero expected events.
    // D1 R survival::survdiff drops zero-expected groups cleanly -> df = 1 instead of k - 1 = 2.
    let c0 = obs(&[(1.0, false), (2.0, false), (2.5, false)]);
    let c1 = obs(&[(4.0, true), (6.0, true), (9.0, false), (12.0, true)]);
    let c2 = obs(&[(5.0, false), (7.0, true), (10.0, true), (14.0, false)]);

    for gehan in [false, true] {
        let cmp_3 = compare_survival_curves(&[&c0, &c1, &c2], gehan).unwrap();
        let cmp_2 = compare_survival_curves(&[&c1, &c2], gehan).unwrap();
        assert_close(cmp_3.df, 1.0, 1e-12, "rank df drops 0-expected cohort");
        assert_close(cmp_3.chi_square, cmp_2.chi_square, 1e-12, "chi2 matches active 2 cohorts");
        assert_close(cmp_3.p_value, cmp_2.p_value, 1e-12, "P value matches active 2 cohorts");

        // Also invariant when the zero-expected cohort sits last or middle.
        let cmp_mid = compare_survival_curves(&[&c1, &c0, &c2], gehan).unwrap();
        assert_close(cmp_mid.df, 1.0, 1e-12, "middle 0-expected cohort df");
        assert_close(cmp_mid.chi_square, cmp_2.chi_square, 1e-12, "middle 0-expected cohort chi2");
    }
}

#[test]
fn audit_20_all_censored_survival_comparisons_report_unavailable() {
    let a = obs(&[(1.0, false), (2.0, false)]);
    let b = obs(&[(3.0, false), (4.0, false)]);
    let c = obs(&[(5.0, false), (6.0, false)]);

    let lr = compare_survival_curves(&[&a, &b], false).unwrap();
    let gbw = compare_survival_curves(&[&a, &b], true).unwrap();
    let trend = log_rank_trend_test(&[&a, &b, &c]).unwrap();
    for res in [&lr, &gbw, &trend] {
        assert!(res.chi_square.is_nan(), "{}: expected NaN chi2, got {}", res.test_name, res.chi_square);
        assert!(res.df.is_nan(), "{}: expected NaN df, got {}", res.test_name, res.df);
        assert!(res.p_value.is_nan(), "{}: expected NaN P, got {}", res.test_name, res.p_value);
    }

    // End-to-end 3-arm table with 2 all-censored arms and 1 event arm:
    // pairwise (0 vs 1) has 0 events -> unavailable and excluded from multiplicity family.
    let mut st = SurvivalTable::new(3, 9).unwrap();
    st.cohorts[0].title = String::from("CensA");
    st.cohorts[1].title = String::from("CensB");
    st.cohorts[2].title = String::from("EventsC");
    for (r, &t) in [1.0, 2.0, 2.5].iter().enumerate() {
        st.times[r] = Cell::number(t);
        st.cohorts[0].cells[r][0] = Cell::number(0.0);
    }
    for (r, &t) in [3.0, 4.0, 4.5].iter().enumerate() {
        st.times[3 + r] = Cell::number(t);
        st.cohorts[1].cells[3 + r][0] = Cell::number(0.0);
    }
    for (r, &(t, ev)) in [(2.0, true), (5.0, true), (7.0, false)].iter().enumerate() {
        st.times[6 + r] = Cell::number(t);
        st.cohorts[2].cells[6 + r][0] = Cell::number(if ev { 1.0 } else { 0.0 });
    }
    let sheet = DataSheet::new(SheetId::new("surv-0ev"), "AllCens", TableArchetype::Survival(st)).unwrap();

    // All-censored subset [0, 1] via run_analysis must emit Empty cells (not 0 / 1) and Info note.
    let mut spec_01 = spec_for(
        "r-surv-01",
        "surv-0ev",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: true,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    );
    spec_01.selected_data_sets = vec![0, 1];
    let res_01 = run_analysis(&sheet, &spec_01).unwrap();
    assert!(scalar_opt(&res_01, "Log-rank chi-square").is_none());
    assert!(scalar_opt(&res_01, "Log-rank P value").is_none());
    let cmp_tab = tab_by_id(&res_01, "curve-comparison");
    assert!(cmp_tab.rows.iter().all(|row| row.iter().all(|c| matches!(c, ResultCell::Empty))));
    assert!(res_01.checklist.iter().any(|c| c.assumption == "Survival curve comparison estimability" && c.status == ChecklistStatus::Info));

    // All 3 cohorts: pairwise (CensA vs CensB) unavailable & excluded from Bonferroni m = 2 (CensA vs EventsC, CensB vs EventsC).
    let spec_all = spec_for(
        "r-surv-all",
        "surv-0ev",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: true,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: true },
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    );
    let res_all = run_analysis(&sheet, &spec_all).unwrap();
    let pw = tab_by_id(&res_all, "survival-pairwise");
    assert!(matches!(pw.rows[0][0], ResultCell::Empty), "all-censored pair chi2 must be Empty");
    assert!(matches!(pw.rows[0][1], ResultCell::Empty), "all-censored pair raw P must be Empty");
    assert!(matches!(pw.rows[0][2], ResultCell::Empty), "all-censored pair adjusted P must be Empty");
    assert!(matches!(pw.rows[0][5], ResultCell::Empty), "all-censored pair significance must be Empty");
    assert_eq!(res_all.pairwise_comparisons.len(), 2, "all-censored pair excluded from PairwiseComparisonRecord");
    let raw_p1 = cell_p(&pw.rows[1][1]);
    let adj_p1 = cell_p(&pw.rows[1][2]);
    assert_close(adj_p1, (raw_p1 * 2.0).min(1.0), 1e-12, "Bonferroni m = 2 excludes unavailable pair");
}

// ============================================================================
// #32: Duplicate & out-of-range selected_data_sets rejected across families
// ============================================================================

#[test]
fn audit_32_selected_data_sets_rejects_oob_and_duplicates_across_modules() {
    // 1. Survival table KM
    let mut st = SurvivalTable::new(2, 8).unwrap();
    for c in 0..2 {
        for r in 0..4 {
            let row = c * 4 + r;
            st.times[row] = Cell::number((r + 1) as f64);
            st.cohorts[c].cells[row][0] = Cell::number(1.0);
        }
    }
    let surv_sheet = DataSheet::new(SheetId::new("s-sel"), "SurvSel", TableArchetype::Survival(st)).unwrap();
    let base_surv = AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox: true,
        gehan_breslow_wilcoxon: false,
        trend_test: false,
        asymmetrical_greenwood_ci: true,
        multiplicity: MultiplicitySpec::default(),
        mv_time_column: None,
        mv_status_column: None,
        mv_group_column: None,
    });
    for (bad_sel, is_oob) in [(vec![9], true), (vec![0, 0], false)] {
        let mut s = spec_for("r-s", "s-sel", base_surv.clone());
        s.selected_data_sets = bad_sel;
        let err = run_analysis(&surv_sheet, &s).unwrap_err();
        if is_oob {
            assert!(matches!(err, StatsError::MissingColumn { index: 9 }), "got {err:?}");
        } else {
            assert!(matches!(err, StatsError::InvalidParameter { name: "selected_data_sets", .. }), "got {err:?}");
        }
    }

    // 2. MV sheet for Cox, GLM, Passing-Bablok, Correlation, PCA, Clustering, MI
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X1");
    mv.variables[2].title = String::from("X2");
    for r in 0..6 {
        let x = (r + 1) as f64;
        mv.variables[0].cells[r] = Cell::number(1.0 + 2.0 * x);
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number((r % 2) as f64);
    }
    let mv_sheet = DataSheet::new(SheetId::new("mv-sel"), "MvSel", TableArchetype::MultipleVariables(mv)).unwrap();

    let kinds = vec![
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 2,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Breslow,
        }),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
        AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 0 }),
        AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: None, pcr_outcome_column: None }),
        AnalysisKind::Multivariate(MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        }),
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 3,
            max_iters: 5,
            seed: 7,
            response_column: None,
            predictor_columns: Vec::new(),
        }),
    ];
    for kind in kinds {
        let mut s_oob = spec_for("r-oob", "mv-sel", kind.clone());
        s_oob.selected_data_sets = vec![12];
        assert!(matches!(run_analysis(&mv_sheet, &s_oob), Err(StatsError::MissingColumn { index: 12 })), "kind={kind:?}");

        let mut s_dup = spec_for("r-dup", "mv-sel", kind.clone());
        s_dup.selected_data_sets = vec![0, 1, 0];
        assert!(matches!(run_analysis(&mv_sheet, &s_dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })), "kind={kind:?}");
    }

    // 3. Duplicate on XY GLM
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * (r + 1) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(3.0 * (r + 1) as f64);
    }
    let xy_sheet = DataSheet::new(SheetId::new("xy-sel"), "XySel", TableArchetype::Xy(xy)).unwrap();
    let mut xy_dup = spec_for(
        "r-xy-dup",
        "xy-sel",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![0],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    );
    xy_dup.selected_data_sets = vec![0, 0];
    assert!(matches!(run_analysis(&xy_sheet, &xy_dup), Err(StatsError::InvalidParameter { name: "selected_data_sets", .. })));
}

// ============================================================================
// #39: Undefined correlations unavailable & excluded from BH FDR family
// ============================================================================

#[test]
fn audit_39_undefined_correlations_unavailable_and_excluded_from_fdr() {
    let a = [Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)];
    let b = [Some(2.0), Some(1.0), Some(4.0), Some(3.0), Some(6.0)];
    let k_const = [Some(7.0), Some(7.0), Some(7.0), Some(7.0), Some(7.0)];
    let d_sparse = [Some(1.0), Some(3.0), None, None, None];

    let cm3 = correlation_matrix(&[("A", &a), ("B", &b), ("K", &k_const)], false).unwrap();
    let cm4 = correlation_matrix(&[("A", &a), ("B", &b), ("K", &k_const), ("D", &d_sparse)], false).unwrap();

    // Only (A, B) is defined -> q(A, B) must equal raw P(A, B) (approx 0.0877058...), uninflated by K or D.
    assert_close(cm3.fdr_q_matrix[0][1], cm3.p_matrix[0][1], 1e-12, "q(A,B) == P(A,B) with constant column");
    assert_close(cm4.fdr_q_matrix[0][1], cm4.p_matrix[0][1], 1e-12, "q(A,B) == P(A,B) with sparse < 3 pair column");

    for pair in [(0, 2), (1, 2), (0, 3), (1, 3), (2, 3)] {
        assert!(cm4.r_matrix[pair.0][pair.1].is_nan(), "expected NaN r for {pair:?}");
        assert!(cm4.p_matrix[pair.0][pair.1].is_nan(), "expected NaN P for {pair:?}");
        assert!(cm4.fdr_q_matrix[pair.0][pair.1].is_nan(), "expected NaN q for {pair:?}");
    }

    // End-to-end ResultSheet must render unavailable cells as Empty and note exclusion.
    let mut mv = MultipleVariablesTable::new(4, 5).unwrap();
    mv.variables[0].title = String::from("A");
    mv.variables[1].title = String::from("B");
    mv.variables[2].title = String::from("Const");
    mv.variables[3].title = String::from("TwoPairs");
    for r in 0..5 {
        mv.variables[0].cells[r] = Cell::number(a[r].unwrap());
        mv.variables[1].cells[r] = Cell::number(b[r].unwrap());
        mv.variables[2].cells[r] = Cell::number(k_const[r].unwrap());
        mv.variables[3].cells[r] = d_sparse[r].map_or_else(Cell::empty, Cell::number);
    }
    let sheet = DataSheet::new(SheetId::new("mv-corr"), "CorrUndef", TableArchetype::MultipleVariables(mv)).unwrap();
    let res = run_analysis(
        &sheet,
        &spec_for("r-corr", "mv-corr", AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false })),
    )
    .unwrap();
    let tab_r = tab_by_id(&res, "corr-matrix-r");
    let tab_p = tab_by_id(&res, "corr-matrix-p");
    let tab_q = tab_by_id(&res, "corr-matrix-fdr");
    assert!(matches!(tab_r.rows[0][2], ResultCell::Empty));
    assert!(matches!(tab_p.rows[0][2], ResultCell::Empty));
    assert!(matches!(tab_q.rows[0][2], ResultCell::Empty));
    assert_close(cell_p(&tab_q.rows[0][1]), cell_p(&tab_p.rows[0][1]), 1e-12, "sheet q(A,B) == P(A,B)");
    assert!(tab_q.footnotes.iter().any(|f| f.contains("excluded from Benjamini-Hochberg")));
}

// ============================================================================
// #64 & #86: Aliased columns in MLR / GLM / MI & Exact Linear Fit
// ============================================================================

#[test]
fn audit_64_and_86_aliased_columns_and_exact_linear_fit() {
    // 1. Exact collinearity (#64): y = 1 + 2*x1 + noise, predictor x2 = x1.
    // Basic QR solution retains x1 with unhalved slope ~ 2.0 and flags one aliased column.
    let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let noise = [0.05, -0.08, 0.04, -0.02, 0.07, -0.06, 0.03, -0.03];
    let y: Vec<f64> = x1.iter().zip(noise.iter()).map(|(&x, &e)| 1.0 + 2.0 * x + e).collect();
    let x_mat = DenseMatrix::from_row_major(8, 3, (0..8).flat_map(|i| [1.0, x1[i], x1[i]]).collect()).unwrap();
    let mlr = multiple_linear_regression(
        &x_mat,
        &y,
        &[String::from("Intercept"), String::from("X1"), String::from("X2_dup")],
        true,
        true,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap();
    assert_eq!(mlr.rank, 2);
    let aliased_idx = mlr.aliased_columns.iter().position(|&a| a).expect("one column must be aliased");
    let retained_idx = if aliased_idx == 1 { 2 } else { 1 };
    assert_close(mlr.terms[retained_idx].estimate, 2.0, 0.05, "retained partner unhalved");
    assert!(mlr.terms[aliased_idx].estimate.is_nan());
    assert!(mlr.terms[aliased_idx].se.is_nan());
    assert!(mlr.terms[aliased_idx].t_statistic.is_nan());
    assert!(mlr.terms[aliased_idx].p_value.is_nan());
    assert!(mlr.terms[aliased_idx].ci.0.is_nan() && mlr.terms[aliased_idx].ci.1.is_nan());
    assert_eq!(mlr.terms[aliased_idx].vif, None);
    for k in 0..3 {
        assert_close(mlr.cov_beta[aliased_idx][k], 0.0, 0.0, "aliased zero cov_beta row");
        assert_close(mlr.cov_beta[k][aliased_idx], 0.0, 0.0, "aliased zero cov_beta col");
    }

    // End-to-end GLM sheet marks aliased term "Aliased" with Empty stats + Warn checklist.
    let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X1");
    mv.variables[2].title = String::from("X2_dup");
    for i in 0..8 {
        mv.variables[0].cells[i] = Cell::number(y[i]);
        mv.variables[1].cells[i] = Cell::number(x1[i]);
        mv.variables[2].cells[i] = Cell::number(x1[i]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-alias"), "Aliased", TableArchetype::MultipleVariables(mv)).unwrap();
    let res = run_analysis(
        &sheet,
        &spec_for(
            "r-alias",
            "mv-alias",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![1, 2],
                reference_levels: Vec::new(),
                compute_vif: true,
            }),
        ),
    )
    .unwrap();
    let ptab = tab_by_id(&res, "glm-linear-params");
    assert!(matches!(&ptab.rows[aliased_idx][0], ResultCell::Text(s) if s == "Aliased"));
    assert!(matches!(ptab.rows[aliased_idx][1], ResultCell::Empty));
    assert!(matches!(ptab.rows[aliased_idx][2], ResultCell::Empty));
    assert!(matches!(ptab.rows[aliased_idx][3], ResultCell::Empty));
    assert!(matches!(ptab.rows[aliased_idx][4], ResultCell::Empty));
    assert!(matches!(ptab.rows[aliased_idx][5], ResultCell::Empty));
    assert_close(cell_num(&ptab.rows[retained_idx][0]), 2.0, 0.05, "GLM retained estimate unhalved");
    assert!(res.checklist.iter().any(|c| c.assumption.contains("aliased") && c.status == ChecklistStatus::Warn));

    // Also MI regression on aliased column surfaces "Aliased" rather than failing or splitting.
    let mut mv_mi = MultipleVariablesTable::new(3, 8).unwrap();
    mv_mi.variables[0].title = String::from("Y");
    mv_mi.variables[1].title = String::from("X1");
    mv_mi.variables[2].title = String::from("X2_dup");
    for i in 0..8 {
        mv_mi.variables[0].cells[i] = if i == 3 { Cell::empty() } else { Cell::number(y[i]) };
        mv_mi.variables[1].cells[i] = Cell::number(x1[i]);
        mv_mi.variables[2].cells[i] = Cell::number(x1[i]);
    }
    let mi_sheet = DataSheet::new(SheetId::new("mv-mi-alias"), "MiAlias", TableArchetype::MultipleVariables(mv_mi)).unwrap();
    let mi_res = run_analysis(
        &mi_sheet,
        &spec_for(
            "r-mi-alias",
            "mv-mi-alias",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 3,
                max_iters: 5,
                seed: 19,
                response_column: Some(0),
                predictor_columns: vec![1, 2],
            }),
        ),
    )
    .unwrap();
    let mi_reg = tab_by_id(&mi_res, "mi-pooled-regression");
    assert!(mi_reg.rows.iter().any(|r| matches!(&r[0], ResultCell::Text(s) if s == "Aliased")));
}

#[test]
fn audit_86_exact_linear_fit_marks_t_p_ci_unavailable() {
    // 2. Exact linear fit (#86): y = 1 + 2x for x = 1..=6 -> SE = 0, t/P/CI unavailable (never t=0, P=1).
    let ex = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ey: Vec<f64> = ex.iter().map(|&x| 1.0 + 2.0 * x).collect();
    let ex_mat = DenseMatrix::from_row_major(6, 2, ex.iter().flat_map(|&x| [1.0, x]).collect()).unwrap();
    let exact =
        multiple_linear_regression(&ex_mat, &ey, &[String::from("Intercept"), String::from("x")], true, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(exact.rss, 0.0, 1e-24, "exact fit RSS == 0");
    assert_close(exact.terms[1].estimate, 2.0, 1e-12, "exact fit slope == 2");
    assert_close(exact.terms[1].se, 0.0, 1e-12, "exact fit SE == 0");
    assert!(exact.terms[0].t_statistic.is_nan() && exact.terms[1].t_statistic.is_nan());
    assert!(exact.terms[0].p_value.is_nan() && exact.terms[1].p_value.is_nan());
    assert!(exact.terms[0].ci.0.is_nan() && exact.terms[1].ci.1.is_nan());

    let mut xy_ex = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    for (r, (&xv, &yv)) in ex.iter().zip(ey.iter()).enumerate() {
        xy_ex.set_x_cell(r, Cell::number(xv)).unwrap();
        xy_ex.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let ex_sheet = DataSheet::new(SheetId::new("xy-exact"), "ExactLin", TableArchetype::Xy(xy_ex)).unwrap();
    let ex_res = run_analysis(
        &ex_sheet,
        &spec_for(
            "r-exact",
            "xy-exact",
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
    let ex_tab = tab_by_id(&ex_res, "glm-linear-params");
    assert!(matches!(ex_tab.rows[1][2], ResultCell::Empty), "exact fit t must be Empty");
    assert!(matches!(ex_tab.rows[1][3], ResultCell::Empty), "exact fit P must be Empty");
    assert!(matches!(ex_tab.rows[1][4], ResultCell::Empty), "exact fit CI must be Empty");
}

// ============================================================================
// #84 & #85: Scale-invariant logistic separation, convergence & HL ties
// ============================================================================

#[test]
fn audit_84_logistic_separation_scale_invariance_and_convergence_reporting() {
    // 1. Non-separated 40-point dataset rescaled by s in {100, 1, 0.25, 0.1}:
    // Previously s <= 1 tripped raw |beta| > 15 and injected bias. Now ALL scales match identically.
    let ys = [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0,
        1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ];
    let mut ref_beta1_unit = None;
    let mut ref_dev = None;
    for scale in [100.0_f64, 1.0, 0.25, 0.1] {
        let xs: Vec<f64> = (1..=40).map(|i| ((i as f64) / 40.0) * scale).collect();
        let x_mat = DenseMatrix::from_row_major(40, 2, xs.iter().flat_map(|&x| [1.0, x]).collect()).unwrap();
        let fit =
            multiple_logistic_regression(&x_mat, &ys, &[String::from("Intercept"), String::from("X")], false, ConfidenceLevel::DEFAULT_95).unwrap();
        assert!(fit.converged && !fit.separation_detected, "scale={scale}: expected clean convergence without separation");
        let scaled_slope = fit.terms[1].beta * scale;
        let scaled_se = fit.terms[1].se * scale;
        assert!(scaled_slope > 10.0, "expected steep unpenalized slope, got {scaled_slope}");
        if let Some(rb) = ref_beta1_unit {
            assert_close(scaled_slope, rb, 1e-8, &format!("scale={scale} unit slope invariance"));
            assert_close(fit.deviance_residual, ref_dev.unwrap(), 1e-8, &format!("scale={scale} deviance invariance"));
        } else {
            ref_beta1_unit = Some(scaled_slope);
            ref_dev = Some(fit.deviance_residual);
        }
        assert!(scaled_se.is_finite() && scaled_se > 0.0);
    }

    // 2. Separated large-scale x = 100..=1000, y = (x > 500): must detect separation and converged=false.
    let xs_sep: Vec<f64> = (1..=10).map(|i| (i as f64) * 100.0).collect();
    let ys_sep: Vec<f64> = xs_sep.iter().map(|&x| if x > 500.0 { 1.0 } else { 0.0 }).collect();
    let mut mv = MultipleVariablesTable::new(2, 10).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X_large");
    for i in 0..10 {
        mv.variables[0].cells[i] = Cell::number(ys_sep[i]);
        mv.variables[1].cells[i] = Cell::number(xs_sep[i]);
    }
    let sep_sheet = DataSheet::new(SheetId::new("mv-sep1000"), "SepLarge", TableArchetype::MultipleVariables(mv)).unwrap();
    let res = run_analysis(
        &sep_sheet,
        &spec_for(
            "r-sep1000",
            "mv-sep1000",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 0,
                predictor_columns: vec![1],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap();
    assert_close(scalar_opt(&res, "Converged").unwrap(), 0.0, 0.0, "separated logistic scalar Converged == 0");
    let chk = res.checklist.iter().find(|c| c.assumption == "Logistic separation & calibration").unwrap();
    assert_eq!(chk.status, ChecklistStatus::Warn);
    assert!(!chk.explanation.contains("IRLS converged without separation"));
    assert!(tab_by_id(&res, "glm-logistic-params").footnotes.iter().any(|f| f.contains("separation")));
}

#[test]
fn audit_85_hosmer_lemeshow_keeps_ties_and_marks_under_3_groups_unavailable() {
    // 1. Saturated binary predictor (40 obs: 20 at x=0 with 5 events => p=0.25, 20 at x=1 with 15 events => p=0.75).
    let mut mv_first = MultipleVariablesTable::new(2, 40).unwrap();
    let mut mv_inter = MultipleVariablesTable::new(2, 40).unwrap();
    mv_first.variables[0].title = String::from("Y");
    mv_first.variables[1].title = String::from("X");
    mv_inter.variables[0].title = String::from("Y");
    mv_inter.variables[1].title = String::from("X");

    let mut rows_first = Vec::with_capacity(40);
    rows_first.extend(std::iter::repeat_n((1.0, 0.0), 5));
    rows_first.extend(std::iter::repeat_n((0.0, 0.0), 15));
    rows_first.extend(std::iter::repeat_n((1.0, 1.0), 15));
    rows_first.extend(std::iter::repeat_n((0.0, 1.0), 5));

    let mut rows_inter = Vec::with_capacity(40);
    for i in 0..20 {
        rows_inter.push((if i % 4 == 0 { 1.0 } else { 0.0 }, 0.0));
        rows_inter.push((if i % 4 != 0 { 1.0 } else { 0.0 }, 1.0));
    }

    for (r, ((yf, xf), (yi, xi))) in rows_first.into_iter().zip(rows_inter).enumerate() {
        mv_first.variables[0].cells[r] = Cell::number(yf);
        mv_first.variables[1].cells[r] = Cell::number(xf);
        mv_inter.variables[0].cells[r] = Cell::number(yi);
        mv_inter.variables[1].cells[r] = Cell::number(xi);
    }

    let glm_spec =
        GlmSpec { family: GlmFamily::Logistic, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let sh_f = DataSheet::new(SheetId::new("hl-f"), "HLfirst", TableArchetype::MultipleVariables(mv_first)).unwrap();
    let sh_i = DataSheet::new(SheetId::new("hl-i"), "HLinter", TableArchetype::MultipleVariables(mv_inter)).unwrap();
    let res_f = run_analysis(&sh_f, &spec_for("r-f", "hl-f", AnalysisKind::GeneralizedLinearModel(glm_spec.clone()))).unwrap();
    let res_i = run_analysis(&sh_i, &spec_for("r-i", "hl-i", AnalysisKind::GeneralizedLinearModel(glm_spec))).unwrap();

    // Only 2 distinct fitted probability groups -> HL unavailable cleanly on both orderings.
    assert!(scalar_opt(&res_f, "Hosmer-Lemeshow chi2").is_none());
    assert!(scalar_opt(&res_i, "Hosmer-Lemeshow P").is_none());
    assert!(tab_by_id(&res_f, "glm-logistic-params").footnotes.iter().any(|f| f.contains("fewer than 3 distinct fitted-probability groups")));

    // 2. Multi-level tied groups (4 levels x 10 obs each = 40 obs): completely row-order invariant!
    let mut rows_4lvl = Vec::new();
    for x_lvl in [0.0, 1.0, 2.0, 3.0] {
        for k in 0..10 {
            let yv = if (k as f64) < (2.0 + 2.0 * x_lvl) { 1.0 } else { 0.0 };
            rows_4lvl.push((yv, x_lvl));
        }
    }
    let mut rows_4lvl_rev = rows_4lvl.clone();
    rows_4lvl_rev.reverse();

    let x_a = DenseMatrix::from_row_major(40, 2, rows_4lvl.iter().flat_map(|&(_, x)| [1.0, x]).collect()).unwrap();
    let y_a: Vec<f64> = rows_4lvl.iter().map(|&(y, _)| y).collect();
    let x_b = DenseMatrix::from_row_major(40, 2, rows_4lvl_rev.iter().flat_map(|&(_, x)| [1.0, x]).collect()).unwrap();
    let y_b: Vec<f64> = rows_4lvl_rev.iter().map(|&(y, _)| y).collect();
    let names = [String::from("Intercept"), String::from("Level")];
    let fit_a = multiple_logistic_regression(&x_a, &y_a, &names, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let fit_b = multiple_logistic_regression(&x_b, &y_b, &names, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(fit_a.hosmer_lemeshow_df, 2.0, 1e-12, "4 tied groups -> 4 bins => df = 2");
    assert_close(fit_a.hosmer_lemeshow_chi2, fit_b.hosmer_lemeshow_chi2, 1e-12, "HL chi2 row-order invariant");
    assert_close(fit_a.hosmer_lemeshow_p, fit_b.hosmer_lemeshow_p, 1e-12, "HL P row-order invariant");
}

// ============================================================================
// #87, #91, #113: Multiple Imputation pooling, 1-var PMM/ABB & parameter bounds
// ============================================================================

#[test]
fn audit_87_and_113_rubin_zero_variance_and_mi_parameter_validation() {
    // #87 direct: zero total variance => NaN t, P, CI.
    let pooled = rubin_pool_estimates("Const", &[5.0, 5.0, 5.0], &[0.0, 0.0, 0.0], Some(9.0), ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(pooled.estimate, 5.0, 1e-12, "zero variance estimate");
    assert_close(pooled.total_variance, 0.0, 0.0, "zero total variance");
    assert_close(pooled.pooled_se, 0.0, 0.0, "zero pooled SE");
    assert!(pooled.t_statistic.is_nan());
    assert!(pooled.p_value.is_nan());
    assert!(pooled.ci.0.is_nan() && pooled.ci.1.is_nan());

    // #87 end-to-end on constant column with missing values => Empty CI cell (never Open or (5, 5)).
    let mut mv = MultipleVariablesTable::new(1, 6).unwrap();
    mv.variables[0].title = String::from("Const");
    for r in 0..6 {
        mv.variables[0].cells[r] = if r == 2 || r == 4 { Cell::empty() } else { Cell::number(5.0) };
    }
    let sheet = DataSheet::new(SheetId::new("mv-cmi"), "ConstMI", TableArchetype::MultipleVariables(mv)).unwrap();
    let res = run_analysis(
        &sheet,
        &spec_for(
            "r-cmi",
            "mv-cmi",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 4,
                max_iters: 3,
                seed: 1,
                response_column: None,
                predictor_columns: Vec::new(),
            }),
        ),
    )
    .unwrap();
    let means_tab = tab_by_id(&res, "mi-pooled-means");
    assert!(matches!(means_tab.rows[0][5], ResultCell::Empty), "zero-variance MI pooled mean CI must be Empty");

    // #113: reject m outside 2..=50 and max_iters outside 1..=100 (both direct and run_analysis).
    let cols = vec![vec![Some(1.0), Some(2.0), Some(3.0), None, Some(5.0)]];
    let labels = vec![String::from("X")];
    for bad_m in [0, 1, 51, 200] {
        let cfg = MultipleImputationConfig { imputations_m: bad_m, max_iters: 5, ..Default::default() };
        assert!(matches!(mice_predictive_mean_matching(&labels, &cols, &cfg), Err(StatsError::InvalidParameter { name: "imputations_m", .. })));
    }
    for bad_it in [0, 101, 500] {
        let cfg = MultipleImputationConfig { imputations_m: 5, max_iters: bad_it, ..Default::default() };
        assert!(matches!(mice_predictive_mean_matching(&labels, &cols, &cfg), Err(StatsError::InvalidParameter { name: "max_iters", .. })));
    }

    let bad_spec = spec_for(
        "r-bad-mi",
        "mv-cmi",
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 1,
            max_iters: 5,
            seed: 1,
            response_column: None,
            predictor_columns: Vec::new(),
        }),
    );
    assert!(matches!(run_analysis(&sheet, &bad_spec), Err(StatsError::InvalidParameter { name: "imputations_m", .. })));
    let bad_spec_it = spec_for(
        "r-bad-mi-it",
        "mv-cmi",
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 0,
            seed: 1,
            response_column: None,
            predictor_columns: Vec::new(),
        }),
    );
    assert!(matches!(run_analysis(&sheet, &bad_spec_it), Err(StatsError::InvalidParameter { name: "max_iters", .. })));
}

#[test]
fn audit_91_single_variable_mi_intercept_bayesian_bootstrap_coverage() {
    // 50% MCAR missingness on a single Gaussian column (n = 40, n_obs = 20, m = 10):
    // Approximate Bayesian Bootstrap over all observed donors must give ~95% Rubin CI coverage.
    let mut data_rng = DeterministicRng::from_seed(0x91_CAFE);
    let true_mean = 10.0;
    let true_sd = 3.0;
    let trials = 250_usize;
    let mut covered = 0_usize;

    for trial in 0..trials {
        let mut col = Vec::with_capacity(40);
        for r in 0..40 {
            if r < 20 {
                col.push(Some(true_mean + true_sd * data_rng.standard_normal()));
            } else {
                col.push(None);
            }
        }
        let cfg = MultipleImputationConfig {
            imputations_m: 10,
            max_iters: 1,
            seed: 5000 + trial as u64,
            response_col: None,
            predictor_cols: Vec::new(),
            confidence: ConfidenceLevel::DEFAULT_95,
        };
        let res = mice_predictive_mean_matching(&[String::from("Y")], &[col], &cfg).unwrap();
        let (lo, hi) = res.pooled_means[0].ci;
        if lo <= true_mean && true_mean <= hi {
            covered += 1;
        }
    }
    let coverage = (covered as f64) / (trials as f64);
    assert!((0.91..=0.985).contains(&coverage), "single-variable MI coverage = {coverage:.3} (covered {covered}/{trials}), expected ~0.95");
}

// ============================================================================
// #92: k-means distinct point cap, empty cluster repair, smaller-k tie-break
// ============================================================================

#[test]
fn audit_92_kmeans_distinct_cap_empty_repair_and_tie_break() {
    // 1. 8 points occupying only 2 distinct locations (0,0)x4 and (5,5)x4 => k = 3 rejected cleanly.
    let dup_rows =
        vec![vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.0], vec![5.0, 5.0], vec![5.0, 5.0], vec![5.0, 5.0], vec![5.0, 5.0]];
    for k_bad in [3, 4] {
        assert!(matches!(kmeans_pp(&dup_rows, k_bad, 7), Err(StatsError::InvalidParameter { name: "k_means_k", .. })));
        assert!(matches!(
            kmeans_pp_with_distance(&dup_rows, k_bad, ClusterDistance::Manhattan, 7),
            Err(StatsError::InvalidParameter { name: "k_means_k", .. })
        ));
    }
    let km_ok = kmeans_pp(&dup_rows, 2, 7).unwrap();
    assert_eq!(km_ok.k, 2);
    assert!(km_ok.assignments.contains(&0) && km_ok.assignments.contains(&1));

    // 2. Tied silhouette automatic selection picks smaller k = 2 rather than larger k.
    let mut mv = MultipleVariablesTable::new(2, 8).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[1].title = String::from("Y");
    for (r, pt) in dup_rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(pt[0]);
        mv.variables[1].cells[r] = Cell::number(pt[1]);
    }
    let dup_sheet = DataSheet::new(SheetId::new("mv-km-dup"), "KmDup", TableArchetype::MultipleVariables(mv)).unwrap();
    let res_dup = run_analysis(
        &dup_sheet,
        &spec_for(
            "r-km-dup",
            "mv-km-dup",
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: None,
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Ward,
                k_sweep_max: None,
            }),
        ),
    )
    .unwrap();
    assert_close(scalar_opt(&res_dup, "Clusters (k)").unwrap(), 2.0, 0.0, "only k=2 valid for 2 distinct points");

    // 3. Two separated discrete levels with 6 distinct points (tight epsilon variations within each cluster):
    // If silhouette ties across k=2 and k>=3, tie-break must pick smaller k.
    let tight_pts = [vec![0.0, 0.0], vec![0.0, 0.0], vec![0.0, 0.0], vec![10.0, 0.0], vec![10.0, 0.0], vec![10.0, 0.0]];
    let mut mv_tight = MultipleVariablesTable::new(2, 6).unwrap();
    mv_tight.variables[0].title = String::from("X");
    mv_tight.variables[1].title = String::from("Y");
    for (r, pt) in tight_pts.iter().enumerate() {
        mv_tight.variables[0].cells[r] = Cell::number(pt[0]);
        mv_tight.variables[1].cells[r] = Cell::number(pt[1]);
    }
    let tight_sheet = DataSheet::new(SheetId::new("mv-km-tight"), "KmTight", TableArchetype::MultipleVariables(mv_tight)).unwrap();
    let res_tight = run_analysis(
        &tight_sheet,
        &spec_for(
            "r-km-tight",
            "mv-km-tight",
            AnalysisKind::Multivariate(MultivariateSpec::Clustering {
                k_means_k: None,
                distance: ClusterDistance::Euclidean,
                linkage: ClusterLinkage::Ward,
                k_sweep_max: None,
            }),
        ),
    )
    .unwrap();
    assert_close(scalar_opt(&res_tight, "Clusters (k)").unwrap(), 2.0, 0.0, "tied silhouette picks smallest k");

    // 4. Skewed points with k = n_distinct across metrics never produce an empty cluster.
    let skewed = vec![vec![0.0, 0.0], vec![0.01, 0.0], vec![1.0, 0.0], vec![100.0, 0.0], vec![1000.0, 0.0]];
    for dist in [ClusterDistance::Euclidean, ClusterDistance::Manhattan] {
        for seed in 0..8 {
            let km = kmeans_pp_with_distance(&skewed, 4, dist, seed).unwrap();
            let mut counts = vec![0_usize; 4];
            for &c in &km.assignments {
                counts[c] += 1;
            }
            assert!(counts.iter().all(|&cnt| cnt >= 1), "dist={dist:?}, seed={seed}: empty cluster in {counts:?}");
        }
    }
}

// ============================================================================
// #96: Missing reference level in GLM and Cox PH returns typed error
// ============================================================================

#[test]
fn audit_96_absent_reference_level_returns_typed_error_in_glm_and_cox() {
    let mut mv = MultipleVariablesTable::new(4, 8).unwrap();
    mv.variables[0].title = String::from("TimeOrY");
    mv.variables[1].title = String::from("Event");
    mv.variables[2].title = String::from("Dose");
    mv.variables[3].title = String::from("Arm");
    mv.variables[3].lock_role(VariableRole::Categorical);
    for r in 0..8 {
        mv.variables[0].cells[r] = Cell::number((r + 2) as f64);
        mv.variables[1].cells[r] = Cell::number(if r % 3 == 0 { 0.0 } else { 1.0 });
        mv.variables[2].cells[r] = Cell::number((r as f64) * 0.5);
        mv.variables[3].cells[r] = Cell::text(if r % 2 == 0 { "Control" } else { "Treated" });
    }
    let sheet = DataSheet::new(SheetId::new("mv-ref"), "RefErr", TableArchetype::MultipleVariables(mv)).unwrap();

    // Requested reference `"ctrl"` does not exist after recode to `"Control"`.
    let glm_err = run_analysis(
        &sheet,
        &spec_for(
            "r-glm-ref",
            "mv-ref",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![2, 3],
                reference_levels: vec![(3, String::from("ctrl"))],
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(glm_err, StatsError::InvalidParameter { name: "reference_levels", .. }), "got {glm_err:?}");

    let cox_err = run_analysis(
        &sheet,
        &spec_for(
            "r-cox-ref",
            "mv-ref",
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2, 3],
                reference_levels: vec![(3, String::from("ctrl"))],
                tie_method: CoxTieMethod::Breslow,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(cox_err, StatsError::InvalidParameter { name: "reference_levels", .. }), "got {cox_err:?}");

    // Out-of-bounds reference column index returns MissingColumn cleanly.
    let glm_oob = run_analysis(
        &sheet,
        &spec_for(
            "r-glm-oob",
            "mv-ref",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 0,
                predictor_columns: vec![2, 3],
                reference_levels: vec![(9, String::from("Control"))],
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(glm_oob, StatsError::MissingColumn { index: 9 }), "got {glm_oob:?}");
}

// ============================================================================
// #101 & #103: Poisson large counts (unclamped eta) & non-integer rejection
// ============================================================================

#[test]
fn audit_101_and_103_poisson_unclamped_large_counts_and_integer_validation() {
    // #101: x = 0..=3, y = [1e11, 2.1e11, 3.9e11, 8e11] -> eta in ~[25.3, 27.4] > 25!
    // Exact score condition: sum x_i * exp(b1 * x_i) / sum exp(b1 * x_i) == sum(x * y) / sum(y) = 2.26.
    // Let r = exp(b1): 0.74 r^3 - 0.26 r^2 - 1.26 r - 2.26 = 0.
    let mut r = 2.0_f64;
    for _ in 0..30 {
        let f = 0.74 * r.powi(3) - 0.26 * r.powi(2) - 1.26 * r - 2.26;
        let df = 2.22 * r.powi(2) - 0.52 * r - 1.26;
        r -= f / df;
    }
    let exact_b1 = r.ln();
    let exact_b0 = (1.5e12_f64 / (1.0 + r + r.powi(2) + r.powi(3))).ln();

    let x_mat = DenseMatrix::from_row_major(4, 2, vec![1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 3.0]).unwrap();
    let y_large = [1.0e11, 2.1e11, 3.9e11, 8.0e11];
    let p_res =
        multiple_poisson_regression(&x_mat, &y_large, &[String::from("Intercept"), String::from("x")], false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(p_res.converged, "1e11 Poisson IRLS must converge without hitting [-25, 25] clamp");
    assert_close(p_res.terms[0].beta, exact_b0, 1e-8, "Poisson 1e11 b0 matches R glm");
    assert_close(p_res.terms[1].beta, exact_b1, 1e-8, "Poisson 1e11 b1 matches R glm");
    assert!(p_res.terms[0].beta > 25.0, "linear predictor exceeded legacy 25 clamp");

    // #103: non-integer Poisson counts must fail with InvalidParameter("poisson_y").
    let x_non_int = DenseMatrix::from_row_major(5, 2, (0..5).flat_map(|i| [1.0, i as f64]).collect()).unwrap();
    let err_frac = multiple_poisson_regression(
        &x_non_int,
        &[1.0, 2.5, 4.0, 6.0, 9.0],
        &[String::from("Intercept"), String::from("x")],
        false,
        ConfidenceLevel::DEFAULT_95,
    )
    .unwrap_err();
    assert!(matches!(err_frac, StatsError::InvalidParameter { name: "poisson_y", .. }), "got {err_frac:?}");

    // Also via run_analysis
    let mut mv_frac = MultipleVariablesTable::new(2, 5).unwrap();
    mv_frac.variables[0].title = String::from("Rate");
    mv_frac.variables[1].title = String::from("X");
    for (r, &yv) in [1.0, 2.5, 4.0, 6.0, 9.0].iter().enumerate() {
        mv_frac.variables[0].cells[r] = Cell::number(yv);
        mv_frac.variables[1].cells[r] = Cell::number(r as f64);
    }
    let frac_sheet = DataSheet::new(SheetId::new("mv-pfrac"), "PFrac", TableArchetype::MultipleVariables(mv_frac)).unwrap();
    let err_sheet = run_analysis(
        &frac_sheet,
        &spec_for(
            "r-pfrac",
            "mv-pfrac",
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Poisson,
                response_column: 0,
                predictor_columns: vec![1],
                reference_levels: Vec::new(),
                compute_vif: false,
            }),
        ),
    )
    .unwrap_err();
    assert!(matches!(err_sheet, StatsError::InvalidParameter { name: "poisson_y", .. }), "got {err_sheet:?}");

    // Non-convergent boundary-separated Poisson via run_analysis must surface converged == false.
    let x_sep_p = DenseMatrix::from_row_major(6, 2, (0..6).flat_map(|i| [1.0, i as f64]).collect()).unwrap();
    let y_sep_p = [0.0, 0.0, 0.0, 1.0, 3.0, 9.0];
    let p_sep = multiple_poisson_regression(&x_sep_p, &y_sep_p, &[String::from("Intercept"), String::from("x")], false, ConfidenceLevel::DEFAULT_95);
    if let Ok(fit_sep) = p_sep
        && !fit_sep.converged
    {
        let mut mv_sep_p = MultipleVariablesTable::new(2, 6).unwrap();
        mv_sep_p.variables[0].title = String::from("Count");
        mv_sep_p.variables[1].title = String::from("X");
        for (i, &yc) in y_sep_p.iter().enumerate() {
            mv_sep_p.variables[0].cells[i] = Cell::number(yc);
            mv_sep_p.variables[1].cells[i] = Cell::number(i as f64);
        }
        let sh = DataSheet::new(SheetId::new("mv-psep"), "PSep", TableArchetype::MultipleVariables(mv_sep_p)).unwrap();
        let rsh = run_analysis(
            &sh,
            &spec_for(
                "r-psep",
                "mv-psep",
                AnalysisKind::GeneralizedLinearModel(GlmSpec {
                    family: GlmFamily::Poisson,
                    response_column: 0,
                    predictor_columns: vec![1],
                    reference_levels: Vec::new(),
                    compute_vif: false,
                }),
            ),
        )
        .unwrap();
        assert_close(scalar_opt(&rsh, "Converged").unwrap(), 0.0, 0.0, "non-convergent Poisson Converged scalar == 0");
        assert!(rsh.checklist.iter().any(|c| c.assumption == "Poisson IRLS convergence" && c.status == ChecklistStatus::Warn));
        assert!(tab_by_id(&rsh, "glm-poisson-params").footnotes.iter().any(|f| f.contains("did not converge")));
    }
}

// ============================================================================
// #102 & #117: Passing-Bablok CUSUM H/sqrt(L + 1) calibration & residual rows
// ============================================================================

#[test]
fn audit_102_passing_bablok_cusum_null_rejection_calibration() {
    let mut rng = DeterministicRng::from_seed(0x102_BABC);
    for (n, trials, min_rate, max_rate) in [(30_usize, 400_usize, 0.025_f64, 0.085_f64), (100_usize, 250_usize, 0.025_f64, 0.085_f64)] {
        let mut rejected = 0_usize;
        for _ in 0..trials {
            let mut xs = Vec::with_capacity(n);
            let mut ys = Vec::with_capacity(n);
            for _ in 0..n {
                let x = 1.0 + 9.0 * rng.next_open_f64();
                let err_x = 0.25 * rng.standard_normal();
                let err_y = 0.25 * rng.standard_normal();
                xs.push(x + err_x);
                ys.push(1.5 + 1.2 * x + err_y);
            }
            let pb = passing_bablok_regression(&xs, &ys, ConfidenceLevel::DEFAULT_95).unwrap();
            if pb.cusum_p_value < 0.05 {
                rejected += 1;
            }
        }
        let rate = (rejected as f64) / (trials as f64);
        assert!(
            (min_rate..=max_rate).contains(&rate),
            "Passing-Bablok CUSUM n={n} null rejection rate = {rate:.3} ({rejected}/{trials}), expected ~0.05"
        );
    }
}

#[test]
fn audit_117_passing_bablok_residuals_preserve_source_row_indices() {
    let expected = vec![String::from("Row 1"), String::from("Row 3"), String::from("Row 4"), String::from("Row 6")];
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let ys = [2.1, 4.0, 6.2, 7.9, 10.1, 12.0];

    // ColumnTable
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 6).unwrap();
    for r in 0..6 {
        col.data_sets[0].cells[r][0] = if r == 4 { Cell::excluded_number(xs[r]) } else { Cell::number(xs[r]) };
        col.data_sets[1].cells[r][0] = if r == 1 { Cell::empty() } else { Cell::number(ys[r]) };
    }
    let sh_col = DataSheet::new(SheetId::new("pb-c"), "PbC", TableArchetype::Column(col)).unwrap();
    let res_col =
        run_analysis(&sh_col, &spec_for("r-pb-c", "pb-c", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }))).unwrap();
    assert_eq!(tab_by_id(&res_col, "pb-residuals").row_labels, expected);

    // MultipleVariablesTable
    let mut mv = MultipleVariablesTable::new(2, 6).unwrap();
    for r in 0..6 {
        mv.variables[0].cells[r] = if r == 4 { Cell::excluded_number(xs[r]) } else { Cell::number(xs[r]) };
        mv.variables[1].cells[r] = if r == 1 { Cell::empty() } else { Cell::number(ys[r]) };
    }
    let sh_mv = DataSheet::new(SheetId::new("pb-m"), "PbM", TableArchetype::MultipleVariables(mv)).unwrap();
    let res_mv =
        run_analysis(&sh_mv, &spec_for("r-pb-m", "pb-m", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }))).unwrap();
    assert_eq!(tab_by_id(&res_mv, "pb-residuals").row_labels, expected);

    // XyTable
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
    for r in 0..6 {
        xy.set_x_cell(r, if r == 4 { Cell::excluded_number(xs[r]) } else { Cell::number(xs[r]) }).unwrap();
        xy.data_sets[0].cells[r][0] = if r == 1 { Cell::empty() } else { Cell::number(ys[r]) };
    }
    let sh_xy = DataSheet::new(SheetId::new("pb-x"), "PbX", TableArchetype::Xy(xy)).unwrap();
    let res_xy =
        run_analysis(&sh_xy, &spec_for("r-pb-x", "pb-x", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 }))).unwrap();
    assert_eq!(tab_by_id(&res_xy, "pb-residuals").row_labels, expected);
}

// ============================================================================
// #120: XY GLM extracts all active replicate subcolumns
// ============================================================================

#[test]
fn audit_120_xy_glm_extracts_all_active_replicate_subcolumns() {
    let x_vals = [1.0, 2.0, 3.0, 4.0];
    let rep_lin = [[2.0, 2.4, 2.8], [4.1, 4.5, 4.9], [6.0, 6.4, 6.8], [8.2, 8.6, 9.0]];
    let rep_bin = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]];
    let rep_cnt = [[1.0, 2.0, 1.0], [3.0, 4.0, 3.0], [6.0, 8.0, 7.0], [14.0, 16.0, 15.0]];

    for (family, reps, resid_id) in [
        (GlmFamily::Linear, rep_lin, "glm-linear-residuals"),
        (GlmFamily::Logistic, rep_bin, "glm-logistic-predictions"),
        (GlmFamily::Poisson, rep_cnt, "glm-poisson-residuals"),
    ] {
        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 1, 4).unwrap();
        xy.x_title = String::from("Dose");
        xy.data_sets[0].title = String::from("Resp");
        let mut flat_x = Vec::new();
        let mut flat_y = Vec::new();
        for (r, &xv) in x_vals.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(xv)).unwrap();
            for s in 0..3 {
                // Exclude row 1 replicate 1 (0-based r=1, s=1) to verify mixed active/excluded replicate filtering.
                if r == 1 && s == 1 {
                    xy.data_sets[0].cells[r][s] = Cell::excluded_number(999.0);
                } else {
                    xy.data_sets[0].cells[r][s] = Cell::number(reps[r][s]);
                    flat_x.push(1.0);
                    flat_x.push(xv);
                    flat_y.push(reps[r][s]);
                }
            }
        }
        let sh = DataSheet::new(SheetId::new("xy-rep"), "XyRep", TableArchetype::Xy(xy)).unwrap();
        let res = run_analysis(
            &sh,
            &spec_for(
                "r-xy-rep",
                "xy-rep",
                AnalysisKind::GeneralizedLinearModel(GlmSpec {
                    family,
                    response_column: 0,
                    predictor_columns: vec![0],
                    reference_levels: Vec::new(),
                    compute_vif: false,
                }),
            ),
        )
        .unwrap();
        let resid_tab = tab_by_id(&res, resid_id);
        assert_eq!(resid_tab.rows.len(), 11, "{family:?} must extract all 11 active replicates across 3 subcolumns");
        assert_eq!(resid_tab.row_labels[0], "Row 1 (rep 1)");
        assert_eq!(resid_tab.row_labels[4], "Row 2 (rep 3)");

        let x_mat = DenseMatrix::from_row_major(11, 2, flat_x).unwrap();
        let names = [String::from("Intercept"), String::from("Dose")];
        let param_tab = match family {
            GlmFamily::Linear => {
                let oracle = multiple_linear_regression(&x_mat, &flat_y, &names, true, false, ConfidenceLevel::DEFAULT_95).unwrap();
                let pt = tab_by_id(&res, "glm-linear-params");
                assert_close(cell_num(&pt.rows[1][0]), oracle.terms[1].estimate, 1e-12, "XY replicate Linear slope");
                pt
            }
            GlmFamily::Logistic => {
                let oracle = multiple_logistic_regression(&x_mat, &flat_y, &names, false, ConfidenceLevel::DEFAULT_95).unwrap();
                let pt = tab_by_id(&res, "glm-logistic-params");
                assert_close(cell_num(&pt.rows[1][0]), oracle.terms[1].beta, 1e-10, "XY replicate Logistic slope");
                pt
            }
            GlmFamily::Poisson => {
                let oracle = multiple_poisson_regression(&x_mat, &flat_y, &names, false, ConfidenceLevel::DEFAULT_95).unwrap();
                let pt = tab_by_id(&res, "glm-poisson-params");
                assert_close(cell_num(&pt.rows[1][0]), oracle.terms[1].beta, 1e-10, "XY replicate Poisson slope");
                pt
            }
        };
        assert_eq!(param_tab.rows.len(), 2);
    }
}
