#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Decision D1 Numerical Verification Oracle Suite for `curvebook_stats::power`.
//!
//! Reference provenance:
//! - Exact noncentral-t, noncentral-F, and noncentral-chi2 distributions (`curvebook_math::dist::noncentral`).
//! - Published analytical power & sample-size benchmarks (Cohen 1988 *Statistical Power Analysis
//!   for the Behavioral Sciences*; Fleiss 1981 *Statistical Methods for Rates and Proportions*;
//!   R `pwr` package conventions for `pwr.t.test`, `pwr.t2n.test`, `pwr.anova.test`,
//!   `pwr.2p.test`, `pwr.r.test`, `pwr.chisq.test`, and `pwr.f2.test`).

use curvebook_math::dist::{f_isf, ncf_sf};
use curvebook_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode,
    TwoProportionsMethod, cohens_h, compute_power_curve, compute_sensitivity_table, solve_power,
};

#[test]
fn oracle_unpaired_t_equal_and_unequal_allocation() {
    // Cohen's d = 0.5, alpha = 0.05, power = 0.80, two-sided:
    // R pwr.t.test(d = 0.5, sig.level = 0.05, power = 0.80, type = "two.sample") -> n = 63.76561
    let req = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80);
    let res = solve_power(&req).unwrap();

    assert!((res.continuous_n - 63.765_611).abs() < 1e-4, "continuous_n = {}", res.continuous_n);
    assert_eq!(res.n_per_group, 64);
    assert_eq!(res.n_group2, Some(64));
    assert_eq!(res.total_n, 128);
    assert_eq!(res.actual_df, Some(126.0));
    assert!((res.power_at_continuous_n - 0.80).abs() < 1e-10);
    assert!((res.achieved_power - 0.801_459_5).abs() < 1e-5, "achieved_power = {}", res.achieved_power);

    // Raw means and pooled SD yield identical output.
    let raw_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_effect(PowerEffectSpec::GroupMeansAndSd { means: vec![12.0, 10.0], within_sd: 4.0 });
    let raw_res = solve_power(&raw_req).unwrap();
    assert!((raw_res.continuous_n - res.continuous_n).abs() < 1e-12);
    assert!((raw_res.achieved_power - res.achieved_power).abs() < 1e-12);
    assert!((raw_res.raw_effect_size.unwrap() - 2.0).abs() < 1e-12);

    // Unequal allocation k = n2 / n1 = 2.0:
    let k2_req = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_allocation_ratio(2.0);
    let k2_res = solve_power(&k2_req).unwrap();
    assert_eq!(k2_res.n_per_group, 48);
    assert_eq!(k2_res.n_group2, Some(96));
    assert_eq!(k2_res.total_n, 144);
    assert!(k2_res.total_n > res.total_n);
    assert!(k2_res.achieved_power >= 0.80);

    // Explicit (n1 = 48, n2 = 96) in Power mode matches k2_res.achieved_power exactly.
    let explicit_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(48.0)
        .with_sample_size_group2(96.0)
        .with_effect(PowerEffectSpec::Standardized(0.5));
    let explicit_res = solve_power(&explicit_req).unwrap();
    assert!((explicit_res.achieved_power - k2_res.achieved_power).abs() < 1e-12);

    // One-sided unpaired t (d = 0.5 -> continuous n ~ 50.15, ceil n = 51).
    let onesided_pos = solve_power(&req.clone().with_tail(TailMode::ONE_SIDED_GREATER)).unwrap();
    assert!((onesided_pos.continuous_n - 50.150_8).abs() < 1e-3, "n = {}", onesided_pos.continuous_n);
    assert_eq!(onesided_pos.n_per_group, 51);
    assert!(onesided_pos.achieved_power >= 0.80);

    let onesided_neg =
        solve_power(&PowerAnalysisRequest::unpaired_t(-0.5, 0.05, 0.80).with_tail(TailMode::OneSided(OneSidedDirection::Less))).unwrap();
    assert!((onesided_neg.continuous_n - onesided_pos.continuous_n).abs() < 1e-10);
    assert_eq!(onesided_neg.n_per_group, onesided_pos.n_per_group);
}

