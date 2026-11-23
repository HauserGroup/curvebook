//! Extended D1 statistical reference oracles backed strictly by independent
//! black-box implementations (`R 4.5.2`, `SciPy 1.18.1`, `statsmodels 0.15.0`)
//! and published worked examples (no white-mirror formula transcriptions).
//!
//! Covers:
//! - Two-sample Kolmogorov–Smirnov (`ks_two_sample`) with overlapping untied
//!   samples (exact Smirnov DP), cross-group ties, and larger asymptotic samples
//!   (`R 4.5.2 stats::ks.test`).
//! - Heteroscedastic post-hoc distributions: `GamesHowell` (direct 2D quadrature
//!   + `qtukey`) and `DunnettT3` (adaptive SMM quadrature via `integrate`).
//! - Published worked examples: `levene_variance_test` (`scipy.stats.levene` &
//!   `R anova(lm)`), `grubbs_outliers` (NIST e-Handbook `G = 2.4687`), and
//!   `bland_altman` (Bland & Altman *Lancet* 1986 Table 1).
//! - Normality tests against independent `SciPy 1.18.1` + `statsmodels 0.15.0`:
//!   `dagostino_pearson_k2` (`scipy.stats.normaltest`), `anderson_darling`
//!   (`scipy.stats.anderson` + `statsmodels normal_ad`), and `lilliefors_ks`
//!   (`statsmodels.stats.diagnostic.lilliefors`).
//! - Two-stage BKY FDR (`adjust_two_stage_bky`) against
//!   `statsmodels.stats.multitest.multipletests(method="fdr_tsbky")`.
//! - Cohen's kappa (`cohens_kappa` unweighted, linear, quadratic + SE/z/p/CI)
//!   against `statsmodels.stats.inter_rater.cohens_kappa`.
//! - Noncentral-*t* and noncentral-*F* effect-size CIs (`cohens_d_ci`,
//!   `hedges_g_ci`, `hedges_gz_ci`, `eta_squared_ci`) against `R 4.5.2` `pt`/`pf`
//!   and `SciPy 1.18.1` `scipy.stats.nct`.
//!
//! D1 Provenance:
//! - `R 4.5.2 (2025-10-31)` via `crates/stats/tests/reference/stats_extended_oracles.R` (2026-11-23).
//! - `SciPy 1.18.1` + `statsmodels 0.15.0` via `crates/stats/tests/reference/scipy_statsmodels_oracles.py` (2026-11-23).
//! - NIST/SEMATECH e-Handbook of Statistical Methods §1.3.5.17.1 (Grubbs 1969, *Technometrics* 11(1):1–21, doi:10.1080/00401706.1969.10490657).
//! - Bland & Altman (1986), *Lancet* 327(8476):307–310 (doi:10.1016/s0140-6736(86)90837-8), Table 1;
//!   Bland & Altman (1999), *Stat Methods Med Res* 8(2):135–160 (doi:10.1177/096228029900800204).
//! - Levene (1960), in I. Olkin (Ed.), *Contributions to Probability and Statistics*, Stanford UP, pp. 278–292;
//!   Brown & Forsythe (1974a), *JASA* 69(346):364–367 (doi:10.1080/01621459.1974.10482955).
//! - Games & Howell (1976), *J Educ Stat* 1(2):113–125 (doi:10.3102/10769986001002113);
//!   Dunnett (1980), *JASA* 75(372):796–800 (doi:10.1080/01621459.1980.10477552).
//! - D'Agostino, Belanger & D'Agostino (1990), *Am Stat* 44(4):316–321 (doi:10.1080/00031305.1990.10475751);
//!   Stephens (1986), in D'Agostino & Stephens (Eds.), *Goodness-of-Fit Techniques*, Marcel Dekker, pp. 97–193;
//!   Lilliefors (1967), *JASA* 62(318):399–402 (doi:10.1080/01621459.1967.10482916).
//! - Benjamini, Krieger & Yekutieli (2006), *Biometrika* 93(3):491–507 (doi:10.1093/biomet/93.3.491).
//! - Cohen (1960), *Educ Psychol Meas* 20(1):37–46 (doi:10.1177/001316446002000104);
//!   Cohen (1968), *Psychol Bull* 70(4):213–220 (doi:10.1037/h0026256);
//!   Fleiss, Cohen & Everitt (1969), *Psychol Bull* 72(5):323–327 (doi:10.1037/h0028106).
//! - Cohen (1988), *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.), Lawrence Erlbaum;
//!   Hedges (1981), *J Educ Stat* 6(2):107–128 (doi:10.3102/10769986006002107);
//!   Steiger & Fouladi (1997), in Harlow, Mulaik & Steiger (Eds.), *What If There Were No Significance Tests?*, Erlbaum, pp. 221–257;
//!   Steiger (2004), *Psychol Methods* 9(2):164–182 (doi:10.1037/1082-989X.9.2.164).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::KappaWeighting;
use curvebook_math::ConfidenceLevel;
use curvebook_stats::anova::ordinary_one_way_anova;
use curvebook_stats::categorical::cohens_kappa;
use curvebook_stats::column::{anderson_darling, dagostino_pearson_k2, grubbs_outliers, lilliefors_ks};
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, adjust_two_stage_bky, compare_groups_with_procedure};
use curvebook_stats::two_group::{bland_altman, ks_two_sample, levene_variance_test, paired_t_test, unpaired_t_test};
use curvebook_testkit::assert_approx_eq;

