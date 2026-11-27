//! R3.5 oracle and reproducer tests for `power.rs` and `multiplicity.rs`.
//!
//! Every fix starts with a failing reproducer plus published-literature `D1` oracles
//! with explicit numerical tolerances and citations.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{MultiplicityMethod, MultiplicitySpec};
use curvebook_math::dist::{
    chi2_pdf, dunnett_two_sided_p, f_sf, nct_cdf, nct_sf, student_t_isf, student_t_two_sided_isf, student_t_two_sided_p, studentized_range_sf,
};
use curvebook_math::quad::gauss_legendre_32;
use curvebook_stats::StatsError;
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_p_values, compare_dunn_ranks, compare_groups_pairwise, compare_groups_with_procedure,
    linear_trend_test,
};
use curvebook_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode, solve_power,
};
use curvebook_stats::two_group::multiple_row_t_tests;
use curvebook_testkit::assert_approx_eq;

fn welch_se_and_df(a: &GroupComparisonInput, b: &GroupComparisonInput) -> (f64, f64) {
    let va = (a.sd * a.sd) / a.n;
    let vb = (b.sd * b.sd) / b.n;
    let v = va + vb;
    let se = v.sqrt();
    let denom = (va * va) / (a.n - 1.0) + (vb * vb) / (b.n - 1.0);
    (se, (v * v) / denom)
}