#[test]
fn oracle_paired_t_correlation_reduction_and_identity() {
    // Paired t with dz = 0.5 -> n ~ 33.367, ceil n = 34.
    let dz_res = solve_power(&PowerAnalysisRequest::paired_t(0.5, 0.05, 0.80)).unwrap();
    assert!((dz_res.continuous_n - 33.367_13).abs() < 1e-4, "n = {}", dz_res.continuous_n);
    assert_eq!(dz_res.n_per_group, 34);
    assert_eq!(dz_res.n_group2, None);
    assert_eq!(dz_res.total_n, 34);
    assert_eq!(dz_res.actual_df, Some(33.0));
    assert!(dz_res.achieved_power >= 0.80);

    // Correlation r = 0.7 vs r = 0.2 with mean_diff = 2.0, group sigma = 4.0:
    let req_r07 = PowerAnalysisRequest::paired_t_correlated(2.0, 4.0, 0.7, 0.05, 0.80);
    let req_r02 = PowerAnalysisRequest::paired_t_correlated(2.0, 4.0, 0.2, 0.05, 0.80);
    let res_r07 = solve_power(&req_r07).unwrap();
    let res_r02 = solve_power(&req_r02).unwrap();

    assert!(res_r07.n_per_group < res_r02.n_per_group, "r=0.7 n={} should be < r=0.2 n={}", res_r07.n_per_group, res_r02.n_per_group);
    assert_eq!(res_r07.n_per_group, 21);
    assert_eq!(res_r02.n_per_group, 53);

    // Exact algebraic identity: sigma_d = sigma * sqrt(2 * (1 - r)).
    let sigma_d_07 = 4.0 * (2.0 * (1.0 - 0.7_f64)).sqrt();
    let direct_req =
        PowerAnalysisRequest::new(PowerTestFamily::TwoSamplePairedT).with_effect(PowerEffectSpec::PairedDiff { mean_diff: 2.0, sd_diff: sigma_d_07 });
    let direct_res = solve_power(&direct_req).unwrap();
    assert!((direct_res.continuous_n - res_r07.continuous_n).abs() < 1e-12);
    assert!((direct_res.effect_size - res_r07.effect_size).abs() < 1e-12);
    assert!((direct_res.achieved_power - res_r07.achieved_power).abs() < 1e-12);
    assert!((direct_res.noncentrality_param - res_r07.noncentrality_param).abs() < 1e-12);
    assert!((direct_res.critical_value - res_r07.critical_value).abs() < 1e-12);

    // At r = 0.5, sigma_d == sigma, so dz == mean_diff / sigma = 0.5.
    let res_r05 = solve_power(&PowerAnalysisRequest::paired_t_correlated(2.0, 4.0, 0.5, 0.05, 0.80)).unwrap();
    assert!((res_r05.continuous_n - dz_res.continuous_n).abs() < 1e-12);
}

