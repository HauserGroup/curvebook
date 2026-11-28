//! Wave 2 (`L2` Stats) unit, analytical, and oracle reference parity tests for:
//! - Barnard's exact unconditional 2x2 test (`ContingencyMethod::BarnardExact`)
//! - Passing–Bablok nonparametric method-comparison regression (`PassingBablokSpec`)
//! - Multiple Imputation (`MultivariateSpec::MultipleImputation`: MICE Type-1 PMM + Rubin / Barnard–Rubin pooling)
//!
//! Reference provenance (clean-room Decision D1):
//! - Suissa, S., & Shuster, J. J. (1985), *Journal of the Royal Statistical Society: Series A*, 148(4), 317–327
//!   (exact 2x2 unconditional nuisance maximization independently verified in both `R 4.5.2 (2025-10-31)`
//!   `stats::dbinom` + `stats::optimize` via `crates/stats/tests/reference/stats_extended_oracles.R` 2026-11-27
//!   and Python 3.13 stdlib `math` 2026-11-13).
//! - Boschloo (1970); Barnard (1945/1947) CSM convexity-symmetry-maximum ordering.
//! - Passing, H., & Bablok, W. (1983), *Journal of Clinical Chemistry and Clinical Biochemistry*, 21(11), 709–720
//!   (rank CIs, `K` shift offset, and CUSUM Kolmogorov supremum test independently verified in both
//!   `R 4.5.2 (2025-10-31)` via `crates/stats/tests/reference/stats_extended_oracles.R` 2026-11-27 and
//!   Python 3.13 stdlib 2026-11-13).
//! - Rubin, D. B. (1987), *Multiple Imputation for Nonresponse in Surveys*; Barnard, J., & Rubin, D. B. (1999), *Biometrika*, 86(4), 948–955;
//!   van Buuren, S. & Groothuis-Oudshoorn, K. (2011), *Journal of Statistical Software*, 45(3), 1–67
//!   (Rubin/Barnard-Rubin *t*/df/CI oracle verified in R 4.5.2 `pt`/`qt` 2026-11-13).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ContingencyMethod, ContingencySpec, MultivariateSpec, PassingBablokSpec, ResultCell, ResultTabKind,
    VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::StatsError;
use curvebook_stats::categorical::{BarnardOrdering, barnard_exact_2x2, barnard_exact_2x2_with_ordering, fisher_exact_2x2};
use curvebook_stats::multivariate::{MultipleImputationConfig, mice_predictive_mean_matching, rubin_pool_estimates};
use curvebook_stats::regression::{execute_passing_bablok, passing_bablok_regression};
use curvebook_testkit::assert_approx_eq;

use crate::common::{col_sheet_hint, contingency_sheet, mv_cells_sheet, run_kind, scalar as scalar_by_key, tab_by_kind, xy_multi_sheet};

#[test]
fn barnard_exact_score_matches_exact_combinatorics() {
    let cases = [
        (3, 1, 1, 3, 74.0 / 256.0, 1e-11, Some((0.5, 1e-3))),
        (1, 7, 6, 2, 0.021270751953125, 1e-11, None),
        (14, 6, 5, 15, 0.006439537086407654, 1e-9, None),
    ];
    for (a, b, c, d, exp_p, tol, exp_pi) in cases {
        let r = barnard_exact_2x2(a, b, c, d).unwrap();
        assert!(!r.used_asymptotic_fallback && r.ordering == BarnardOrdering::ScorePooled);
        assert_approx_eq(r.p_value, exp_p, tol, tol, "Barnard exact P");
        if let Some((pi, pi_tol)) = exp_pi {
            assert_approx_eq(r.nuisance_pi, pi, pi_tol, pi_tol, "Barnard nuisance pi");
        }
    }
}