fn conf_95() -> ConfidenceLevel {
    ConfidenceLevel::DEFAULT_95
}

fn group_input(index: usize, label: &str, data: &[f64]) -> GroupComparisonInput {
    let n = data.len() as f64;
    let mean = data.iter().sum::<f64>() / n;
    let var = data.iter().map(|&x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    GroupComparisonInput { index, label: label.to_owned(), n, mean, sd: var.sqrt() }
}

// ============================================================================
// 1. Two-sample KS (overlapping exact DP, cross-group ties, large-n)
// ============================================================================

#[test]
fn oracle_ks_two_sample_overlap_ties_and_asymptotic() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::ks.test, 2026-11-23
    let x1 = [1.2, 2.5, 3.1, 4.8, 5.2, 6.9, 7.4, 8.8];
    let y1 = [2.0, 3.4, 4.1, 5.5, 6.2, 7.9, 9.1, 10.3, 11.4];
    let r1 = ks_two_sample(&x1, &y1).unwrap();
    assert!(r1.exact_p_used, "untied small samples must use exact Smirnov DP");
    assert_approx_eq(r1.d_statistic, 1.0 / 3.0, 1e-14, 0.0, "KS overlap D");
    assert_approx_eq(r1.p_value, 0.576_100_370_218_017, 1e-12, 1e-12, "KS overlap exact p");

    let x2 = [1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 5.0, 6.0, 7.0, 8.0];
    let y2 = [2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let r2 = ks_two_sample(&x2, &y2).unwrap();
    assert!(!r2.exact_p_used, "cross-group ties must fall back to asymptotic Smirnov series");
    assert_approx_eq(r2.d_statistic, 0.2, 1e-14, 0.0, "KS ties D");
    assert_approx_eq(r2.p_value, 0.988_261_077_643_524, 1e-10, 1e-10, "KS ties asymptotic p");

    let x3: Vec<f64> = (1..=40).map(|i| 0.5 + (i - 1) as f64 * (39.5 / 39.0) + (i as f64).sin() * 1.5).collect();
    let y3: Vec<f64> = (1..=45).map(|i| 6.0 + (i - 1) as f64 * (40.0 / 44.0) + (i as f64).cos() * 1.5).collect();
    let r3 = ks_two_sample(&x3, &y3).unwrap();
    assert!(!r3.exact_p_used, "n > 25 must use asymptotic Smirnov series");
    assert_approx_eq(r3.d_statistic, 2.0 / 9.0, 1e-14, 0.0, "KS large D");
    assert_approx_eq(r3.p_value, 0.246_537_412_090_920, 1e-10, 1e-10, "KS large asymptotic p");
}

// ============================================================================
// 2. Heteroscedastic post-hoc distributions: Games-Howell & Dunnett T3
// ============================================================================

#[test]
fn oracle_heteroscedastic_posthocs_games_howell_and_dunnett_t3() {
    // D1 Provenance: R 4.5.2 (2025-10-31), t.test(var.equal=F) + direct 2D quadrature + qtukey + SMM integrate, 2026-11-23
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];

    let groups = vec![group_input(0, "G1", &g1), group_input(1, "G2", &g2), group_input(2, "G3", &g3), group_input(3, "G4", &g4)];

    // Games-Howell (p-values checked against exact 2D studentized-range quadrature in R 4.5.2,
    // which avoids base R `ptukey`'s known tail truncation below ~1e-5; CIs checked against `qtukey`).
    let gh = compare_groups_with_procedure(&groups, 0.0, 1.0, MultiplicityProcedure::GamesHowell, 0.05, true).unwrap();
    assert_eq!(gh.len(), 6);
    let gh_expected: &[(f64, f64, f64, f64, f64)] = &[
        // (q, df, adj_p, ci_lo, ci_hi)
        (7.417_174_810_810, 8.146_636_829_720, 3.267_449_878_592e-3, -5.340_097_282_946, -1.301_569_383_721),
        (10.139_916_280_361, 4.138_739_889_497, 6.162_672_356_169e-3, -15.105_843_183_570, -4.280_823_483_096),
        (95.802_789_607_841, 9.429_979_187_019, 2.692_152_156_284e-13, -15.406_051_548_208, -14.060_615_118_458),
        (6.122_878_026_486, 5.678_651_507_584, 2.135_565_282_652e-2, -11.559_846_185_201, -1.185_153_814_799),
        (25.978_136_599_132, 7.609_346_396_973, 6.538_939_527_407e-7, -13.425_782_372_742, -9.399_217_627_258),
        (5.293_851_186_388, 4.072_198_222_085, 6.353_991_737_180e-2, -10.472_822_419_763, 0.392_822_419_763),
    ];
    for (idx, &(exp_q, exp_df, exp_p, exp_lo, exp_hi)) in gh_expected.iter().enumerate() {
        assert_approx_eq(gh[idx].statistic, exp_q, 1e-10, 1e-10, &format!("GH[{idx}] q"));
        assert_approx_eq(gh[idx].df, exp_df, 1e-10, 1e-10, &format!("GH[{idx}] df"));
        assert_approx_eq(gh[idx].adjusted_p_or_q, exp_p, 1e-15, 1e-8, &format!("GH[{idx}] p"));
        let ci = gh[idx].ci.expect("GH CI");
        assert_approx_eq(ci.0, exp_lo, 1e-5, 1e-5, &format!("GH[{idx}] ci_lo"));
        assert_approx_eq(ci.1, exp_hi, 1e-5, 1e-5, &format!("GH[{idx}] ci_hi"));
    }
    assert!(!gh[5].significant, "pair (2,3) should not be significant at alpha=0.05");

    // Dunnett T3 (independent adaptive SMM quadrature E_S[(2*Phi(t*S) - 1)^m] in R vs CurveBook's
    // Hartley folded-range Gauss-Legendre `max_studentized_range_sf`).
    let dt3 = compare_groups_with_procedure(&groups, 0.0, 1.0, MultiplicityProcedure::DunnettT3, 0.05, true).unwrap();
    assert_eq!(dt3.len(), 6);
    let dt3_expected: &[(f64, f64, f64)] = &[
        // (adj_p, ci_lo, ci_hi)
        (3.974_864_581_717e-3, -5.441_616_166_633, -1.200_050_500_033),
        (7.716_267_033_320e-3, -15.491_300_117_842, -3.895_366_548_825),
        (3.079_758_670_310e-13, -15.437_641_958_183, -14.029_024_708_484),
        (2.680_271_742_768e-2, -11.872_326_839_698, -0.872_673_160_302),
        (7.785_004_214_611e-7, -13.530_390_121_502, -9.294_609_878_498),
        (7.990_866_442_105e-2, -10.863_091_679_514, 0.783_091_679_514),
    ];
    for (idx, &(exp_p, exp_lo, exp_hi)) in dt3_expected.iter().enumerate() {
        let p_rel = if exp_p < 1e-8 { 5e-2 } else { 1e-4 };
        assert_approx_eq(dt3[idx].adjusted_p_or_q, exp_p, 1e-11, p_rel, &format!("DT3[{idx}] p"));
        let ci = dt3[idx].ci.expect("DT3 CI");
        assert_approx_eq(ci.0, exp_lo, 5e-4, 1e-4, &format!("DT3[{idx}] ci_lo"));
        assert_approx_eq(ci.1, exp_hi, 5e-4, 1e-4, &format!("DT3[{idx}] ci_hi"));
    }
}

