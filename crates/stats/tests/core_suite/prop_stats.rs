//! Property-based tests (`proptest`) verifying statistical invariances, bounds, equivariances,
//! and algebraic identities across `curvebook-stats`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, Cell, ClusterDistance, CoxTieMethod, DataSheet, GlmDispersionMode, GlmFamily, GlmSpec, KMeansAlgorithm, MultipleVariablesTable,
    RegressionCiMethod, SheetId, SurvivalAnalysisSpec, SurvivalOtherValuePolicy, SurvivalPowerMethod, TableArchetype,
};
use curvebook_math::sum::SummaryMoments;
use curvebook_math::{ConfidenceLevel, DenseMatrix};
use curvebook_stats::anova::ordinary_one_way_anova;
use curvebook_stats::categorical::fisher_exact_2x2;
use curvebook_stats::column::{normality_battery, one_sample_t_test, one_sample_wilcoxon, summarize_column};
use curvebook_stats::glm_ext::multiple_linear_regression_weighted;
use curvebook_stats::lognormal::{lognormal_one_way_ordinary, lognormal_welch_t_test, one_sample_ratio_t_test};
use curvebook_stats::multiplicity::{
    adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni, adjust_holm_sidak, adjust_sidak,
    adjust_two_stage_bky,
};
use curvebook_stats::multivariate::{consensus_optimal_k, pca_analysis, pearson_correlation, spearman_correlation};
use curvebook_stats::power::{PowerAnalysisRequest, solve_power};
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression};
use curvebook_stats::survival::{CoxSubject, cox_ph_regression};
use curvebook_stats::two_group::{mann_whitney_u, paired_t_test, unpaired_t_test};
use curvebook_testkit::assert_approx_eq;
use proptest::prelude::*;

use crate::common::{assert_all_sheet_numbers_finite, cell_ci_finite, cell_num, run_kind, tab_by_id};

fn non_constant_vec(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(-100.0_f64..100.0_f64, min_len..=max_len).prop_filter("non-constant finite sample", |v| {
        let min = v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (max - min) > 1e-3
    })
}