#[test]
fn barnard_exact_maximizes_over_skewed_nuisance_parameter_pi() {
    let r1 = barnard_exact_2x2(1, 19, 6, 14).unwrap();
    assert!(!r1.used_asymptotic_fallback);
    assert_approx_eq(r1.p_value, 0.04875822085272651, 1e-12, 1e-12, "Barnard 1/19/6/14 skewed pi maximization");
    assert_approx_eq(r1.nuisance_pi, 0.159325643541092, 1e-5, 1e-5, "Barnard 1/19/6/14 skewed pi location");

    let r2 = barnard_exact_2x2(3, 0, 0, 12).unwrap();
    assert!(!r2.used_asymptotic_fallback && r2.nuisance_pi < 0.35);
    assert_approx_eq(r2.p_value, 0.0005497579110400005, 1e-8, 1e-8, "Barnard 3/0/0/12 exact P");
}

#[test]
fn barnard_exact_orderings_symmetry_and_degenerate_tables() {
    let fisher_p = fisher_exact_2x2(1, 7, 6, 2).unwrap();
    let score = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::ScorePooled).unwrap();
    let wald = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::WaldUnpooled).unwrap();
    let boschloo = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::Boschloo).unwrap();
    let csm = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::Csm).unwrap();
    for r in [&score, &wald, &csm] {
        assert!(r.p_value > 0.0 && r.p_value < 0.05);
    }
    assert!(boschloo.p_value <= fisher_p + 1e-12);

    for (a, b, c, d, lbl) in [(6, 2, 1, 7, "row-swap invariance"), (7, 1, 2, 6, "column-complement invariance")] {
        assert_approx_eq(barnard_exact_2x2(a, b, c, d).unwrap().p_value, score.p_value, 1e-11, 1e-11, lbl);
    }
    for (a, b, c, d, lbl) in [(5, 0, 5, 0, "identical marginal proportions P = 1"), (0, 0, 5, 5, "empty group margin P = 1")] {
        assert_approx_eq(barnard_exact_2x2(a, b, c, d).unwrap().p_value, 1.0, 1e-12, 1e-12, lbl);
    }
}

#[test]
fn barnard_exact_result_sheet_and_table_caps() {
    let res = run_kind(
        &contingency_sheet("ct-barnard", &[&[14, 6], &[5, 15]]),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::BarnardExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            expected_proportions: None,
            null_proportion: None,
        }),
    )
    .unwrap();
    assert_approx_eq(scalar_by_key(&res, "Barnard's exact P"), 0.006439537086407654, 1e-9, 1e-9, "scalar Barnard P");
    assert!((0.0..=1.0).contains(&scalar_by_key(&res, "Barnard nuisance pi")));
    assert!(scalar_by_key(&res, "Barnard Z-pooled").abs() > 2.5);
    assert_eq!(tab_by_kind(&res, ResultTabKind::MainSummary).row_labels[0], "Barnard's exact test");
    assert!(res.checklist.iter().any(|c| c.assumption.contains("Barnard")));

    assert!(matches!(
        run_kind(
            &contingency_sheet("ct-2x3", &[&[5, 6, 7], &[8, 7, 6]]),
            AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::BarnardExact, ..Default::default() })
        ),
        Err(StatsError::InvalidParameter { .. })
    ));
    let huge = barnard_exact_2x2(5_000, 5_000, 4_800, 5_200).unwrap();
    assert!(huge.used_asymptotic_fallback && huge.p_value.is_finite() && huge.p_value > 0.0 && huge.p_value < 0.05);
    assert!(matches!(barnard_exact_2x2(u64::MAX, 1, 1, 1), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(barnard_exact_2x2_with_ordering(44, 46, 46, 44, BarnardOrdering::Csm), Err(StatsError::InvalidParameter { .. })));
}