// ============================================================================
// 3. Published worked examples & SciPy Levene (Levene, Grubbs, Bland-Altman)
// ============================================================================

#[test]
fn oracle_levene_grubbs_and_bland_altman_1986() {
    // 3a. Levene (mean-centered) & Brown-Forsythe (median-centered)
    // D1 Provenance: SciPy 1.18.1 `scipy.stats.levene` & R 4.5.2 `anova(lm)`, 2026-11-23
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];

    let bf_lev = levene_variance_test(&[&g1, &g2, &g3, &g4], true).unwrap();
    assert_approx_eq(bf_lev.f_statistic, 6.458_547_396_160_991, 1e-10, 1e-10, "Levene median F");
    assert_approx_eq(bf_lev.df_between, 3.0, 1e-12, 0.0, "Levene df1");
    assert_approx_eq(bf_lev.df_within, 22.0, 1e-12, 0.0, "Levene df2");
    assert_approx_eq(bf_lev.p_value, 2.657_922_147_045_504e-3, 1e-10, 1e-10, "Levene median p");

    let mean_lev = levene_variance_test(&[&g1, &g2, &g3, &g4], false).unwrap();
    assert_approx_eq(mean_lev.f_statistic, 7.895_495_746_240_47, 1e-10, 1e-10, "Levene mean F");
    assert_approx_eq(mean_lev.p_value, 9.328_792_479_575_741e-4, 1e-10, 1e-10, "Levene mean p");

    // 3b. NIST/SEMATECH e-Handbook Grubbs worked example (n = 8, certified G = 2.4687, G_crit = 2.1266)
    let grubbs_nist = [199.31, 199.53, 200.19, 200.82, 201.92, 201.95, 202.18, 245.57];
    let gr = grubbs_outliers(&grubbs_nist, 0.05, false).unwrap();
    assert_eq!(gr.outliers.len(), 1);
    assert_eq!(gr.outliers[0].index, 7);
    assert_approx_eq(gr.outliers[0].value, 245.57, 1e-12, 0.0, "Grubbs NIST outlier value");
    assert_approx_eq(gr.outliers[0].score, 2.468_764_611_212, 1e-10, 1e-10, "Grubbs NIST G");
    assert_approx_eq(gr.outliers[0].threshold_or_p, 2.126_645_087_195, 1e-10, 1e-10, "Grubbs NIST G_crit");

    // 3c. Bland & Altman (Lancet 1986, Table 1, n = 17 Wright vs Mini-Wright PEFR)
    let wright = [494.0, 395.0, 516.0, 434.0, 476.0, 557.0, 413.0, 442.0, 650.0, 433.0, 417.0, 656.0, 267.0, 478.0, 178.0, 423.0, 427.0];
    let mini = [512.0, 430.0, 520.0, 428.0, 500.0, 600.0, 364.0, 380.0, 658.0, 445.0, 432.0, 626.0, 260.0, 477.0, 259.0, 350.0, 451.0];
    let ba = bland_altman(&wright, &mini, false, conf_95()).unwrap();
    assert_approx_eq(ba.bias, -2.117_647_058_824, 1e-10, 1e-10, "BA 1986 bias");
    assert_approx_eq(ba.sd_bias, 38.765_129_873_607, 1e-10, 1e-10, "BA 1986 sd");
    assert_approx_eq(ba.loa_lower, -78.095_905_467_112, 1e-9, 1e-9, "BA 1986 loa_lower");
    assert_approx_eq(ba.loa_upper, 73.860_611_349_465, 1e-9, 1e-9, "BA 1986 loa_upper");
    assert_approx_eq(ba.bias_ci.0, -22.048_837_696_645, 1e-9, 1e-9, "BA 1986 bias_ci lo");
    assert_approx_eq(ba.bias_ci.1, 17.813_543_578_998, 1e-9, 1e-9, "BA 1986 bias_ci hi");
    assert_approx_eq(ba.loa_lower_ci.0, -112.851_553_139_311, 1e-9, 1e-9, "BA 1986 loa_lo_ci lo");
    assert_approx_eq(ba.loa_lower_ci.1, -43.340_257_794_913, 1e-9, 1e-9, "BA 1986 loa_lo_ci hi");
    assert_approx_eq(ba.loa_upper_ci.0, 39.104_963_677_266, 1e-9, 1e-9, "BA 1986 loa_hi_ci lo");
    assert_approx_eq(ba.loa_upper_ci.1, 108.616_259_021_664, 1e-9, 1e-9, "BA 1986 loa_hi_ci hi");
}