proptest! {
    #[test]
    fn prop_location_shift_and_scale_equivariance_sd_iqr(
        xs in non_constant_vec(5, 40),
        shift in -500.0_f64..500.0_f64,
        scale in prop::num::f64::NORMAL.prop_filter("nonzero bounded scale", |&s| s.abs() >= 0.05 && s.abs() <= 50.0),
    ) {
        let base = summarize_column("x", &xs, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
        let shifted_vec: Vec<f64> = xs.iter().map(|&x| x + shift).collect();
        let shifted = summarize_column("x+c", &shifted_vec, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();

        let base_sd = base.sd.unwrap();
        let shift_sd = shifted.sd.unwrap();
        assert_approx_eq(shift_sd, base_sd, 1e-9, 1e-9, "location-shift SD");
        assert_approx_eq(shifted.iqr, base.iqr, 1e-9, 1e-9, "location-shift IQR");

        let scaled_vec: Vec<f64> = xs.iter().map(|&x| x * scale).collect();
        let scaled = summarize_column("a*x", &scaled_vec, 0, ConfidenceLevel::DEFAULT_95, None).unwrap();
        assert_approx_eq(scaled.sd.unwrap(), scale.abs() * base_sd, 1e-9, 1e-9, "scale equivariant SD");
        assert_approx_eq(scaled.iqr, scale.abs() * base.iqr, 1e-9, 1e-9, "scale equivariant IQR");
    }

    #[test]
    fn prop_paired_t_matches_one_sample_t_on_differences(
        xs in prop::collection::vec(-50.0_f64..50.0_f64, 5..=30),
        ys in prop::collection::vec(-50.0_f64..50.0_f64, 5..=30),
    ) {
        let n = xs.len().min(ys.len());
        let a = &xs[..n];
        let b = &ys[..n];
        let diffs: Vec<f64> = a.iter().zip(b.iter()).map(|(&x, &y)| x - y).collect();
        let d_min = diffs.iter().copied().fold(f64::INFINITY, f64::min);
        let d_max = diffs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        prop_assume!(d_max - d_min > 1e-4);

        let paired = paired_t_test(a, b, true, ConfidenceLevel::DEFAULT_95).unwrap();
        let one_samp = one_sample_t_test(&diffs, 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();

        assert_approx_eq(paired.t_statistic, one_samp.t_statistic, 1e-12, 1e-12, "paired t == one-sample t");
        assert_approx_eq(paired.p_value, one_samp.p_value, 1e-12, 1e-12, "paired p == one-sample p");
        assert_approx_eq(paired.mean_of_differences, one_samp.discrepancy, 1e-12, 1e-12, "paired mean diff");
        assert!((0.0..=1.0).contains(&paired.p_value));
    }

    #[test]
    fn prop_mann_whitney_invariant_to_strictly_monotone_transform(
        a in prop::collection::vec(-4.0_f64..4.0_f64, 4..=14),
        b in prop::collection::vec(-4.0_f64..4.0_f64, 4..=14),
    ) {
        let mw_raw = mann_whitney_u(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
        // Use strictly increasing exp(x) over [-4, 4], which preserves strict order without overflow/ties.
        let ea: Vec<f64> = a.iter().map(|&x| x.exp()).collect();
        let eb: Vec<f64> = b.iter().map(|&x| x.exp()).collect();
        // Guard against any IEEE-754 rounding collapsing distinct nearby values under exp().
        let mut all_raw: Vec<f64> = a.iter().chain(b.iter()).copied().collect();
        all_raw.sort_by(f64::total_cmp);
        let has_near_tie = all_raw.windows(2).any(|w| (w[1] - w[0]).abs() < 1e-12);
        prop_assume!(!has_near_tie);

        let mw_exp = mann_whitney_u(&ea, &eb, true, ConfidenceLevel::DEFAULT_95).unwrap();
        assert_approx_eq(mw_raw.u_a, mw_exp.u_a, 1e-12, 1e-12, "Mann-Whitney U_a monotone invariance");
        assert_approx_eq(mw_raw.p_value, mw_exp.p_value, 1e-12, 1e-12, "Mann-Whitney p monotone invariance");
        assert!((0.0..=1.0).contains(&mw_raw.p_value));
    }

    #[test]
    fn prop_multiplicity_monotone_and_bounded(
        raw_p in prop::collection::vec(0.0_f64..=1.0_f64, 1..=30),
    ) {
        let adjusted_sets = [
            (adjust_bonferroni(&raw_p), true),
            (adjust_sidak(&raw_p), true),
            (adjust_holm_bonferroni(&raw_p), true),
            (adjust_holm_sidak(&raw_p), true),
            (adjust_benjamini_hochberg(&raw_p), true),
            (adjust_benjamini_yekutieli(&raw_p), true),
            (adjust_two_stage_bky(&raw_p, 0.05), false),
        ];
        for (adj, enforce_ge_raw) in &adjusted_sets {
            prop_assert_eq!(adj.len(), raw_p.len());
            for (&r, &a) in raw_p.iter().zip(adj.iter()) {
                prop_assert!((0.0..=1.0).contains(&a), "adjusted p out of [0,1]: {a}");
                if *enforce_ge_raw {
                    prop_assert!(a + 1e-15 >= r, "adjusted p {a} < raw p {r}");
                }
            }
            for i in 0..raw_p.len() {
                for j in 0..raw_p.len() {
                    if raw_p[i] <= raw_p[j] {
                        prop_assert!(adj[i] <= adj[j] + 1e-12, "monotonicity violated: p[i]={}<=p[j]={}, adj[i]={}>adj[j]={}", raw_p[i], raw_p[j], adj[i], adj[j]);
                    }
                }
            }
        }
    }

    #[test]
    fn prop_pvalues_always_valid_across_tests(
        g1 in non_constant_vec(4, 20),
        g2 in non_constant_vec(4, 20),
        g3 in non_constant_vec(4, 20),
        a00 in 0_u64..=40,
        a01 in 0_u64..=40,
        a10 in 0_u64..=40,
        a11 in 0_u64..=40,
    ) {
        let ut = unpaired_t_test(&g1, &g2, true, ConfidenceLevel::DEFAULT_95).unwrap();
        prop_assert!((0.0..=1.0).contains(&ut.p_value));

        let aov = ordinary_one_way_anova(&[("G1", &g1), ("G2", &g2), ("G3", &g3)], ConfidenceLevel::DEFAULT_95).unwrap();
        prop_assert!((0.0..=1.0).contains(&aov.p_value));
        prop_assert!((0.0..=1.0).contains(&aov.r_squared));

        let n = g1.len().min(g2.len());
        if n >= 3 {
            let pr = pearson_correlation(&g1[..n], &g2[..n], ConfidenceLevel::DEFAULT_95);
            if let Ok(c) = pr {
                prop_assert!((0.0..=1.0).contains(&c.p_value));
                prop_assert!((-1.0 - 1e-12..=1.0 + 1e-12).contains(&c.r));
            }
            let sr = spearman_correlation(&g1[..n], &g2[..n]);
            if let Ok(c) = sr {
                prop_assert!((0.0..=1.0).contains(&c.p_value));
            }
        }

        let wil = one_sample_wilcoxon(&g1, 0.0, true, ConfidenceLevel::DEFAULT_95).unwrap();
        prop_assert!((0.0..=1.0).contains(&wil.p_value));

        let fp = fisher_exact_2x2(a00, a01, a10, a11).unwrap();
        prop_assert!((0.0..=1.0).contains(&fp));
    }

    #[test]
    fn prop_ols_r_squared_in_unit_interval_and_pca_variance_trace(
        rows in prop::collection::vec(prop::collection::vec(-25.0_f64..25.0_f64, 3..=3), 8..=24),
        y in prop::collection::vec(-50.0_f64..50.0_f64, 8..=24),
    ) {
        let n = rows.len().min(y.len());
        let p_pred = 2; // x1, x2 (+ intercept)
        let mut x_design = Vec::with_capacity(n * (p_pred + 1));
        for row in rows.iter().take(n) {
            x_design.push(1.0);
            x_design.push(row[0]);
            x_design.push(row[1]);
        }
        let x_mat = DenseMatrix::from_row_major(n, p_pred + 1, x_design).unwrap();
        let names = vec![String::from("Intercept"), String::from("X1"), String::from("X2")];
        if let Ok(fit) = multiple_linear_regression(&x_mat, &y[..n], &names, true, false, ConfidenceLevel::DEFAULT_95) {
            prop_assert!((0.0..=1.0).contains(&fit.r_squared), "R^2={} out of [0,1]", fit.r_squared);
        }

        // PCA on n x 3 matrix: sum of eigenvalues == trace(Cov) for unstandardized, and == 3 for standardized.
        let mut pca_data = Vec::with_capacity(n * 3);
        let mut col_vars = [0.0_f64; 3];
        for c in 0..3 {
            let col: Vec<f64> = rows.iter().take(n).map(|r| r[c]).collect();
            let var = SummaryMoments::from_slice(&col).unwrap().sample_variance().unwrap();
            prop_assume!(var > 1e-6);
            col_vars[c] = var;
        }
        for row in rows.iter().take(n) {
            pca_data.extend_from_slice(row);
        }
        let dm = DenseMatrix::from_row_major(n, 3, pca_data).unwrap();
        let labels = vec![String::from("V1"), String::from("V2"), String::from("V3")];

        let pca_cov = pca_analysis(&dm, &labels, false, None).unwrap();
        let ev_sum_cov: f64 = pca_cov.eigenvalues.iter().sum();
        let trace_cov: f64 = col_vars.iter().sum();
        assert_approx_eq(ev_sum_cov, trace_cov, 1e-9, 1e-9, "PCA unstandardized eigenvalue sum == trace(Cov)");

        let pca_cor = pca_analysis(&dm, &labels, true, None).unwrap();
        let ev_sum_cor: f64 = pca_cor.eigenvalues.iter().sum();
        assert_approx_eq(ev_sum_cor, 3.0, 1e-9, 1e-9, "PCA standardized eigenvalue sum == p");
    }

    #[test]
    fn prop_lognormal_edge_guards_and_row_permutation_invariance(
        g1 in prop::collection::vec(0.2_f64..100.0_f64, 4..=12),
        g2 in prop::collection::vec(0.2_f64..100.0_f64, 4..=12),
        g3 in prop::collection::vec(0.2_f64..100.0_f64, 4..=12),
        nonpos in -25.0_f64..=0.0_f64,
    ) {
        // Non-positive value must be rejected cleanly across all lognormal adapters and tracked in normality_battery.
        let mut bad = g1.clone();
        bad[0] = nonpos;
        prop_assert!(one_sample_ratio_t_test(&bad, 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(lognormal_welch_t_test(&bad, &g2, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(lognormal_one_way_ordinary(&[("G1", &bad), ("G2", &g2)], ConfidenceLevel::DEFAULT_95).is_err());
        let bat = normality_battery(&bad, 0.05).unwrap();
        prop_assert!(bat.n_impossible_nonpositive >= 1);
        prop_assert!(bat.lognormal_shapiro_wilk.is_none());

        // n = 0, 1, zero-variance, and NaN/Inf must return clean Err without panicking.
        prop_assert!(one_sample_ratio_t_test(&[], 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(one_sample_ratio_t_test(&[g1[0]], 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(one_sample_ratio_t_test(&[5.0, 5.0, 5.0], 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(one_sample_ratio_t_test(&[1.0, f64::NAN, 3.0], 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(one_sample_ratio_t_test(&[1.0, f64::INFINITY, 3.0], 1.0, true, ConfidenceLevel::DEFAULT_95).is_err());

        // Row permutation invariance for lognormal one-way ANOVA and lognormal Welch t-test.
        let mut g1_rev = g1.clone();
        g1_rev.reverse();
        let mut g2_rev = g2.clone();
        g2_rev.reverse();
        let mut g3_rev = g3.clone();
        g3_rev.reverse();
        if let (Ok(w1), Ok(w2)) = (
            lognormal_welch_t_test(&g1, &g2, true, ConfidenceLevel::DEFAULT_95),
            lognormal_welch_t_test(&g1_rev, &g2_rev, true, ConfidenceLevel::DEFAULT_95),
        ) {
            assert_approx_eq(w1.t_statistic, w2.t_statistic, 1e-11, 1e-11, "ln Welch permutation t");
            assert_approx_eq(w1.geometric_mean_ratio.unwrap(), w2.geometric_mean_ratio.unwrap(), 1e-11, 1e-11, "ln Welch permutation GMR");
        }
        if let (Ok(a1), Ok(a2)) = (
            lognormal_one_way_ordinary(&[("A", &g1), ("B", &g2), ("C", &g3)], ConfidenceLevel::DEFAULT_95),
            lognormal_one_way_ordinary(&[("A", &g1_rev), ("B", &g2_rev), ("C", &g3_rev)], ConfidenceLevel::DEFAULT_95),
        ) {
            assert_approx_eq(a1.log_anova.f_statistic, a2.log_anova.f_statistic, 1e-10, 1e-10, "ln ANOVA permutation F");
            assert_approx_eq(a1.log_anova.p_value, a2.log_anova.p_value, 1e-10, 1e-10, "ln ANOVA permutation p");
        }
    }

    #[test]
    fn prop_consensus_kmeans_collinear_skipindex_and_survival_power_arm_symmetry(
        xs in prop::collection::vec(-20.0_f64..20.0_f64, 6..=14),
        hr in 0.25_f64..0.85_f64,
        k in 0.3_f64..3.0_f64,
    ) {
        // Collinear 2D points (y = 3x + 1): Rubin/Friedman/Marriot must cleanly SkipIndex without NaN/Inf/panic.
        let mut unique_x = xs;
        unique_x.sort_by(f64::total_cmp);
        unique_x.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        if unique_x.len() >= 6 {
            let rows: Vec<Vec<f64>> = unique_x.iter().map(|&x| vec![x, 3.0 * x + 1.0]).collect();
            let cons = consensus_optimal_k(&rows, 2, 3, ClusterDistance::Euclidean, KMeansAlgorithm::HartiganWong, 13).unwrap();
            prop_assert_eq!(cons.index_votes.len(), 17);
            prop_assert!((2..=3).contains(&cons.optimal_k));
            for m in &cons.metrics_by_k {
                for v in [
                    m.trace_w,
                    m.ball_hall,
                    m.calinski_harabasz,
                    m.c_index,
                    m.davies_bouldin,
                    m.dunn,
                    m.rubin,
                    m.friedman,
                    m.marriot,
                    m.silhouette,
                ]
                .into_iter()
                .flatten()
                {
                    prop_assert!(v.is_finite(), "validity index emitted non-finite value {v}");
                }
            }
        }

        // Survival power arm-relabeling invariance: (HR, k) <-> (1/HR, 1/k) yields identical required events.
        for method in [SurvivalPowerMethod::Schoenfeld, SurvivalPowerMethod::Freedman] {
            let r1 = solve_power(
                &PowerAnalysisRequest::log_rank_survival(hr, 0.05, 0.80)
                    .with_survival_method(method)
                    .with_allocation_ratio(k),
            )
            .unwrap();
            let r2 = solve_power(
                &PowerAnalysisRequest::log_rank_survival(1.0 / hr, 0.05, 0.80)
                    .with_survival_method(method)
                    .with_allocation_ratio(1.0 / k),
            )
            .unwrap();
            assert_approx_eq(r1.required_events.unwrap(), r2.required_events.unwrap(), 1e-10, 1e-10, "survival arm symmetry");
            prop_assert!(r1.achieved_power >= 0.80 - 1e-9);
        }
    }

    #[test]
    fn prop_r23_5_glm_cox_edge_guards_collinearity_separation_oos_and_nonfinite(
        xs in prop::collection::vec(-5.0_f64..5.0_f64, 8..=12),
        slope in -1.5_f64..1.5_f64,
        shift in -2.0_f64..2.0_f64,
    ) {
        // 1. Tiny-sample edge guards (n = 0, 1) and NaN/Inf guards must return Err without panicking.
        let empty_x = DenseMatrix::from_row_major(0, 2, vec![]).unwrap();
        let one_x = DenseMatrix::from_row_major(1, 2, vec![1.0, 2.0]).unwrap();
        let names = vec![String::from("Intercept"), String::from("x1")];
        prop_assert!(multiple_linear_regression_weighted(&empty_x, &[], None, &names, true, false, GlmDispersionMode::Canonical, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_linear_regression_weighted(&one_x, &[1.0], None, &names, true, false, GlmDispersionMode::Canonical, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_logistic_regression(&one_x, &[1.0], &names, false, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_poisson_regression(&one_x, &[2.0], &names, false, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(cox_ph_regression(&[], &[String::from("x1")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).is_err());
        let one_cox = [CoxSubject { time: 2.0, event: true, x: vec![1.0] }];
        prop_assert!(cox_ph_regression(&one_cox, &[String::from("x1")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).is_err());

        // Non-finite inputs (NaN / Inf) must return Err cleanly across all models.
        let nan_x = DenseMatrix::from_row_major(4, 2, vec![1.0, 1.0, 1.0, f64::NAN, 1.0, 3.0, 1.0, 4.0]).unwrap();
        let inf_y = vec![1.0, f64::INFINITY, 2.0, 3.0];
        prop_assert!(multiple_linear_regression_weighted(&nan_x, &[1.0, 2.0, 3.0, 4.0], None, &names, true, false, GlmDispersionMode::Canonical, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_linear_regression_weighted(&DenseMatrix::from_row_major(4, 2, vec![1.0, 1.0, 1.0, 2.0, 1.0, 3.0, 1.0, 4.0]).unwrap(), &inf_y, None, &names, true, false, GlmDispersionMode::Canonical, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_logistic_regression(&nan_x, &[0.0, 1.0, 0.0, 1.0], &names, false, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_poisson_regression(&nan_x, &[1.0, 2.0, 3.0, 4.0], &names, false, ConfidenceLevel::DEFAULT_95).is_err());
        let nan_cox = vec![
            CoxSubject { time: 1.0, event: true, x: vec![1.0] },
            CoxSubject { time: f64::NAN, event: true, x: vec![2.0] },
            CoxSubject { time: 3.0, event: false, x: vec![3.0] },
            CoxSubject { time: 4.0, event: true, x: vec![4.0] },
        ];
        prop_assert!(cox_ph_regression(&nan_cox, &[String::from("x1")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).is_err());

        // Deduplicate xs so we have non-degenerate x1 values for collinearity, separation, Cox ties, and OOS checks.
        let mut x_sorted = xs;
        x_sorted.sort_by(f64::total_cmp);
        x_sorted.dedup_by(|a, b| (*a - *b).abs() < 0.05);
        if x_sorted.len() < 8 {
            return Ok(());
        }
        let n = x_sorted.len();

        // 2. Collinear predictors (x2 = 2 * x1 + 1):
        //    - Linear GLM marks aliased column and stays finite.
        //    - Logistic & Poisson reject singular design matrices cleanly.
        //    - Cox PH handles collinearity without panicking (non-converged or Err, never NaN/Inf).
        let mut col_flat = Vec::with_capacity(n * 3);
        let mut y_lin = Vec::with_capacity(n);
        let mut y_bin = Vec::with_capacity(n);
        let mut y_cnt = Vec::with_capacity(n);
        for (i, &x) in x_sorted.iter().enumerate() {
            col_flat.extend_from_slice(&[1.0, x, 2.0 * x + 1.0]);
            y_lin.push(shift + slope * x + ((i % 3) as f64 - 1.0) * 0.25);
            y_bin.push(if i % 2 == 0 { 0.0 } else { 1.0 });
            y_cnt.push(((i % 4) + 1) as f64);
        }
        let col_x = DenseMatrix::from_row_major(n, 3, col_flat).unwrap();
        let col_names = vec![String::from("Intercept"), String::from("x1"), String::from("x2")];
        let lin_col = multiple_linear_regression_weighted(
            &col_x,
            &y_lin,
            None,
            &col_names,
            true,
            false,
            GlmDispersionMode::Canonical,
            ConfidenceLevel::DEFAULT_95,
        )
        .unwrap();
        prop_assert!(lin_col.core.aliased_columns.iter().any(|&a| a), "collinear predictor must be aliased");
        prop_assert!(lin_col.core.rss.is_finite());
        prop_assert!(multiple_logistic_regression(&col_x, &y_bin, &col_names, false, ConfidenceLevel::DEFAULT_95).is_err());
        prop_assert!(multiple_poisson_regression(&col_x, &y_cnt, &col_names, false, ConfidenceLevel::DEFAULT_95).is_err());

        let cox_col_subjs: Vec<CoxSubject> = x_sorted
            .iter()
            .enumerate()
            .map(|(i, &x)| CoxSubject { time: (i + 1) as f64, event: i % 2 == 0, x: vec![x, 2.0 * x + 1.0] })
            .collect();
        if let Ok(res_col_cox) = cox_ph_regression(&cox_col_subjs, &[String::from("x1"), String::from("x2")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95) {
            prop_assert!(!res_col_cox.converged);
            prop_assert!(res_col_cox.likelihood_ratio_p.is_finite());
        }

        // 3. Cox PH zero-events (Err) and all-tied event times (finite ResultSheet).
        let zero_ev_subjs: Vec<CoxSubject> = x_sorted
            .iter()
            .enumerate()
            .map(|(i, &x)| CoxSubject { time: (i + 1) as f64, event: false, x: vec![x] })
            .collect();
        prop_assert!(cox_ph_regression(&zero_ev_subjs, &[String::from("x1")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).is_err());

        let tied_subjs: Vec<CoxSubject> = x_sorted
            .iter()
            .enumerate()
            .map(|(i, &x)| CoxSubject { time: 5.0, event: i % 2 == 0, x: vec![x] })
            .collect();
        if let Ok(tied_res) = cox_ph_regression(&tied_subjs, &[String::from("x1")], CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95) {
            prop_assert!(tied_res.log_lik_model.is_finite());
            prop_assert!(tied_res.covariates[0].beta.is_finite());
        }

        // 4. Complete separation in Logistic GLM + ProfileLikelihood + diagnostics + OOS row:
        //    sheet must stay completely finite with no panics.
        let mut mv_sep = MultipleVariablesTable::new(2, n + 1).unwrap();
        mv_sep.variables[0].title = String::from("x1");
        mv_sep.variables[1].title = String::from("y");
        for (i, &x) in x_sorted.iter().enumerate() {
            mv_sep.variables[0].cells[i] = Cell::number(x);
            mv_sep.variables[1].cells[i] = Cell::number(if i < n / 2 { 0.0 } else { 1.0 });
        }
        mv_sep.variables[0].cells[n] = Cell::number(0.5 * (x_sorted[0] + x_sorted[n - 1]));
        mv_sep.variables[1].cells[n] = Cell::empty();
        let sep_sheet = DataSheet::new(SheetId::new("prop-sep"), "prop-sep", TableArchetype::MultipleVariables(mv_sep)).unwrap();
        let sep_res = run_kind(
            &sep_sheet,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 1,
                predictor_columns: vec![0],
                ci_method: RegressionCiMethod::ProfileLikelihood,
                emit_influence_diagnostics: true,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_all_sheet_numbers_finite(&sep_res, "prop_sep_logistic");

        // 5. Blank-Y out-of-sample predictions across Linear, Logistic, and Poisson GLMs + custom survival status codes.
        let mut mv_oos = MultipleVariablesTable::new(4, n + 1).unwrap();
        mv_oos.variables[0].title = String::from("x1");
        mv_oos.variables[1].title = String::from("y_lin");
        mv_oos.variables[2].title = String::from("y_bin");
        mv_oos.variables[3].title = String::from("y_cnt");
        for (i, &x) in x_sorted.iter().enumerate() {
            mv_oos.variables[0].cells[i] = Cell::number(x);
            mv_oos.variables[1].cells[i] = Cell::number(y_lin[i]);
            mv_oos.variables[2].cells[i] = Cell::number(y_bin[i]);
            mv_oos.variables[3].cells[i] = Cell::number(y_cnt[i]);
        }
        mv_oos.variables[0].cells[n] = Cell::number(0.5 * (x_sorted[0] + x_sorted[n - 1]));
        mv_oos.variables[1].cells[n] = Cell::empty();
        mv_oos.variables[2].cells[n] = Cell::empty();
        mv_oos.variables[3].cells[n] = Cell::empty();
        let oos_sheet = DataSheet::new(SheetId::new("prop-oos"), "prop-oos", TableArchetype::MultipleVariables(mv_oos)).unwrap();

        let lin_oos_res = run_kind(
            &oos_sheet,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Linear,
                response_column: 1,
                predictor_columns: vec![0],
                emit_influence_diagnostics: true,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_all_sheet_numbers_finite(&lin_oos_res, "prop_lin_oos");
        let lin_oos_tab = tab_by_id(&lin_oos_res, "glm-oos-predictions");
        prop_assert_eq!(lin_oos_tab.rows.len(), 1);
        let fit = cell_num(&lin_oos_tab.rows[0][0]);
        let (ci_lo, ci_hi) = cell_ci_finite(&lin_oos_tab.rows[0][2]);
        let (pi_lo, pi_hi) = cell_ci_finite(&lin_oos_tab.rows[0][3]);
        prop_assert!(pi_lo <= ci_lo + 1e-10 && ci_lo <= fit + 1e-10 && fit <= ci_hi + 1e-10 && ci_hi <= pi_hi + 1e-10);

        let log_oos_res = run_kind(
            &oos_sheet,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Logistic,
                response_column: 2,
                predictor_columns: vec![0],
                ..Default::default()
            }),
        )
        .unwrap();
        assert_all_sheet_numbers_finite(&log_oos_res, "prop_log_oos");
        let log_oos_tab = tab_by_id(&log_oos_res, "glm-oos-predictions");
        let prob = cell_num(&log_oos_tab.rows[0][2]);
        let (p_lo, p_hi) = cell_ci_finite(&log_oos_tab.rows[0][3]);
        prop_assert!(p_lo >= 0.0 && p_lo <= prob + 1e-10 && prob <= p_hi + 1e-10 && p_hi <= 1.0);

        let pois_oos_res = run_kind(
            &oos_sheet,
            AnalysisKind::GeneralizedLinearModel(GlmSpec {
                family: GlmFamily::Poisson,
                response_column: 3,
                predictor_columns: vec![0],
                dispersion_mode: GlmDispersionMode::QuasiPoisson,
                ..Default::default()
            }),
        )
        .unwrap();
        assert_all_sheet_numbers_finite(&pois_oos_res, "prop_pois_oos");
        let pois_oos_tab = tab_by_id(&pois_oos_res, "glm-oos-predictions");
        let rate = cell_num(&pois_oos_tab.rows[0][2]);
        let (r_lo, r_hi) = cell_ci_finite(&pois_oos_tab.rows[0][3]);
        prop_assert!(r_lo > 0.0 && r_lo <= rate + 1e-10 && rate <= r_hi + 1e-10);

        // 6. Custom survival status policy with unexpected code excluded stays finite.
        let mut mv_cox = MultipleVariablesTable::new(3, n).unwrap();
        mv_cox.variables[0].title = String::from("time");
        mv_cox.variables[1].title = String::from("status");
        mv_cox.variables[2].title = String::from("x1");
        for (i, &x) in x_sorted.iter().enumerate() {
            mv_cox.variables[0].cells[i] = Cell::number((i + 1) as f64);
            mv_cox.variables[1].cells[i] = Cell::number(if i == n - 1 { 99.0 } else if i % 2 == 0 { 2.0 } else { 3.0 });
            mv_cox.variables[2].cells[i] = Cell::number(x);
        }
        let cox_sheet = DataSheet::new(SheetId::new("prop-cox"), "prop-cox", TableArchetype::MultipleVariables(mv_cox)).unwrap();
        let cox_res = run_kind(
            &cox_sheet,
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
                time_column: 0,
                status_column: 1,
                predictor_columns: vec![2],
                reference_levels: Vec::new(),
                tie_method: CoxTieMethod::Efron,
                formula: None,
                covariate_profiles: Vec::new(),
                nested_comparison: None,
                emit_cox_snell_residuals: true,
                ci_method: RegressionCiMethod::Wald,
                event_code: Some(2.0),
                censored_code: Some(3.0),
                treat_other_as: SurvivalOtherValuePolicy::Exclude,
            }),
        )
        .unwrap();
        assert_all_sheet_numbers_finite(&cox_res, "prop_cox_custom_status");
    }
}