#[test]
fn passing_bablok_oracle_benchmark_and_collinear_baseline() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0];
    let ys = [3.4, 5.1, 6.8, 9.3, 10.6, 13.1, 14.9, 17.4, 18.8, 21.3, 24.7, 29.2, 32.8, 37.5, 41.0, 45.2];
    let pb = passing_bablok_regression(&xs, &ys, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!((pb.n, pb.valid_slopes, pb.k_offset), (16, 120, 0));
    assert_approx_eq(pb.slope, 2.0, 1e-12, 1e-12, "PB16 slope");
    assert_approx_eq(pb.slope_ci.0, 1.966666666666667, 1e-12, 1e-12, "PB16 slope CI lower");
    assert_approx_eq(pb.slope_ci.1, 2.025, 1e-12, 1e-12, "PB16 slope CI upper");
    assert_approx_eq(pb.intercept, 1.1, 1e-12, 1e-12, "PB16 intercept");
    assert_approx_eq(pb.intercept_ci.0, 0.787499999999997, 1e-11, 1e-11, "PB16 intercept CI lower");
    assert_approx_eq(pb.intercept_ci.1, 1.383333333333331, 1e-11, 1e-11, "PB16 intercept CI upper");
    assert_approx_eq(pb.cusum_h, 1.0, 1e-12, 1e-12, "PB16 CUSUM H");
    assert_approx_eq(pb.cusum_statistic, 1.0 / 14.0_f64.sqrt(), 1e-12, 1e-12, "PB16 CUSUM statistic H / sqrt(L + l)");
    assert_approx_eq(pb.cusum_p_value, 0.999_999_704_128_472, 1e-9, 1e-9, "PB16 CUSUM P");
    assert!(pb.linearity_passed);

    let x_exact = [1.0, 2.0, 3.0, 5.0, 8.0, 11.0];
    let y_exact: Vec<f64> = x_exact.iter().map(|&x| 2.5 * x - 3.0).collect();
    let pb_exact = passing_bablok_regression(&x_exact, &y_exact, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pb_exact.slope, 2.5, 1e-12, 1e-12, "exact line slope");
    assert_approx_eq(pb_exact.intercept, -3.0, 1e-12, 1e-12, "exact line intercept");
    assert_approx_eq(pb_exact.cusum_h, 0.0, 1e-12, 1e-12, "exact line CUSUM H");
    assert_approx_eq(pb_exact.cusum_p_value, 1.0, 1e-12, 1e-12, "exact line CUSUM P");
}