#[test]
fn multiplicity_raw_p_is_genuine_unadjusted_pairwise_p_for_tukey_dunnett_games_howell_and_t3() {
    // 1. Pooled-variance comparisons: Tukey-Kramer (Tukey 1949; Kramer 1956) and Dunnett (1955).
    let pooled_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("Ctrl"), n: 5.0, mean: 10.0, sd: 2.0 },
        GroupComparisonInput { index: 1, label: String::from("Low"), n: 5.0, mean: 13.0, sd: 2.0 },
        GroupComparisonInput { index: 2, label: String::from("High"), n: 5.0, mean: 18.0, sd: 2.0 },
    ];
    let mse = 4.0;
    let df_error = 12.0;

    let tukey = compare_groups_with_procedure(&pooled_groups, mse, df_error, MultiplicityProcedure::TukeyKramer, 0.05, true).unwrap();
    assert_eq!(tukey.len(), 3);
    for row in &tukey {
        let se_diff = (mse * (1.0 / 5.0 + 1.0 / 5.0)).sqrt();
        let expected_raw_p = student_t_two_sided_p(row.mean_diff / se_diff, df_error).unwrap();
        let expected_adj_p = studentized_range_sf(row.mean_diff.abs() / (se_diff * std::f64::consts::FRAC_1_SQRT_2), 3, df_error).unwrap();
        assert_approx_eq(row.raw_p, expected_raw_p, 1e-12, 1e-12, "Tukey raw_p = unadjusted Student t");
        assert_approx_eq(row.adjusted_p_or_q, expected_adj_p, 1e-10, 1e-10, "Tukey adjusted_p");
        assert!(row.raw_p < row.adjusted_p_or_q, "Tukey raw_p ({}) must be < adjusted ({})", row.raw_p, row.adjusted_p_or_q);
    }

    let dunnett =
        compare_groups_with_procedure(&pooled_groups, mse, df_error, MultiplicityProcedure::Dunnett { control_column: 0 }, 0.05, true).unwrap();
    assert_eq!(dunnett.len(), 2);
    let treat_ns = [5.0, 5.0];
    for row in &dunnett {
        let se_diff = (mse * (1.0 / 5.0 + 1.0 / 5.0)).sqrt();
        let t_val = row.mean_diff / se_diff;
        let expected_raw_p = student_t_two_sided_p(t_val, df_error).unwrap();
        let expected_adj_p = dunnett_two_sided_p(t_val.abs(), df_error, 5.0, &treat_ns).unwrap();
        assert_approx_eq(row.raw_p, expected_raw_p, 1e-12, 1e-12, "Dunnett raw_p = unadjusted Student t");
        assert_approx_eq(row.adjusted_p_or_q, expected_adj_p, 1e-10, 1e-10, "Dunnett adjusted_p");
        assert!(row.raw_p < row.adjusted_p_or_q, "Dunnett raw_p ({}) must be < adjusted ({})", row.raw_p, row.adjusted_p_or_q);
    }

    // 2. Unequal-variance comparisons: Games-Howell (1976) and Dunnett T3 (1980).
    let welch_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("A"), n: 6.0, mean: 10.0, sd: 1.2 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 8.0, mean: 13.5, sd: 3.0 },
        GroupComparisonInput { index: 2, label: String::from("C"), n: 5.0, mean: 16.0, sd: 1.8 },
    ];
    let gh = compare_groups_with_procedure(&welch_groups, 0.0, 0.0, MultiplicityProcedure::GamesHowell, 0.05, true).unwrap();
    let t3 = compare_groups_with_procedure(&welch_groups, 0.0, 0.0, MultiplicityProcedure::DunnettT3, 0.05, true).unwrap();
    let pair_defs = [(0usize, 1usize), (0, 2), (1, 2)];
    for (idx, &(i, j)) in pair_defs.iter().enumerate() {
        let (se, df_w) = welch_se_and_df(&welch_groups[i], &welch_groups[j]);
        let diff = welch_groups[i].mean - welch_groups[j].mean;
        let expected_raw_p = student_t_two_sided_p(diff / se, df_w).unwrap();
        let expected_gh_adj = studentized_range_sf(diff.abs() / (se * std::f64::consts::FRAC_1_SQRT_2), 3, df_w).unwrap();

        assert_approx_eq(gh[idx].raw_p, expected_raw_p, 1e-12, 1e-12, "Games-Howell raw_p = unadjusted Welch t");
        assert_approx_eq(gh[idx].adjusted_p_or_q, expected_gh_adj, 1e-10, 1e-10, "Games-Howell adjusted_p");
        assert!(gh[idx].raw_p < gh[idx].adjusted_p_or_q);

        assert_approx_eq(t3[idx].raw_p, expected_raw_p, 1e-12, 1e-12, "Dunnett T3 raw_p = unadjusted Welch t");
        assert!(t3[idx].raw_p < t3[idx].adjusted_p_or_q);
    }

    // 3. Zero-variance preservation (matches audit_r1_s4a_stats semantics).
    let zero_var = vec![
        GroupComparisonInput { index: 0, label: String::from("Z1"), n: 4.0, mean: 5.0, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("Z2"), n: 4.0, mean: 5.0, sd: 0.0 },
        GroupComparisonInput { index: 2, label: String::from("Z3"), n: 4.0, mean: 8.0, sd: 0.0 },
    ];
    for proc in [
        MultiplicityProcedure::TukeyKramer,
        MultiplicityProcedure::Dunnett { control_column: 0 },
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::DunnettT3,
    ] {
        let rows = compare_groups_with_procedure(&zero_var, 0.0, 9.0, proc, 0.05, true).unwrap();
        for row in rows {
            if row.mean_diff.abs() > 0.0 {
                assert_eq!(row.raw_p, 0.0);
                assert_eq!(row.adjusted_p_or_q, 0.0);
                assert!(row.statistic.is_infinite());
            } else {
                assert_eq!(row.raw_p, 1.0);
                assert_eq!(row.adjusted_p_or_q, 1.0);
            }
        }
    }
}