#[test]
fn oracle_one_sample_t_and_one_way_anova() {
    // One-sample t: d = 0.5, alpha = 0.05, power = 0.80, two-sided -> n ~ 33.367, ceil 34.
    let res_1s = solve_power(&PowerAnalysisRequest::one_sample_t(0.5, 0.05, 0.80)).unwrap();
    assert!((res_1s.continuous_n - 33.367_13).abs() < 1e-4);
    assert_eq!(res_1s.n_per_group, 34);
    assert_eq!(res_1s.total_n, 34);

    // One-sample t one-sided greater: n ~ 26.1375, ceil 27.
    let res_1s_gt = solve_power(&PowerAnalysisRequest::one_sample_t(0.5, 0.05, 0.80).with_tail(TailMode::ONE_SIDED_GREATER)).unwrap();
    assert!((res_1s_gt.continuous_n - 26.137_5).abs() < 1e-3);
    assert_eq!(res_1s_gt.n_per_group, 27);

    // One-way ANOVA: k = 4, f = 0.25, alpha = 0.05, power = 0.80 -> n ~ 44.599, ceil 45 per group, total 180.
    let anova_res = solve_power(&PowerAnalysisRequest::one_way_anova(4, 0.25, 0.05, 0.80)).unwrap();
    assert!((anova_res.continuous_n - 44.599_27).abs() < 1e-4, "n = {}", anova_res.continuous_n);
    assert_eq!(anova_res.n_per_group, 45);
    assert_eq!(anova_res.n_group2, None);
    assert_eq!(anova_res.total_n, 180);
    assert_eq!(anova_res.numerator_df, Some(3.0));
    assert_eq!(anova_res.actual_df, Some(176.0));
    assert!(anova_res.achieved_power >= 0.80 && anova_res.achieved_power < 0.81);

    // One-way ANOVA from group means [10, 10, 12, 12] and within_sd = 4.0 -> f = 0.25.
    let anova_means = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::OneWayAnova)
            .with_effect(PowerEffectSpec::GroupMeansAndSd { means: vec![10.0, 10.0, 12.0, 12.0], within_sd: 4.0 }),
    )
    .unwrap();
    assert!((anova_means.effect_size - 0.25).abs() < 1e-12);
    assert!((anova_means.continuous_n - anova_res.continuous_n).abs() < 1e-12);
    assert_eq!(anova_means.n_per_group, 45);
}

#[test]
fn oracle_two_proportions() {
    // 1. Two proportions: p1 = 0.50, p2 = 0.30, alpha = 0.05, power = 0.80, two-sided.
    let prop_pooled = solve_power(&PowerAnalysisRequest::two_proportions(0.50, 0.30, 0.05, 0.80)).unwrap();
    assert!((prop_pooled.continuous_n - 92.998).abs() < 1e-2, "n = {}", prop_pooled.continuous_n);
    assert_eq!(prop_pooled.n_per_group, 93);
    assert_eq!(prop_pooled.n_group2, Some(93));
    assert_eq!(prop_pooled.total_n, 186);
    assert!(prop_pooled.achieved_power >= 0.80);

    // Continuity-corrected Fleiss formula -> ceil 103 per group.
    let prop_cc = solve_power(&PowerAnalysisRequest::two_proportions(0.50, 0.30, 0.05, 0.80).with_continuity_correction(true)).unwrap();
    assert!((prop_cc.continuous_n - 102.77).abs() < 0.05, "cc n = {}", prop_cc.continuous_n);
    assert_eq!(prop_cc.n_per_group, 103);
    assert!(prop_cc.achieved_power >= 0.80);

    // Unpooled Wald SE -> ceil 91 per group.
    let prop_unpooled =
        solve_power(&PowerAnalysisRequest::two_proportions(0.50, 0.30, 0.05, 0.80).with_two_proportions_mode(TwoProportionsMethod::Unpooled))
            .unwrap();
    assert_eq!(prop_unpooled.n_per_group, 91);

    // Cohen's h mode -> matches Standardized(cohens_h(0.5, 0.3)), n ~ 92.696, ceil 93.
    let prop_h =
        solve_power(&PowerAnalysisRequest::two_proportions(0.50, 0.30, 0.05, 0.80).with_two_proportions_mode(TwoProportionsMethod::CohensH)).unwrap();
    let h_direct = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
            .with_effect(PowerEffectSpec::Standardized(cohens_h(0.50, 0.30).unwrap()))
            .with_two_proportions_mode(TwoProportionsMethod::CohensH),
    )
    .unwrap();
    assert!((prop_h.continuous_n - 92.696).abs() < 1e-2);
    assert!((prop_h.continuous_n - h_direct.continuous_n).abs() < 1e-12);
    assert_eq!(prop_h.n_per_group, 93);
}