#[test]
fn passing_bablok_k_offset_symmetry_permutation_and_nonlinear_cusum() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let pb = passing_bablok_regression(&[1.0, 2.0, 2.0, 4.0, 4.0, 5.0, 6.0, 8.0], &[14.0, 8.0, 8.0, 5.0, 11.0, 7.0, 1.0, 4.0], conf).unwrap();
    assert!(pb.k_offset > 0 && pb.valid_slopes < 28 && pb.slope.is_finite() && pb.intercept.is_finite());

    let ax = [1.2, 2.5, 3.8, 5.1, 6.9, 8.4, 10.2, 12.7, 15.0, 18.3];
    let ay = [2.9, 4.8, 7.1, 8.9, 11.7, 13.8, 16.5, 20.1, 23.4, 28.6];
    let (pb_xy, pb_yx) = (passing_bablok_regression(&ax, &ay, conf).unwrap(), passing_bablok_regression(&ay, &ax, conf).unwrap());
    assert_approx_eq(pb_yx.slope, 1.0 / pb_xy.slope, 1e-11, 1e-11, "Passing-Bablok X<->Y inverse slope");
    assert_approx_eq(pb_yx.intercept, -pb_xy.intercept / pb_xy.slope, 1e-10, 1e-10, "Passing-Bablok X<->Y inverse intercept");

    let perm = [7, 2, 9, 0, 5, 1, 8, 3, 6, 4];
    let (px, py): (Vec<f64>, Vec<f64>) = perm.iter().map(|&i| (ax[i], ay[i])).unzip();
    let pb_perm = passing_bablok_regression(&px, &py, conf).unwrap();
    assert_approx_eq(pb_perm.slope, pb_xy.slope, 1e-12, 1e-12, "row permutation slope invariance");
    assert_approx_eq(pb_perm.intercept, pb_xy.intercept, 1e-12, 1e-12, "row permutation intercept invariance");
    assert_approx_eq(pb_perm.cusum_p_value, pb_xy.cusum_p_value, 1e-12, 1e-12, "row permutation CUSUM invariance");

    let qx: Vec<f64> = (1..=36).map(f64::from).collect();
    let qy: Vec<f64> = qx.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    let pb_quad = passing_bablok_regression(&qx, &qy, conf).unwrap();
    assert_approx_eq(pb_quad.cusum_h, 9.0, 1e-9, 1e-9, "quadratic CUSUM H");
    assert_approx_eq(pb_quad.cusum_statistic, 9.0 / 36.0_f64.sqrt(), 1e-9, 1e-9, "quadratic CUSUM statistic H / sqrt(L + l)");
    assert_approx_eq(pb_quad.cusum_p_value, 0.022_217_962_616_525_1, 1e-9, 1e-9, "quadratic CUSUM P");
    assert!(pb_quad.cusum_p_value < 0.05 && !pb_quad.linearity_passed);

    // D1 Provenance: R 4.5.2 (2025-10-31) Passing & Bablok (1983) non-zero K = 1 offset oracle
    // (`crates/stats/tests/reference/stats_extended_oracles.R`, 2026-11-27).
    let pb_kpos = passing_bablok_regression(
        &[1.0, 2.0, 3.0, 4.5, 5.0, 6.0, 7.5, 9.0, 11.0, 13.0],
        &[2.1, 4.5, 5.8, 11.2, 9.0, 12.4, 15.1, 18.3, 22.5, 26.2],
        conf,
    )
    .unwrap();
    assert_eq!((pb_kpos.n, pb_kpos.valid_slopes, pb_kpos.k_offset), (10, 45, 1));
    assert_approx_eq(pb_kpos.slope, 2.018_181_818_182, 1e-11, 1e-11, "PB_Kpos slope");
    assert_approx_eq(pb_kpos.slope_ci.0, 1.85, 1e-11, 1e-11, "PB_Kpos slope CI lo");
    assert_approx_eq(pb_kpos.slope_ci.1, 2.133_333_333_333, 1e-11, 1e-11, "PB_Kpos slope CI hi");
    assert_approx_eq(pb_kpos.intercept, 0.109_090_909_091, 1e-11, 1e-11, "PB_Kpos intercept");
    assert_approx_eq(pb_kpos.intercept_ci.0, -0.75, 1e-10, 1e-10, "PB_Kpos intercept CI lo");
    assert_approx_eq(pb_kpos.intercept_ci.1, 1.262_5, 1e-10, 1e-10, "PB_Kpos intercept CI hi");
    assert_approx_eq(pb_kpos.cusum_h, 2.0, 1e-11, 1e-11, "PB_Kpos CUSUM H");
    assert_approx_eq(pb_kpos.cusum_p_value, 0.818_621_174_471, 1e-9, 1e-9, "PB_Kpos CUSUM P");

    assert!(passing_bablok_regression(&[1.0, 2.0], &[3.0, 4.0], conf).is_err());
    assert!(passing_bablok_regression(&[1.0, 2.0, 3.0], &[1.0, f64::NAN, 3.0], conf).is_err());
}

#[test]
fn passing_bablok_execute_and_run_analysis_across_all_archetypes() {
    let xs = [2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0];
    let ys = [5.1, 8.9, 13.2, 16.8, 21.0, 25.1, 28.9, 33.0];

    let col_sheet = col_sheet_hint("pb-col", ColumnDesignHint::PairedRepeatedMeasures, &[("RefMethod", &xs), ("NewMethod", &ys)]);
    let res_exec = execute_passing_bablok(&col_sheet, &PassingBablokSpec { x_column: 0, y_column: 1 }, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(scalar_by_key(&res_exec, "Slope"), 2.0, 0.05, 0.05, "Column execute slope");
    assert_approx_eq(scalar_by_key(&res_exec, "Intercept"), 1.0, 0.4, 0.4, "Column execute intercept");
    assert_eq!(tab_by_kind(&res_exec, ResultTabKind::ParametersAndConfidenceIntervals).row_labels.len(), 2);
    assert_eq!(tab_by_kind(&res_exec, ResultTabKind::Residuals).rows.len(), xs.len());
    assert!(!res_exec.fitted_curves.is_empty());

    let xy_sheet = xy_multi_sheet("pb-xy", &xs, &[("Unused", &[999.0; 8]), ("TestY", &ys)]);
    let res_xy = run_kind(&xy_sheet, AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 2 })).unwrap();
    assert_approx_eq(scalar_by_key(&res_xy, "Slope"), 2.0, 0.05, 0.05, "XY col 2 slope");

    let mut id_c: Vec<Cell> = (0..xs.len()).map(|r| Cell::text(format!("S{r}"))).collect();
    let mut x_c: Vec<Cell> = xs.iter().map(|&v| Cell::number(v)).collect();
    let mut y_c: Vec<Cell> = ys.iter().map(|&v| Cell::number(v)).collect();
    id_c.push(Cell::text("OutlierExcluded"));
    x_c.push(Cell::number(100.0));
    y_c.push(Cell::excluded_number(-500.0));
    let mv_sheet = mv_cells_sheet("pb-mv", &[("SampleId", Some(VariableRole::Label), &id_c), ("Ref", None, &x_c), ("Assay", None, &y_c)]);
    let res_mv = run_kind(&mv_sheet, AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 2 })).unwrap();
    assert_approx_eq(scalar_by_key(&res_mv, "N"), xs.len() as f64, 1e-12, 1e-12, "excluded row skipped");
    assert_approx_eq(scalar_by_key(&res_mv, "Slope"), 2.0, 0.05, 0.05, "MV col 1 vs 2 slope");
}