#[test]
fn linear_trend_test_zero_mse_and_published_maxwell_delaney_oracle() {
    // 1. Maxwell & Delaney (2004, Ch. 6) polynomial linear contrast oracle:
    //    n = [5, 5, 5], means = [10, 16, 22], MSE = 4, df_error = 12.
    //    c = [-1, 0, 1], L = 12, sum(c_i^2 / n_i) = 0.4 => SS_trend = 360, F = 90, slope = 6.
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("Dose0"), n: 5.0, mean: 10.0, sd: 2.0 },
        GroupComparisonInput { index: 1, label: String::from("Dose1"), n: 5.0, mean: 16.0, sd: 2.0 },
        GroupComparisonInput { index: 2, label: String::from("Dose2"), n: 5.0, mean: 22.0, sd: 2.0 },
    ];
    let tr = linear_trend_test(&groups, 4.0, 12.0).unwrap();
    assert_approx_eq(tr.ss_trend, 360.0, 1e-12, 1e-12, "Linear trend SS");
    assert_approx_eq(tr.f_statistic, 90.0, 1e-12, 1e-12, "Linear trend F");
    assert_approx_eq(tr.slope_estimate, 6.0, 1e-12, 1e-12, "Linear trend slope");
    assert_approx_eq(tr.p_value, f_sf(90.0, 1.0, 12.0).unwrap(), 1e-12, 1e-12, "Linear trend p");

    // 2. Nonzero trend with MSE == 0 => F = +inf, p = 0.0 (not F = 0, p = 1).
    let zero_mse_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("D0"), n: 4.0, mean: 10.0, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("D1"), n: 4.0, mean: 15.0, sd: 0.0 },
        GroupComparisonInput { index: 2, label: String::from("D2"), n: 4.0, mean: 20.0, sd: 0.0 },
    ];
    let tr_inf = linear_trend_test(&zero_mse_groups, 0.0, 9.0).unwrap();
    assert!(tr_inf.ss_trend > 0.0);
    assert!(tr_inf.f_statistic.is_infinite() && tr_inf.f_statistic.is_sign_positive(), "expected +inf, got {}", tr_inf.f_statistic);
    assert_eq!(tr_inf.p_value, 0.0, "expected p = 0.0 when trend != 0 and MSE == 0");
    assert_approx_eq(tr_inf.slope_estimate, 5.0, 1e-12, 1e-12, "Zero-MSE trend slope");

    // 3. Zero linear trend with MSE == 0 => indeterminate 0/0 -> F = 0.0, p = 1.0.
    let flat_zero_mse = vec![
        GroupComparisonInput { index: 0, label: String::from("D0"), n: 4.0, mean: 10.0, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("D1"), n: 4.0, mean: 25.0, sd: 0.0 },
        GroupComparisonInput { index: 2, label: String::from("D2"), n: 4.0, mean: 10.0, sd: 0.0 },
    ];
    let tr_zero = linear_trend_test(&flat_zero_mse, 0.0, 9.0).unwrap();
    assert_eq!(tr_zero.ss_trend, 0.0);
    assert_eq!(tr_zero.f_statistic, 0.0);
    assert_eq!(tr_zero.p_value, 1.0);
    assert_eq!(tr_zero.slope_estimate, 0.0);
}

#[test]
fn dunn_rank_sum_and_linear_trend_never_emit_silent_unadjusted_output() {
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("G1"), n: 5.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: String::from("G2"), n: 5.0, mean: 13.0, sd: 1.5 },
        GroupComparisonInput { index: 2, label: String::from("G3"), n: 5.0, mean: 17.0, sd: 1.5 },
    ];

    // LinearTrend preserves distinct MultiplicityProcedure variant instead of aliasing unadjusted None.
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::LinearTrend), MultiplicityProcedure::LinearTrend);
    assert_eq!(MultiplicityProcedure::from(MultiplicityMethod::DunnRankSum), MultiplicityProcedure::DunnRankSum);

    // Parametric pairwise comparison paths reject both DunnRankSum and LinearTrend with Typed Errors.
    for method in [MultiplicityMethod::DunnRankSum, MultiplicityMethod::LinearTrend] {
        let spec = MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true, control_column: None };
        assert!(
            matches!(compare_groups_pairwise(&groups, 2.25, 12.0, &spec), Err(StatsError::InvalidParameter { .. })),
            "expected InvalidParameter for {method:?} on parametric pairwise path"
        );
    }

    // adjust_p_values applies Bonferroni FWER adjustment rather than raw pass-through.
    let raw = [0.01, 0.04, 0.20];
    for proc in [MultiplicityProcedure::DunnRankSum, MultiplicityProcedure::LinearTrend] {
        let (adj, is_fdr) = adjust_p_values(&raw, proc, 0.05);
        assert!(!is_fdr);
        assert_approx_eq(adj[0], 0.03, 1e-12, 1e-12, "Bonferroni adj[0]");
        assert_approx_eq(adj[1], 0.12, 1e-12, 1e-12, "Bonferroni adj[1]");
        assert_approx_eq(adj[2], 0.60, 1e-12, 1e-12, "Bonferroni adj[2]");
    }

    // Dunn rank-sum post-hoc with LinearTrend also applies Bonferroni rather than None.
    let dunn_lt = compare_dunn_ranks(
        &[(0, String::from("G1")), (1, String::from("G2")), (2, String::from("G3"))],
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false, control_column: None },
    )
    .unwrap();
    for row in &dunn_lt {
        assert_approx_eq(row.adjusted_p_or_q, (row.raw_p * 3.0).min(1.0), 1e-12, 1e-12, "Dunn LinearTrend falls back to Bonferroni, not None");
    }
}