#[test]
fn oracle_correlation() {
    // 2. Correlation: r = 0.3, alpha = 0.05, power = 0.80, two-sided -> n = 85.
    let corr_fz = solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80)).unwrap();
    assert!((corr_fz.continuous_n - 84.9278).abs() < 1e-3, "fz n = {}", corr_fz.continuous_n);
    assert_eq!(corr_fz.n_per_group, 85);
    assert_eq!(corr_fz.actual_df, Some(83.0));
    assert!(corr_fz.achieved_power >= 0.80);

    let corr_pwr =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::FisherZWithTCrit)).unwrap();
    assert!((corr_pwr.continuous_n - 84.0736).abs() < 1e-3, "pwr.r n = {}", corr_pwr.continuous_n);
    assert_eq!(corr_pwr.n_per_group, 85);

    let corr_t =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform)).unwrap();
    assert_eq!(corr_t.n_per_group, 84);
    assert!(corr_t.continuous_n > 83.0 && corr_t.continuous_n <= 84.0);
    assert!(corr_t.achieved_power >= 0.80);
    assert!((corr_t.achieved_power - 0.800_338_953_408_2).abs() < 1e-9);
    let corr_t_83 = solve_power(
        &PowerAnalysisRequest::correlation(0.3, 0.05, 0.80)
            .with_correlation_method(CorrelationPowerMethod::ExactTTransform)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(83.0),
    )
    .unwrap();
    assert!(corr_t_83.achieved_power < 0.80);

    // ConditionalSlopeT aligns with SimpleLinearRegression's Cohen (1988) lambda = n * f^2 convention (n = 82 at r = 0.3).
    let corr_cond =
        solve_power(&PowerAnalysisRequest::correlation(0.3, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)).unwrap();
    assert_eq!(corr_cond.n_per_group, 82);
    assert!(corr_cond.achieved_power >= 0.80);

    // Null correlation r = 0 yields exact alpha under FisherZ, ExactTTransform, and ConditionalSlopeT.
    for method in [CorrelationPowerMethod::FisherZ, CorrelationPowerMethod::ExactTTransform, CorrelationPowerMethod::ConditionalSlopeT] {
        let null_c = solve_power(
            &PowerAnalysisRequest::correlation(0.0, 0.05, 0.80)
                .with_target(PowerSolveTarget::Power)
                .with_sample_size(50.0)
                .with_correlation_method(method),
        )
        .unwrap();
        assert!((null_c.achieved_power - 0.05).abs() < 1e-12);
    }
}

#[test]
fn oracle_chi_square_and_regression() {
    // 3. Chi-square: w = 0.3, df = 3, alpha = 0.05, power = 0.80 -> n ~ 121.139, ceil 122.
    let chi2_res = solve_power(&PowerAnalysisRequest::chi_square(0.3, 3.0, 0.05, 0.80)).unwrap();
    assert!((chi2_res.continuous_n - 121.139).abs() < 1e-2, "chi2 n = {}", chi2_res.continuous_n);
    assert_eq!(chi2_res.n_per_group, 122);
    assert_eq!(chi2_res.total_n, 122);
    assert_eq!(chi2_res.actual_df, Some(3.0));
    assert!(chi2_res.achieved_power >= 0.80);

    // 4. Simple linear regression (u = 1): f2 = 0.15, alpha = 0.05, power = 0.80 -> n ~ 54.324, ceil 55.
    let reg_res = solve_power(&PowerAnalysisRequest::simple_linear_regression(0.15, 0.05, 0.80)).unwrap();
    assert!((reg_res.continuous_n - 54.324).abs() < 1e-2, "reg n = {}", reg_res.continuous_n);
    assert_eq!(reg_res.n_per_group, 55);
    assert_eq!(reg_res.numerator_df, Some(1.0));
    assert_eq!(reg_res.actual_df, Some(53.0));
    assert!(reg_res.achieved_power >= 0.80);

    // Verify F(1, n-2, lambda = n * f2) identity with t-test formulation to < 1e-11:
    let f_crit = f_isf(0.05, 1.0, 53.0).unwrap();
    let f_power = ncf_sf(f_crit, 1.0, 53.0, 55.0 * 0.15).unwrap();
    assert!((reg_res.achieved_power - f_power).abs() < 1e-11);

    // Verify RSquared and RegressionSlope parameterizations match f2 = 0.25 (R^2 = 0.20):
    let reg_f2 = solve_power(&PowerAnalysisRequest::simple_linear_regression(0.25, 0.05, 0.80)).unwrap();
    let reg_r2 =
        solve_power(&PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression).with_effect(PowerEffectSpec::RSquared(0.20))).unwrap();
    let reg_slope = solve_power(&PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression).with_effect(PowerEffectSpec::RegressionSlope {
        beta1: 0.5,
        sigma_x: 2.0,
        sigma_eps: 2.0,
    }))
    .unwrap();
    assert!((reg_f2.continuous_n - reg_r2.continuous_n).abs() < 1e-12);
    assert!((reg_f2.continuous_n - reg_slope.continuous_n).abs() < 1e-12);
    assert!((reg_slope.raw_effect_size.unwrap() - 0.5).abs() < 1e-12);
}