#[test]
fn rubin_pooling_oracle_with_barnard_rubin_df() {
    let pooled =
        rubin_pool_estimates("Slope", &[1.80, 2.05, 1.95, 2.20, 2.00], &[0.04, 0.05, 0.045, 0.06, 0.055], Some(30.0), ConfidenceLevel::DEFAULT_95)
            .unwrap();
    assert_eq!(pooled.m, 5);
    assert_approx_eq(pooled.estimate, 2.0, 1e-12, 1e-12, "Rubin pooled Q_bar");
    assert_approx_eq(pooled.within_variance, 0.05, 1e-12, 1e-12, "Rubin within variance U");
    assert_approx_eq(pooled.between_variance, 0.02125, 1e-12, 1e-12, "Rubin between variance B");
    assert_approx_eq(pooled.total_variance, 0.0755, 1e-12, 1e-12, "Rubin total variance T");
    assert_approx_eq(pooled.pooled_se, 0.274772633280682, 1e-12, 1e-12, "Rubin pooled SE");
    assert_approx_eq(pooled.riv, 0.51, 1e-12, 1e-12, "Rubin RIV");
    assert_approx_eq(pooled.lambda, 0.337748344370861, 1e-12, 1e-12, "Rubin lambda");
    assert_approx_eq(pooled.df, 12.180_396_862_592_83, 1e-10, 1e-10, "Barnard-Rubin adjusted df");
    assert_approx_eq(pooled.fmi, 0.424999245862250, 1e-11, 1e-11, "Barnard-Rubin FMI");
    assert_approx_eq(pooled.t_statistic, 7.278745252468389, 1e-11, 1e-11, "Rubin t statistic");
    assert_approx_eq(pooled.p_value, 8.968027011903839e-6, 1e-10, 1e-7, "R 4.5.2 pt P value");
    assert_approx_eq(pooled.ci.0, 1.402303781090206, 1e-10, 1e-10, "R 4.5.2 qt CI lower");
    assert_approx_eq(pooled.ci.1, 2.597696218909794, 1e-10, 1e-10, "R 4.5.2 qt CI upper");

    let zero_b = rubin_pool_estimates("ZeroMiss", &[3.0, 3.0, 3.0], &[0.25, 0.25, 0.25], Some(18.0), ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(zero_b.between_variance, 0.0, 1e-12, 1e-12, "zero B");
    assert_approx_eq(zero_b.riv, 0.0, 1e-12, 1e-12, "zero RIV");
    assert_approx_eq(zero_b.fmi, 0.0, 1e-12, 1e-12, "zero FMI");
    assert_approx_eq(zero_b.df, 18.0, 1e-12, 1e-12, "complete-data df preserved when B = 0");
}

#[test]
fn mice_pmm_deterministic_reproducibility_recovery_and_zero_missingness() {
    let labels = vec![String::from("X1"), String::from("X2"), String::from("Y")];
    let n = 24;
    let (mut x1, mut x2, mut y) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let (f1, f2, eps) = (1.0 + (i as f64) * 0.5, ((i * 7) % 11) as f64 * 0.3 - 1.2, (((i * 5) % 9) as f64 - 4.0) * 0.05);
        x1.push(Some(f1));
        x2.push(Some(f2));
        y.push(Some(2.0 + 1.5 * f1 - 0.8 * f2 + eps));
    }

    let cfg_invalid_m = MultipleImputationConfig {
        imputations_m: 1,
        max_iters: 5,
        seed: 101,
        response_col: Some(2),
        predictor_cols: vec![0, 1],
        confidence: ConfidenceLevel::DEFAULT_95,
    };
    assert!(matches!(
        mice_predictive_mean_matching(&labels, &[x1.clone(), x2.clone(), y.clone()], &cfg_invalid_m),
        Err(StatsError::InvalidParameter { name: "imputations_m", .. })
    ));
    let res_zero =
        mice_predictive_mean_matching(&labels, &[x1.clone(), x2.clone(), y.clone()], &MultipleImputationConfig { imputations_m: 2, ..cfg_invalid_m })
            .unwrap();
    assert_eq!((res_zero.m, res_zero.total_missing), (2, 0));
    for pm in &res_zero.pooled_means {
        assert_approx_eq(pm.between_variance, 0.0, 1e-12, 1e-12, "zero missingness B = 0");
        assert_approx_eq(pm.fmi, 0.0, 1e-12, 1e-12, "zero missingness FMI = 0");
    }
    assert_approx_eq(res_zero.pooled_regression.as_ref().unwrap()[1].estimate, 1.5, 0.05, 0.05, "complete-case X1 slope");

    let (mut x2_mis, mut y_mis) = (x2.clone(), y.clone());
    for idx in [3, 9, 15, 20] {
        y_mis[idx] = None;
    }
    for idx in [6, 12, 18] {
        x2_mis[idx] = None;
    }
    let cfg_a = MultipleImputationConfig {
        imputations_m: 12,
        max_iters: 10,
        seed: 2026,
        response_col: Some(2),
        predictor_cols: vec![0, 1],
        confidence: ConfidenceLevel::DEFAULT_95,
    };
    let data_mis = [x1.clone(), x2_mis.clone(), y_mis.clone()];
    let res_a = mice_predictive_mean_matching(&labels, &data_mis, &cfg_a).unwrap();
    let res_a_repeat = mice_predictive_mean_matching(&labels, &data_mis, &cfg_a).unwrap();
    assert_eq!((&res_a.imputed_datasets, &res_a.pooled_means, res_a.total_missing), (&res_a_repeat.imputed_datasets, &res_a_repeat.pooled_means, 7));

    assert!(matches!(
        mice_predictive_mean_matching(&labels, &data_mis, &MultipleImputationConfig { seed: 9090, imputations_m: 80, ..cfg_a.clone() }),
        Err(StatsError::InvalidParameter { name: "imputations_m", .. })
    ));
    let res_b = mice_predictive_mean_matching(&labels, &data_mis, &MultipleImputationConfig { seed: 9090, imputations_m: 50, ..cfg_a }).unwrap();
    assert_eq!(res_b.m, 50);
    assert_ne!(res_a.imputed_datasets[0], res_b.imputed_datasets[0]);

    for ds in &res_a.imputed_datasets {
        for r in 0..n {
            if let Some(v) = x1[r] {
                assert_approx_eq(ds[r][0], v, 0.0, 0.0, "observed X1 unchanged");
            }
            if let Some(v) = x2_mis[r] {
                assert_approx_eq(ds[r][1], v, 0.0, 0.0, "observed X2 unchanged");
            } else {
                assert!(ds[r][1].is_finite() && (-1.5..=2.5).contains(&ds[r][1]));
            }
            if let Some(v) = y_mis[r] {
                assert_approx_eq(ds[r][2], v, 0.0, 0.0, "observed Y unchanged");
            } else {
                assert!(ds[r][2].is_finite());
            }
        }
    }
    let reg_a = res_a.pooled_regression.as_ref().unwrap();
    assert_approx_eq(reg_a[0].estimate, 2.0, 0.5, 0.5, "MICE recovered intercept ~ 2");
    assert_approx_eq(reg_a[1].estimate, 1.5, 0.1, 0.1, "MICE recovered X1 slope ~ 1.5");
    assert_approx_eq(reg_a[2].estimate, -0.8, 0.25, 0.25, "MICE recovered X2 slope ~ -0.8");
    assert!(reg_a[1].between_variance > 0.0 && reg_a[1].fmi > 0.0 && reg_a[2].between_variance > 0.0 && reg_a[2].fmi > 0.0);
}