#[test]
fn dunnett_and_dunn_control_column_lookup_errors_on_missing_and_respects_sparse_indices() {
    let sparse_groups = vec![
        GroupComparisonInput { index: 1, label: String::from("Ctrl"), n: 5.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 3, label: String::from("Low"), n: 5.0, mean: 13.0, sd: 1.5 },
        GroupComparisonInput { index: 5, label: String::from("High"), n: 5.0, mean: 17.0, sd: 1.5 },
    ];
    assert!(matches!(
        compare_groups_with_procedure(&sparse_groups, 2.25, 12.0, MultiplicityProcedure::Dunnett { control_column: 0 }, 0.05, true),
        Err(StatsError::InvalidParameter { name: "control_column", .. })
    ));
    let ok_dunnett =
        compare_groups_with_procedure(&sparse_groups, 2.25, 12.0, MultiplicityProcedure::Dunnett { control_column: 3 }, 0.05, true).unwrap();
    assert_eq!(ok_dunnett.len(), 2);
    assert!(ok_dunnett.iter().all(|r| r.group_b == 3));

    let sparse_labels = [(1, String::from("Ctrl")), (3, String::from("Low")), (5, String::from("High"))];
    assert!(matches!(
        compare_dunn_ranks(
            &sparse_labels,
            &[5, 5, 5],
            &[3.0, 8.0, 13.0],
            15,
            0.0,
            &MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: false,
                control_column: None
            },
        ),
        Err(StatsError::InvalidParameter { name: "control_column", .. })
    ));
    let ok_dunn = compare_dunn_ranks(
        &sparse_labels,
        &[5, 5, 5],
        &[3.0, 8.0, 13.0],
        15,
        0.0,
        &MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 3 },
            threshold: 0.05,
            report_confidence_intervals: false,
            control_column: None,
        },
    )
    .unwrap();
    assert_eq!(ok_dunn.len(), 2);
    assert!(ok_dunn.iter().all(|r| r.group_b == 3));
}

#[test]
fn multiplicity_unbalanced_dunnett_raw_p_and_multiple_row_bonferroni_adjustment() {
    // Unbalanced Dunnett (1955) with non-zero control_column: verify raw_p matches unadjusted Student t
    // on pooled MSE while adjusted_p_or_q matches exact multivariate-t Dunnett (unequal group sizes).
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("T1"), n: 6.0, mean: 14.0, sd: 2.0 },
        GroupComparisonInput { index: 1, label: String::from("T2"), n: 5.0, mean: 16.5, sd: 2.0 },
        GroupComparisonInput { index: 2, label: String::from("Ctrl"), n: 8.0, mean: 11.0, sd: 2.0 },
        GroupComparisonInput { index: 3, label: String::from("T3"), n: 4.0, mean: 13.2, sd: 2.0 },
    ];
    let mse = 4.0;
    let df_error = 19.0;
    let rows = compare_groups_with_procedure(&groups, mse, df_error, MultiplicityProcedure::Dunnett { control_column: 2 }, 0.05, true).unwrap();
    assert_eq!(rows.len(), 3);
    let treat_ns = [6.0, 5.0, 4.0];
    for (row, &tn) in rows.iter().zip(treat_ns.iter()) {
        let se_diff = (mse * (1.0 / tn + 1.0 / 8.0)).sqrt();
        let t_val = row.mean_diff / se_diff;
        let expected_raw_p = student_t_two_sided_p(t_val, df_error).unwrap();
        let expected_adj_p = dunnett_two_sided_p(t_val.abs(), df_error, 8.0, &treat_ns).unwrap();
        assert_approx_eq(row.raw_p, expected_raw_p, 1e-12, 1e-12, "Unbalanced Dunnett raw_p");
        assert_approx_eq(row.adjusted_p_or_q, expected_adj_p, 1e-10, 1e-10, "Unbalanced Dunnett adj_p");
        assert!(row.raw_p < row.adjusted_p_or_q);
    }

    // Multiple row t-tests reject ANOVA-only procedures cleanly (#81) while applying Bonferroni directly.
    let row_labels = vec![String::from("R1"), String::from("R2"), String::from("R3")];
    let ga = vec![vec![10.0, 11.0, 12.0, 13.0], vec![10.0, 10.5, 11.0, 11.5], vec![1.0, 2.0, 3.0, 4.0]];
    let gb = vec![vec![14.0, 15.0, 16.0, 17.0], vec![10.1, 10.6, 11.1, 11.6], vec![2.0, 3.0, 4.0, 5.0]];
    for proc in [MultiplicityProcedure::LinearTrend, MultiplicityProcedure::DunnRankSum] {
        let err = multiple_row_t_tests(&row_labels, &ga, &gb, false, proc, 0.05).unwrap_err();
        assert!(matches!(err, StatsError::IncompatibleAnalysisSpec { .. }), "expected IncompatibleAnalysisSpec for {proc:?}, got {err:?}");
    }
    let mr_bonf = multiple_row_t_tests(&row_labels, &ga, &gb, false, MultiplicityProcedure::Bonferroni, 0.05).unwrap();
    assert_eq!(mr_bonf.len(), 3);
    for r in mr_bonf {
        assert_approx_eq(r.adjusted_p_or_q, (r.raw_p * 3.0).min(1.0), 1e-12, 1e-12, "Multiple row t-test Bonferroni");
    }
}