// ============================================================================
// 4. SciPy 1.18.1 + statsmodels 0.15.0: D'Agostino K^2, Anderson-Darling, Lilliefors
// ============================================================================

#[test]
fn oracle_scipy_statsmodels_normality_dagostino_ad_lilliefors() {
    // D1 Provenance: SciPy 1.18.1 (`scipy.stats.normaltest`, `scipy.stats.anderson`) and
    // statsmodels 0.15.0 (`normal_ad`, `lilliefors`), 2026-11-23
    let dp_norm = [9.2, 9.5, 9.7, 9.8, 9.9, 10.0, 10.1, 10.1, 10.2, 10.2, 10.3, 10.4, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 11.1, 11.4];
    let dp_mid = [1.0, 1.3, 1.5, 1.8, 2.0, 2.2, 2.5, 2.8, 3.1, 3.5, 4.0, 4.6, 5.3, 6.2, 7.5, 9.2];
    let dp_skew = [1.1, 1.2, 1.2, 1.3, 1.4, 1.5, 1.5, 1.6, 1.7, 1.8, 2.0, 2.1, 2.3, 2.6, 2.9, 3.3, 3.8, 4.5, 5.4, 6.8, 8.5, 10.9, 14.2, 19.0, 26.5];

    // 4a. D'Agostino-Pearson K^2 vs scipy.stats.normaltest
    let r_norm = dagostino_pearson_k2(&dp_norm, 0.05).unwrap();
    assert_approx_eq(r_norm.statistic, 6.268_867_529_764_53e-2, 1e-10, 1e-10, "scipy normaltest norm K2");
    assert_approx_eq(r_norm.p_value, 9.691_418_036_045_973e-1, 1e-10, 1e-10, "scipy normaltest norm p");
    assert!(r_norm.passed);

    let r_skew = dagostino_pearson_k2(&dp_skew, 0.05).unwrap();
    assert_approx_eq(r_skew.statistic, 2.411_074_882_803_205e1, 1e-10, 1e-10, "scipy normaltest skew K2");
    assert_approx_eq(r_skew.p_value, 5.813_228_776_448_127e-6, 1e-11, 1e-9, "scipy normaltest skew p");
    assert!(!r_skew.passed);

    // 4b. Anderson-Darling A2* vs scipy.stats.anderson + statsmodels normal_ad
    let ad_norm = anderson_darling(&dp_norm, 0.05).unwrap();
    assert_approx_eq(ad_norm.statistic, 8.149_615_743_467_194e-2, 1e-11, 1e-11, "AD norm A2*");
    assert_approx_eq(ad_norm.p_value, 9.987_434_797_717_529e-1, 1e-6, 1e-6, "AD norm p");
    assert!(ad_norm.passed);

    let ad_skew = anderson_darling(&dp_skew, 0.05).unwrap();
    assert_approx_eq(ad_skew.statistic, 3.263_284_860_426_419, 1e-11, 1e-11, "AD skew A2*");
    assert_approx_eq(ad_skew.p_value, 3.605_135_649_092_591e-8, 1e-10, 1e-4, "AD skew p");
    assert!(!ad_skew.passed);

    // 4c. Lilliefors KS end-to-end vs statsmodels.stats.diagnostic.lilliefors
    let lf_norm = lilliefors_ks(&dp_norm, 0.05).unwrap();
    assert_approx_eq(lf_norm.statistic, 6.956_126_979_099_897e-2, 1e-12, 1e-12, "Lilliefors norm D");
    assert!(lf_norm.passed);

    let lf_mid = lilliefors_ks(&dp_mid, 0.05).unwrap();
    assert_approx_eq(lf_mid.statistic, 1.556_038_506_772_208e-1, 1e-12, 1e-12, "Lilliefors mid D");
    // statsmodels uses Monte-Carlo-tabulated `pvalmethod="table"`, while CurveBook uses Dallal-Wilkinson /
    // Stephens analytic approximation; both agree within ~0.015 in the mid-range.
    assert_approx_eq(lf_mid.p_value, 3.794_646_176_550_191e-1, 2e-2, 5e-2, "Lilliefors mid p");
    assert!(lf_mid.passed);

    let lf_skew = lilliefors_ks(&dp_skew, 0.05).unwrap();
    assert_approx_eq(lf_skew.statistic, 2.653_739_463_886_268e-1, 1e-12, 1e-12, "Lilliefors skew D");
    assert!(lf_skew.p_value < 0.002, "Lilliefors skew p={} must reject strongly", lf_skew.p_value);
    assert!(!lf_skew.passed);
}