#[test]
fn roundtrip_sample_size_power_and_detectable_effect_across_all_families() {
    let cases: Vec<PowerAnalysisRequest> = vec![
        PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80),
        PowerAnalysisRequest::unpaired_t(0.5, 0.01, 0.90).with_allocation_ratio(2.0),
        PowerAnalysisRequest::unpaired_t(0.45, 0.05, 0.85).with_tail(TailMode::ONE_SIDED_GREATER),
        PowerAnalysisRequest::paired_t(0.5, 0.05, 0.80),
        PowerAnalysisRequest::paired_t(-0.4, 0.05, 0.80).with_tail(TailMode::ONE_SIDED_LESS),
        PowerAnalysisRequest::one_sample_t(0.6, 0.05, 0.80),
        PowerAnalysisRequest::one_way_anova(4, 0.25, 0.05, 0.80),
        PowerAnalysisRequest::two_proportions(0.55, 0.35, 0.05, 0.80),
        PowerAnalysisRequest::two_proportions(0.55, 0.35, 0.05, 0.80).with_continuity_correction(true),
        PowerAnalysisRequest::two_proportions(0.55, 0.35, 0.05, 0.80).with_two_proportions_mode(TwoProportionsMethod::CohensH),
        PowerAnalysisRequest::correlation(0.3, 0.05, 0.80),
        PowerAnalysisRequest::correlation(-0.35, 0.05, 0.80).with_tail(TailMode::ONE_SIDED_LESS),
        PowerAnalysisRequest::chi_square(0.3, 3.0, 0.05, 0.80),
        PowerAnalysisRequest::simple_linear_regression(0.15, 0.05, 0.80),
    ];

    for req in cases {
        let target_p = req.target_power.unwrap();
        let sol_n = solve_power(&req).unwrap();
        assert!((sol_n.power_at_continuous_n - target_p).abs() < 1e-6, "family {:?}: {}", req.family, sol_n.power_at_continuous_n);
        assert!(sol_n.achieved_power >= target_p - 1e-12);

        // Re-solve for Power at continuous_n -> matches target_p to < 1e-6.
        let mut p_req = req.clone();
        p_req.solve_target = PowerSolveTarget::Power;
        p_req.sample_size = Some(sol_n.continuous_n);
        let sol_p = solve_power(&p_req).unwrap();
        assert!(
            (sol_p.power_at_continuous_n - target_p).abs() < 1e-6,
            "power roundtrip failed for {:?}: {} vs {}",
            req.family,
            sol_p.power_at_continuous_n,
            target_p
        );

        // Re-solve for DetectableEffect at continuous_n -> recovers effect_size to < 1e-6.
        let mut mde_req = req.clone();
        mde_req.solve_target = PowerSolveTarget::DetectableEffect;
        mde_req.sample_size = Some(sol_n.continuous_n);
        if req.family != PowerTestFamily::TwoProportions || req.two_proportions_mode == TwoProportionsMethod::CohensH {
            mde_req.effect = PowerEffectSpec::Unspecified;
        }
        let sol_e = solve_power(&mde_req).unwrap();
        assert!(
            (sol_e.effect_size - sol_n.effect_size).abs() < 1e-6,
            "MDE roundtrip failed for {:?}: {} vs {}",
            req.family,
            sol_e.effect_size,
            sol_n.effect_size
        );
    }
}