#[test]
fn correlation_exact_hotelling_r_oracles_two_sided_and_one_sided() {
    // Exact unconditional bivariate-normal distribution of sample correlation r
    // (Fisher 1915, Biometrika 10:507-521; Hotelling 1953, JRSS-B 15:193-232; Anderson 2003, Ch. 4).
    // Verified independently by both Hotelling hypergeometric PDF quadrature and chi^2_{n-1} mixture of nct(df=n-2).
    let cases = [
        (84.0, 0.3, 0.05, TailMode::TwoSided, 0.800_338_953_408_2),
        (84.0, 0.3, 0.05, TailMode::OneSided(OneSidedDirection::Greater), 0.877_014_132_221_5),
        (84.0, -0.3, 0.05, TailMode::OneSided(OneSidedDirection::Greater), 0.000_006_054_127_0),
        (20.0, 0.5, 0.05, TailMode::TwoSided, 0.641_656_317_241_9),
        (50.0, 0.4, 0.01, TailMode::TwoSided, 0.634_278_448_887_5),
        (10.0, 0.6, 0.05, TailMode::TwoSided, 0.485_834_501_959_4),
    ];
    for (n, r, alpha, tail_mode, expected) in cases {
        let mut req = PowerAnalysisRequest::correlation(r, alpha, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
        req.solve_target = PowerSolveTarget::Power;
        req.tail_mode = tail_mode;
        req.sample_size = Some(n);
        let res = solve_power(&req).unwrap();
        assert_approx_eq(res.achieved_power, expected, 1e-9, 1e-9, &format!("Exact r power at n={n}, r={r}, alpha={alpha}"));
    }

    // Solve sample size at rho=0.3, alpha=0.05, target power=0.80: continuous_n = 83.9292074693, n = 84.
    let n_res =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform)).unwrap();
    assert_eq!(n_res.n_per_group, 84);
    assert_approx_eq(n_res.continuous_n, 83.929_207_469_3, 1e-7, 1e-7, "Exact r continuous_n at rho=0.3, power=0.80");

    // Independent cross-check against Fisher (1928) / Anderson Ch. 4 representation:
    // T = r * sqrt(n - 2) / sqrt(1 - r^2) | U ~ t_{n-2}(delta = rho / sqrt(1 - rho^2) * sqrt(U)), U ~ chi^2_{n-1}.
    let mixture_check = |n: f64, r: f64, alpha: f64, tail_mode: TailMode| -> f64 {
        let df_t = n - 2.0;
        let nu_u = n - 1.0;
        let f = r / (1.0 - r * r).sqrt();
        let lo = (nu_u - 10.0 * (2.0 * nu_u).sqrt()).max(1e-8);
        let hi = nu_u + 12.0 * (2.0 * nu_u).sqrt();
        let splits = [lo, (lo + nu_u) * 0.5, nu_u, (nu_u + hi) * 0.5, hi];
        let cond_pow = |u: f64| -> f64 {
            let delta = f * u.sqrt();
            let p_cond = match tail_mode {
                TailMode::TwoSided => {
                    let tc = student_t_two_sided_isf(alpha, df_t).unwrap();
                    nct_sf(tc, df_t, delta).unwrap() + nct_cdf(-tc, df_t, delta).unwrap()
                }
                TailMode::OneSided(OneSidedDirection::Greater) => {
                    let tc = student_t_isf(alpha, df_t).unwrap();
                    nct_sf(tc, df_t, delta).unwrap()
                }
                TailMode::OneSided(OneSidedDirection::Less) => {
                    let tc = student_t_isf(alpha, df_t).unwrap();
                    nct_cdf(-tc, df_t, delta).unwrap()
                }
            };
            p_cond * chi2_pdf(u, nu_u).unwrap()
        };
        splits.windows(2).map(|w| gauss_legendre_32(cond_pow, w[0], w[1])).sum()
    };

    for (n, r, alpha, tail) in [
        (12.0, 0.45, 0.05, TailMode::TwoSided),
        (25.0, -0.35, 0.05, TailMode::OneSided(OneSidedDirection::Less)),
        (60.0, 0.25, 0.01, TailMode::OneSided(OneSidedDirection::Greater)),
    ] {
        let mut req = PowerAnalysisRequest::correlation(r, alpha, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
        req.solve_target = PowerSolveTarget::Power;
        req.sample_size = Some(n);
        req.tail_mode = tail;
        let got = solve_power(&req).unwrap().achieved_power;
        let expected_mix = mixture_check(n, r, alpha, tail);
        assert_approx_eq(got, expected_mix, 1e-8, 1e-8, "Exact r matches chi2_{n-1} mixture of nct(n-2)");
    }

    // Null correlation r = 0 gives exact alpha across small and large n and all tail modes.
    for n in [2.5, 3.0, 10.0, 84.0] {
        for tail in [TailMode::TwoSided, TailMode::OneSided(OneSidedDirection::Greater), TailMode::OneSided(OneSidedDirection::Less)] {
            let mut req = PowerAnalysisRequest::correlation(0.0, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
            req.solve_target = PowerSolveTarget::Power;
            req.sample_size = Some(n);
            req.tail_mode = tail;
            assert_approx_eq(solve_power(&req).unwrap().achieved_power, 0.05, 1e-12, 1e-12, "Null exact r == alpha");
        }
    }

    // Extreme-n / extreme-r stability and DetectableEffect roundtrip under ExactTTransform.
    for (n, r) in [(2.25, 0.5), (3.0, 0.99), (10_000.0, 0.03), (100_000.0, 0.9)] {
        let mut req = PowerAnalysisRequest::correlation(r, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
        req.solve_target = PowerSolveTarget::Power;
        req.sample_size = Some(n);
        let p = solve_power(&req).unwrap().achieved_power;
        assert!(p.is_finite() && (0.0..=1.0).contains(&p) && p > 0.05, "invalid exact power {p} at n={n}, r={r}");
    }

    let mut eff_req = PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
    eff_req.solve_target = PowerSolveTarget::DetectableEffect;
    eff_req.sample_size = Some(84.0);
    let det = solve_power(&eff_req).unwrap();
    let mut check_pow =
        PowerAnalysisRequest::correlation(det.effect_size, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
    check_pow.solve_target = PowerSolveTarget::Power;
    check_pow.sample_size = Some(84.0);
    assert_approx_eq(solve_power(&check_pow).unwrap().achieved_power, 0.80, 1e-10, 1e-10, "ExactTTransform effect_size roundtrip");
}

#[test]
fn correlation_and_simple_linear_regression_n_convention_alignment() {
    // Cohen (1988, Ch. 9, Eq. 9.2.3, p. 414) and Chow, Shao & Wang (2008, §10.2):
    // Fixed-x slope t-test (CorrelationPowerMethod::ConditionalSlopeT) and SimpleLinearRegression
    // share df = n - 2 and lambda = n * f^2 where f^2 = r^2 / (1 - r^2) = R^2 / (1 - R^2).
    for (r, alpha, target_pow) in [(0.3_f64, 0.05, 0.80), ((0.15_f64 / 1.15_f64).sqrt(), 0.05, 0.80), (0.45, 0.01, 0.90)] {
        let r2 = r * r;
        let f2 = r2 / (1.0 - r2);

        let corr_n =
            solve_power(&PowerAnalysisRequest::correlation(r, alpha, target_pow).with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT))
                .unwrap();
        let mut reg_r2_req = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression).with_effect(PowerEffectSpec::RSquared(r2));
        reg_r2_req.alpha = alpha;
        reg_r2_req.target_power = Some(target_pow);
        let reg_r2_n = solve_power(&reg_r2_req).unwrap();

        let mut reg_f2_req = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression).with_effect(PowerEffectSpec::Standardized(f2));
        reg_f2_req.alpha = alpha;
        reg_f2_req.target_power = Some(target_pow);
        let reg_f2_n = solve_power(&reg_f2_req).unwrap();

        assert_eq!(corr_n.n_per_group, reg_r2_n.n_per_group);
        assert_eq!(corr_n.n_per_group, reg_f2_n.n_per_group);
        assert_approx_eq(
            corr_n.continuous_n,
            reg_r2_n.continuous_n,
            1e-11,
            1e-11,
            "Correlation ConditionalSlopeT vs Regression RSquared continuous_n",
        );
        assert_approx_eq(corr_n.continuous_n, reg_f2_n.continuous_n, 1e-11, 1e-11, "Correlation ConditionalSlopeT vs Regression f2 continuous_n");
        assert_approx_eq(corr_n.achieved_power, reg_r2_n.achieved_power, 1e-12, 1e-12, "Aligned achieved_power");
    }

    // Signed RegressionSlope alignment across TwoSided and OneSided(Less) with negative correlation.
    let r_neg = -0.35_f64;
    let beta1 = -0.7_f64;
    let sigma_x = 1.5_f64;
    let sigma_eps = (beta1.abs() * sigma_x) * (1.0 - r_neg * r_neg).sqrt() / r_neg.abs();
    for tail in [TailMode::TwoSided, TailMode::OneSided(OneSidedDirection::Less)] {
        let mut c_req = PowerAnalysisRequest::correlation(r_neg, 0.05, 0.80)
            .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(60.0);
        c_req.tail_mode = tail;
        let mut r_req = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
            .with_effect(PowerEffectSpec::RegressionSlope { beta1, sigma_x, sigma_eps })
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(60.0);
        r_req.tail_mode = tail;
        let c_ev = solve_power(&c_req).unwrap();
        let r_ev = solve_power(&r_req).unwrap();
        assert_approx_eq(c_ev.achieved_power, r_ev.achieved_power, 1e-12, 1e-12, "Signed slope vs negative correlation power alignment");
    }

    // Detectable effect roundtrip equivalence: r^2 == implied_r_squared == f^2 / (1 + f^2).
    let mut c_det = PowerAnalysisRequest::correlation(0.3, 0.05, 0.80)
        .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(55.0);
    c_det.alpha = 0.05;
    let mut r_det = PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
        .with_effect(PowerEffectSpec::Standardized(0.15))
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(55.0);
    r_det.alpha = 0.05;
    let c_eff = solve_power(&c_det).unwrap();
    let r_eff = solve_power(&r_det).unwrap();
    let f2_solved = r_eff.effect_size;
    assert_approx_eq(c_eff.effect_size.powi(2), f2_solved / (1.0 + f2_solved), 1e-10, 1e-10, "Detectable r^2 == f^2/(1+f^2)");

    // Exact bivariate-normal r (n=84 at rho=0.3) vs aligned fixed-x slope (n=82 at r=0.3, R^2=0.09).
    let exact_r_n =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform)).unwrap();
    let fixed_x_n =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)).unwrap();
    assert_eq!(exact_r_n.n_per_group, 84);
    assert_eq!(fixed_x_n.n_per_group, 82);
}