// ============================================================================
// 5. statsmodels 0.15.0: Two-stage BKY FDR & Cohen's Kappa (unweighted/linear/quadratic)
// ============================================================================

#[test]
fn oracle_statsmodels_two_stage_bky_fdr() {
    // D1 Provenance: statsmodels 0.15.0 `multipletests(..., method="fdr_tsbky")`, 2026-11-23
    let raw_p = [0.001, 0.004, 0.012, 0.031, 0.049, 0.120, 0.450, 0.890];

    let q05 = adjust_two_stage_bky(&raw_p, 0.05);
    let exp_q05 = [5.25e-3, 1.05e-2, 2.1e-2, 4.068_75e-2, 5.145e-2, 1.05e-1, 3.375e-1, 5.840_625e-1];
    for (i, (&act, &exp)) in q05.iter().zip(exp_q05.iter()).enumerate() {
        assert_approx_eq(act, exp, 1e-12, 1e-12, &format!("BKY(q=0.05)[{i}]"));
    }

    let q10 = adjust_two_stage_bky(&raw_p, 0.10);
    let exp_q10 = [3.3e-3, 6.6e-3, 1.32e-2, 2.557_5e-2, 3.234e-2, 6.6e-2, 2.121_428_571_428_572e-1, 3.671_25e-1];
    for (i, (&act, &exp)) in q10.iter().zip(exp_q10.iter()).enumerate() {
        assert_approx_eq(act, exp, 1e-12, 1e-12, &format!("BKY(q=0.10)[{i}]"));
    }
}