#[test]
fn power_monotonically_increases_with_n_effect_and_alpha_across_all_families() {
    let base_requests: Vec<PowerAnalysisRequest> = vec![
        PowerAnalysisRequest::unpaired_t(0.4, 0.05, 0.80),
        PowerAnalysisRequest::paired_t(0.4, 0.05, 0.80),
        PowerAnalysisRequest::one_sample_t(0.4, 0.05, 0.80),
        PowerAnalysisRequest::one_way_anova(3, 0.25, 0.05, 0.80),
        PowerAnalysisRequest::two_proportions(0.50, 0.30, 0.05, 0.80),
        PowerAnalysisRequest::correlation(0.30, 0.05, 0.80),
        PowerAnalysisRequest::chi_square(0.30, 3.0, 0.05, 0.80),
        PowerAnalysisRequest::simple_linear_regression(0.15, 0.05, 0.80),
    ];

    for base in base_requests {
        // 1. Monotone in n
        let curve = compute_power_curve(&base, 10, 120, 12).unwrap();
        for w in curve.windows(2) {
            assert!(w[1].power > w[0].power, "{:?}: power({})={} <= power({})={}", base.family, w[1].n, w[1].power, w[0].n, w[0].power);
        }

        // 2. Monotone in |effect| at fixed n = 40
        let effects: &[f64] = match base.family {
            PowerTestFamily::TwoProportions => &[0.40, 0.30, 0.20], // p2 moving further below p1=0.50
            _ => &[0.15, 0.30, 0.50],
        };
        let mut p_eff = Vec::new();
        for &e in effects {
            let mut r = base.clone();
            r.solve_target = PowerSolveTarget::Power;
            r.sample_size = Some(40.0);
            r.effect = match base.family {
                PowerTestFamily::TwoProportions => PowerEffectSpec::TwoProportions { p1: 0.50, p2: e },
                _ => PowerEffectSpec::Standardized(e),
            };
            p_eff.push(solve_power(&r).unwrap().achieved_power);
        }
        assert!(p_eff[0] < p_eff[1] && p_eff[1] < p_eff[2], "{:?} effect monotonicity: {:?}", base.family, p_eff);

        // 3. Monotone in alpha at fixed n = 40
        let mut p_alpha = Vec::new();
        for &a in &[0.005, 0.01, 0.05, 0.10] {
            let mut r = base.clone();
            r.solve_target = PowerSolveTarget::Power;
            r.sample_size = Some(40.0);
            r.alpha = a;
            p_alpha.push(solve_power(&r).unwrap().achieved_power);
        }
        for w in p_alpha.windows(2) {
            assert!(w[1] > w[0], "{:?} alpha monotonicity: {:?}", base.family, p_alpha);
        }

        // 4. Sensitivity table monotonicity
        let table = compute_sensitivity_table(&base, &[0.01, 0.05], &[0.80, 0.90]).unwrap();
        assert_eq!(table.len(), 4);
        assert!(table[0].continuous_n < table[1].continuous_n); // power 0.80 < 0.90 at alpha 0.01
        assert!(table[2].continuous_n < table[3].continuous_n); // power 0.80 < 0.90 at alpha 0.05
        assert!(table[2].continuous_n < table[0].continuous_n); // alpha 0.05 < alpha 0.01 at power 0.80
    }
}
