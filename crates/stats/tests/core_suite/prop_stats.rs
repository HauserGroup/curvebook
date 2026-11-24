//! Property-based tests (`proptest`) verifying statistical invariances, bounds, equivariances,
//! and algebraic identities across `curvebook-stats`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::sum::SummaryMoments;
use curvebook_math::{ConfidenceLevel, DenseMatrix};
use curvebook_stats::anova::ordinary_one_way_anova;
use curvebook_stats::categorical::fisher_exact_2x2;
use curvebook_stats::column::{one_sample_t_test, one_sample_wilcoxon, summarize_column};
use curvebook_stats::multiplicity::{
    adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni, adjust_holm_sidak, adjust_sidak,
    adjust_two_stage_bky,
};
use curvebook_stats::multivariate::{pca_analysis, pearson_correlation, spearman_correlation};
use curvebook_stats::regression::multiple_linear_regression;
use curvebook_stats::two_group::{mann_whitney_u, paired_t_test, unpaired_t_test};
use curvebook_testkit::assert_approx_eq;
use proptest::prelude::*;

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
}