#[test]
fn oracle_statsmodels_cohens_kappa_unweighted_linear_quadratic() {
    // D1 Provenance: statsmodels 0.15.0 `inter_rater.cohens_kappa`, 2026-11-23
    struct KappaCase {
        wt: KappaWeighting,
        label: &'static str,
        kappa: f64,
        se: f64,
        z: f64,
        p: f64,
        ci_lo: f64,
        ci_hi: f64,
    }
    let table = vec![vec![22, 5, 1], vec![3, 18, 4], vec![0, 2, 15]];

    let cases = [
        KappaCase {
            wt: KappaWeighting::Unweighted,
            label: "unweighted",
            kappa: 6.754_250_386_398_762e-1,
            se: 7.433_558_336_166_253e-2,
            z: 7.966_645_601_286_56,
            p: 1.630_393_662_604_256e-15,
            ci_lo: 5.297_299_724_812_428e-1,
            ci_hi: 8.211_201_047_985_097e-1,
        },
        KappaCase {
            wt: KappaWeighting::Linear,
            label: "linear",
            kappa: 7.345_971_563_981_043e-1,
            se: 6.451_325_551_304_678e-2,
            z: 7.892_213_696_868_386,
            p: 2.968_728_132_651_254e-15,
            ci_lo: 6.081_534_990_671_025e-1,
            ci_hi: 8.610_408_137_291_061e-1,
        },
        KappaCase {
            wt: KappaWeighting::Quadratic,
            label: "quadratic",
            kappa: 7.964_458_804_523_425e-1,
            se: 5.933_748_437_711_975e-2,
            z: 6.703_340_745_790_584,
            p: 2.037_076_870_995_326e-11,
            ci_lo: 6.801_465_481_399_798e-1,
            ci_hi: 9.127_452_127_647_053e-1,
        },
    ];

    for c in &cases {
        let res = cohens_kappa(&table, c.wt, conf_95()).unwrap();
        assert_approx_eq(res.kappa, c.kappa, 1e-12, 1e-12, &format!("kappa {}", c.label));
        assert_approx_eq(res.se, c.se, 1e-11, 1e-11, &format!("kappa {} se", c.label));
        assert_approx_eq(res.z_statistic, c.z, 1e-10, 1e-10, &format!("kappa {} z", c.label));
        assert_approx_eq(res.p_value, c.p, 1e-14, 1e-8, &format!("kappa {} p", c.label));
        assert_approx_eq(res.ci.0, c.ci_lo, 1e-10, 1e-10, &format!("kappa {} ci lo", c.label));
        assert_approx_eq(res.ci.1, c.ci_hi, 1e-10, 1e-10, &format!("kappa {} ci hi", c.label));
    }
}