#[test]
fn multiple_imputation_run_analysis_on_mv_with_label_column_and_errors() {
    let (mut id_c, mut a_c, mut b_c, mut y_c) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in 0..16 {
        let (a, b) = (10.0 + (r as f64) * 1.5, ((r * 3) % 7) as f64 - 3.0);
        id_c.push(Cell::text(format!("ID-{r:02}")));
        a_c.push(Cell::number(a));
        b_c.push(if r == 4 || r == 11 { Cell::empty() } else { Cell::number(b) });
        y_c.push(match r {
            5 => Cell::excluded_number(9999.0),
            9 => Cell::empty(),
            _ => Cell::number(5.0 + 0.8 * a + 1.2 * b),
        });
    }
    let sheet = mv_cells_sheet(
        "mi-mv",
        &[("PatientId", Some(VariableRole::Label), &id_c), ("BiomarkerA", None, &a_c), ("BiomarkerB", None, &b_c), ("Outcome", None, &y_c)],
    );

    let res_means = run_kind(
        &sheet,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 6,
            max_iters: 8,
            seed: 77,
            response_column: None,
            predictor_columns: Vec::new(),
        }),
    )
    .unwrap();
    assert_approx_eq(scalar_by_key(&res_means, "Imputations (m)"), 6.0, 1e-12, 1e-12, "m = 6 scalar");
    assert_approx_eq(scalar_by_key(&res_means, "Total missing cells"), 4.0, 1e-12, 1e-12, "4 missing cells");
    let ms = tab_by_kind(&res_means, ResultTabKind::MainSummary);
    assert_eq!(ms.row_labels, vec!["BiomarkerA", "BiomarkerB", "Outcome"]);
    assert!(matches!(ms.rows[2][0], ResultCell::Number { .. }));

    let res_reg = run_kind(
        &sheet,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 8,
            max_iters: 10,
            seed: 123,
            response_column: Some(3),
            predictor_columns: vec![1, 2],
        }),
    )
    .unwrap();
    assert_eq!(tab_by_kind(&res_reg, ResultTabKind::ParametersAndConfidenceIntervals).row_labels, vec!["Intercept", "BiomarkerA", "BiomarkerB"]);
    assert_approx_eq(scalar_by_key(&res_reg, "Reg BiomarkerA: Pooled estimate"), 0.8, 0.15, 0.15, "MV pooled BiomarkerA slope");
    assert_approx_eq(scalar_by_key(&res_reg, "Reg BiomarkerB: Pooled estimate"), 1.2, 0.25, 0.25, "MV pooled BiomarkerB slope");
    assert!(!tab_by_kind(&res_reg, ResultTabKind::Diagnostics).rows.is_empty() && res_reg.checklist.iter().any(|c| c.assumption.contains("FMI")));

    assert!(matches!(
        run_kind(
            &sheet,
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 5,
                max_iters: 5,
                seed: 1,
                response_column: Some(99),
                predictor_columns: vec![1]
            })
        ),
        Err(StatsError::MissingColumn { index: 99 })
    ));
    assert!(matches!(
        mice_predictive_mean_matching(
            &["A".into(), "B".into()],
            &[vec![Some(1.0), Some(2.0), Some(3.0)], vec![Some(5.0), None, None]],
            &MultipleImputationConfig::default()
        ),
        Err(StatsError::InsufficientSampleSize { .. })
    ));
}