// ============================================================================
// 6. Exact noncentral-t and noncentral-F effect-size CIs
// ============================================================================

#[test]
fn oracle_noncentral_t_and_f_effect_size_cis() {
    // D1 Provenance: R 4.5.2 `pt`/`pf` + `uniroot` & SciPy 1.18.1 `scipy.stats.nct` + `brentq`, 2026-11-23
    let ea = [12.4, 13.1, 11.8, 12.9, 13.5, 12.2, 12.7];
    let eb = [10.1, 10.8, 9.7, 11.0, 10.4, 10.2, 9.9, 10.6];
    let t_mod = unpaired_t_test(&ea, &eb, true, conf_95()).unwrap();
    let es = &t_mod.effect_sizes;
    assert_approx_eq(es.cohens_d, 4.551_960_375_228, 1e-10, 1e-10, "unpaired Cohen's d");
    let d_ci = es.cohens_d_ci.expect("cohens_d_ci");
    assert_approx_eq(d_ci.0, 2.525_161_580_869, 1e-6, 1e-6, "unpaired Cohen's d CI lo");
    assert_approx_eq(d_ci.1, 6.534_660_813_427, 1e-6, 1e-6, "unpaired Cohen's d CI hi");
    assert_approx_eq(es.hedges_g, 4.283_314_905_712, 1e-10, 1e-10, "unpaired Hedges' g");
    let g_ci = es.hedges_g_ci.expect("hedges_g_ci");
    assert_approx_eq(g_ci.0, 2.376_132_775_119, 1e-6, 1e-6, "unpaired Hedges' g CI lo");
    assert_approx_eq(g_ci.1, 6.149_001_256_293, 1e-6, 1e-6, "unpaired Hedges' g CI hi");

    let za = [10.0, 10.5, 9.5, 10.2, 9.8, 10.1];
    let zb = [10.1, 9.9, 10.4, 9.6, 10.2, 9.9];
    let t_zero = unpaired_t_test(&za, &zb, true, conf_95()).unwrap();
    assert_approx_eq(t_zero.effect_sizes.cohens_d, 0.0, 1e-12, 0.0, "near-zero Cohen's d");
    let dz_ci = t_zero.effect_sizes.cohens_d_ci.expect("near-zero cohens_d_ci");
    assert_approx_eq(dz_ci.0, -1.131_585_734_076, 1e-6, 1e-6, "near-zero d CI lo");
    assert_approx_eq(dz_ci.1, 1.131_585_734_076, 1e-6, 1e-6, "near-zero d CI hi");

    let pa = [120.0, 132.0, 118.0, 140.0, 125.0, 135.0, 128.0, 144.0];
    let pb = [112.0, 124.0, 115.0, 129.0, 121.0, 126.0, 120.0, 133.0];
    let pt = paired_t_test(&pa, &pb, true, conf_95()).unwrap();
    assert_approx_eq(pt.cohens_dz, 2.658_228_069_709, 1e-10, 1e-10, "paired dz");
    assert_approx_eq(pt.hedges_gz.expect("hedges_gz"), 2.361_045_900_773, 1e-10, 1e-10, "paired gz");
    let gz_ci = pt.hedges_gz_ci.expect("hedges_gz_ci");
    assert_approx_eq(gz_ci.0, 0.991_429_193_722, 1e-6, 1e-6, "paired gz CI lo");
    assert_approx_eq(gz_ci.1, 3.703_489_594_935, 1e-6, 1e-6, "paired gz CI hi");

    let a = [10.0, 11.4, 9.2, 10.8, 10.1];
    let b = [10.6, 12.1, 9.8, 11.3, 10.7];
    let c = [11.3, 12.7, 10.4, 11.9, 11.2];
    let d = [13.8, 15.2, 12.9, 14.6, 14.0];
    let ow_sig = ordinary_one_way_anova(&[("A", &a[..]), ("B", &b[..]), ("C", &c[..]), ("D", &d[..])], conf_95()).unwrap();
    assert_approx_eq(ow_sig.f_statistic, 19.178_082_191_781, 1e-10, 1e-10, "ANOVA sig F");
    assert_approx_eq(ow_sig.effect_sizes.eta_squared, 0.782_414_307_004, 1e-10, 1e-10, "ANOVA sig eta2");
    let eta_ci = ow_sig.effect_sizes.eta_squared_ci.expect("ANOVA sig eta2_ci");
    assert_approx_eq(eta_ci.0, 0.452_940_337_112, 1e-6, 1e-6, "ANOVA sig eta2 CI lo");
    assert_approx_eq(eta_ci.1, 0.851_353_053_380, 1e-6, 1e-6, "ANOVA sig eta2 CI hi");

    let la = [10.0, 11.2, 9.4, 10.6, 9.8];
    let lb = [10.5, 11.1, 9.7, 10.8, 10.2];
    let lc = [9.9, 11.0, 9.6, 10.4, 10.1];
    let ow_low = ordinary_one_way_anova(&[("A", &la[..]), ("B", &lb[..]), ("C", &lc[..])], conf_95()).unwrap();
    assert_approx_eq(ow_low.f_statistic, 0.313_543_599_258, 1e-10, 1e-10, "ANOVA low-F F");
    assert_approx_eq(ow_low.effect_sizes.eta_squared, 0.049_662_062_886, 1e-10, 1e-10, "ANOVA low-F eta2");
    let eta_low_ci = ow_low.effect_sizes.eta_squared_ci.expect("ANOVA low-F eta2_ci");
    assert_approx_eq(eta_low_ci.0, 0.0, 1e-12, 0.0, "ANOVA low-F eta2 CI lo");
    assert_approx_eq(eta_low_ci.1, 0.271_778_259_118, 1e-6, 1e-6, "ANOVA low-F eta2 CI hi");
}
