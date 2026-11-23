//! Extended D1 statistical reference oracles (`WP-B`, `WP-C`, `WP-D`).
//!
//! Covers:
//! - Two-sample Kolmogorov–Smirnov (`ks_two_sample`) with overlapping untied
//!   samples (exact Smirnov DP), cross-group ties, and larger asymptotic samples.
//! - Heteroscedastic & step-down post-hocs (`GamesHowell`, `TamhaneT2`,
//!   `DunnettT3`, `NewmanKeuls`) and Brown–Forsythe `F*` (`brown_forsythe_f_star`).
//! - Published worked examples & exact numerical checks for `levene_variance_test`
//!   (median & mean centering), `dagostino_pearson_k2`, `grubbs_outliers` (NIST
//!   e-Handbook `G = 2.4687`), and `bland_altman` (Bland & Altman *Lancet* 1986
//!   Table 1, Wright vs. Mini-Wright PEFR).
//! - Noncentral-*t* and noncentral-*F* effect-size CIs (`cohens_d_ci`,
//!   `hedges_g_ci`, `hedges_gz_ci`, `eta_squared_ci`, `partial_eta_squared_ci`).
//!
//! D1 Provenance:
//! - `R 4.5.2 (2025-10-31)` black-box reference script
//!   `crates/stats/tests/reference/stats_extended_oracles.R` (2026-11-23).
//! - NIST/SEMATECH e-Handbook of Statistical Methods (Grubbs' test worked example).
//! - Bland & Altman (1986), *Lancet* 1(8476):307–310, Table 1; Bland & Altman
//!   (1999), *Stat Methods Med Res* 8(2):135–160.
//! - D'Agostino, Belanger & D'Agostino (1990), *Am Stat* 44(4):316–321.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_math::ConfidenceLevel;
use curvebook_stats::anova::{ordinary_one_way_anova, unequal_variance_one_way_anova};
use curvebook_stats::column::{dagostino_pearson_k2, grubbs_outliers};
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, compare_groups_with_procedure};
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
// 1. WP-B: Two-sample KS (overlapping exact DP, cross-group ties, large-n)
// ============================================================================

#[test]
fn oracle_ks_two_sample_overlap_ties_and_asymptotic() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::ks.test, 2026-11-23
    // 1a. Overlapping untied samples (n1 = 8, n2 = 9) -> exact Smirnov DP (0 < D < 1)
    let x1 = [1.2, 2.5, 3.1, 4.8, 5.2, 6.9, 7.4, 8.8];
    let y1 = [2.0, 3.4, 4.1, 5.5, 6.2, 7.9, 9.1, 10.3, 11.4];
    let r1 = ks_two_sample(&x1, &y1).unwrap();
    assert!(r1.exact_p_used, "untied small samples must use exact Smirnov DP");
    assert_approx_eq(r1.d_statistic, 1.0 / 3.0, 1e-14, 0.0, "KS overlap D");
    assert_approx_eq(r1.p_value, 0.576_100_370_218_017, 1e-12, 1e-12, "KS overlap exact p");

    // 1b. Samples with cross-group ties -> asymptotic Smirnov series
    let x2 = [1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 5.0, 6.0, 7.0, 8.0];
    let y2 = [2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let r2 = ks_two_sample(&x2, &y2).unwrap();
    assert!(!r2.exact_p_used, "cross-group ties must fall back to asymptotic Smirnov series");
    assert_approx_eq(r2.d_statistic, 0.2, 1e-14, 0.0, "KS ties D");
    assert_approx_eq(r2.p_value, 0.988_261_077_643_524, 1e-10, 1e-10, "KS ties asymptotic p");

    // 1c. Larger samples (n1 = 40, n2 = 45) -> asymptotic Smirnov series
    let x3: Vec<f64> = (1..=40).map(|i| 0.5 + (i - 1) as f64 * (39.5 / 39.0) + (i as f64).sin() * 1.5).collect();
    let y3: Vec<f64> = (1..=45).map(|i| 6.0 + (i - 1) as f64 * (40.0 / 44.0) + (i as f64).cos() * 1.5).collect();
    let r3 = ks_two_sample(&x3, &y3).unwrap();
    assert!(!r3.exact_p_used, "n > 25 must use asymptotic Smirnov series");
    assert_approx_eq(r3.d_statistic, 2.0 / 9.0, 1e-14, 0.0, "KS large D");
    assert_approx_eq(r3.p_value, 0.246_537_412_090_920, 1e-10, 1e-10, "KS large asymptotic p");
}

// ============================================================================
// 2. WP-C: Heteroscedastic & step-down post-hocs + Brown-Forsythe F*
// ============================================================================

#[test]
fn oracle_brown_forsythe_and_heteroscedastic_posthocs() {
    // D1 Provenance: R 4.5.2 (2025-10-31), ptukey/qtukey/qt/integrate/pf, 2026-11-23
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];

    let groups = vec![group_input(0, "G1", &g1), group_input(1, "G2", &g2), group_input(2, "G3", &g3), group_input(3, "G4", &g4)];

    // Brown-Forsythe F* (1974b) via unequal_variance_one_way_anova
    let wbf = unequal_variance_one_way_anova(&groups).unwrap();
    assert_approx_eq(wbf.brown_forsythe_f_star, 89.887_067_108_391, 1e-10, 1e-10, "BF F*");
    assert_approx_eq(wbf.brown_forsythe_df1, 3.0, 1e-12, 0.0, "BF df1");
    assert_approx_eq(wbf.brown_forsythe_df2, 6.597_953_665_104_782, 1e-10, 1e-10, "BF df2");
    assert_approx_eq(wbf.brown_forsythe_p, 1.004_349_798_358_829e-5, 1e-12, 1e-9, "BF p");

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

    // Tamhane T2
    let t2 = compare_groups_with_procedure(&groups, 0.0, 1.0, MultiplicityProcedure::TamhaneT2, 0.05, true).unwrap();
    assert_eq!(t2.len(), 6);
    let t2_expected: &[(f64, f64, f64, f64)] = &[
        // (t, adj_p, ci_lo, ci_hi)
        (-5.244_734_605_970, 4.398_659_913_940e-3, -5.502_491_142_508, -1.139_175_524_159),
        (-7.170_003_562_507, 1.049_952_509_744e-2, -16.058_434_589_064, -3.328_232_077_602),
        (-67.742_802_188_293, 3.217_426_325_364e-13, -15.453_416_642_646, -14.013_250_024_021),
        (-4.329_528_572_906, 3.314_676_500_696e-2, -12.171_309_913_937, -0.573_690_086_063),
        (-18.369_316_551_836, 8.442_802_031_938e-7, -13.598_789_652_734, -9.226_210_347_266),
        (-3.743_318_072_488, 1.110_537_480_615e-1, -11.450_450_669_534, 1.370_450_669_534),
    ];
    for (idx, &(exp_t, exp_p, exp_lo, exp_hi)) in t2_expected.iter().enumerate() {
        assert_approx_eq(t2[idx].statistic, exp_t, 1e-10, 1e-10, &format!("T2[{idx}] t"));
        assert_approx_eq(t2[idx].adjusted_p_or_q, exp_p, 1e-12, 1e-8, &format!("T2[{idx}] p"));
        let ci = t2[idx].ci.expect("T2 CI");
        assert_approx_eq(ci.0, exp_lo, 1e-8, 1e-8, &format!("T2[{idx}] ci_lo"));
        assert_approx_eq(ci.1, exp_hi, 1e-8, 1e-8, &format!("T2[{idx}] ci_hi"));
    }

    // Dunnett T3 (studentized maximum modulus)
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

#[test]
fn oracle_newman_keuls_stepdown() {
    // D1 Provenance: R 4.5.2 (2025-10-31), aov + ptukey step-down matrix, 2026-11-23
    let a = [10.0, 11.4, 9.2, 10.8, 10.1];
    let b = [10.6, 12.1, 9.8, 11.3, 10.7];
    let c = [11.3, 12.7, 10.4, 11.9, 11.2];
    let d = [13.8, 15.2, 12.9, 14.6, 14.0];

    let groups = vec![group_input(0, "A", &a), group_input(1, "B", &b), group_input(2, "C", &c), group_input(3, "D", &d)];
    let nk = compare_groups_with_procedure(&groups, 0.73, 16.0, MultiplicityProcedure::NewmanKeuls, 0.05, false).unwrap();
    assert_eq!(nk.len(), 6);

    let expected: &[(f64, f64, bool)] = &[
        (1.570_271_767_771, 2.832_571_946_730e-1, false),
        (3.140_543_535_541, 9.790_664_179_238e-2, false),
        (9.945_054_529_214, 1.539_544_631_202e-5, true),
        (1.570_271_767_771, 2.832_571_946_730e-1, false),
        (8.374_782_761_443, 6.059_982_568_407e-5, true),
        (6.804_510_993_673, 1.917_725_193_400e-4, true),
    ];
    for (idx, &(exp_q, exp_p, exp_sig)) in expected.iter().enumerate() {
        assert_approx_eq(nk[idx].statistic, exp_q, 1e-10, 1e-10, &format!("NK[{idx}] q"));
        assert_approx_eq(nk[idx].adjusted_p_or_q, exp_p, 1e-6, 1e-4, &format!("NK[{idx}] adj_p"));
        assert_eq!(nk[idx].significant, exp_sig, "NK[{idx}] significance");
        assert!(nk[idx].ci.is_none(), "Newman-Keuls does not emit simultaneous CIs");
    }
}

// ============================================================================
// 3. WP-D: Published worked examples (Levene, D'Agostino K2, Grubbs, B-A 1986)
// ============================================================================

#[test]
fn oracle_levene_dagostino_grubbs_and_bland_altman_1986() {
    // 3a. Levene (mean-centered) & Brown-Forsythe (median-centered) on unbalanced 4-group data
    // D1 Provenance: R 4.5.2 (2025-10-31), anova(lm(abs(y - center) ~ g)), 2026-11-23
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];

    let bf_lev = levene_variance_test(&[&g1, &g2, &g3, &g4], true).unwrap();
    assert_approx_eq(bf_lev.f_statistic, 6.458_547_396_160_992, 1e-10, 1e-10, "Levene median F");
    assert_approx_eq(bf_lev.df_between, 3.0, 1e-12, 0.0, "Levene df1");
    assert_approx_eq(bf_lev.df_within, 22.0, 1e-12, 0.0, "Levene df2");
    assert_approx_eq(bf_lev.p_value, 0.002_657_922_147_045_501, 1e-10, 1e-10, "Levene median p");

    let mean_lev = levene_variance_test(&[&g1, &g2, &g3, &g4], false).unwrap();
    assert_approx_eq(mean_lev.f_statistic, 7.895_495_746_240_469, 1e-10, 1e-10, "Levene mean F");
    assert_approx_eq(mean_lev.p_value, 9.328_792_479_575_768e-4, 1e-10, 1e-10, "Levene mean p");

    // 3b. D'Agostino-Pearson omnibus K^2 (D'Agostino, Belanger & D'Agostino 1990)
    // D1 Provenance: R 4.5.2 (2025-10-31) exact formula evaluation, 2026-11-23
    let dp_norm = [9.2, 9.5, 9.7, 9.8, 9.9, 10.0, 10.1, 10.1, 10.2, 10.2, 10.3, 10.4, 10.4, 10.5, 10.6, 10.7, 10.8, 10.9, 11.1, 11.4];
    let r_norm = dagostino_pearson_k2(&dp_norm, 0.05).unwrap();
    assert_approx_eq(r_norm.statistic, 0.062_688_675_297_651_72, 1e-10, 1e-10, "D'Agostino normal K2");
    assert_approx_eq(r_norm.p_value, 0.969_141_803_604_594_2, 1e-10, 1e-10, "D'Agostino normal p");
    assert!(r_norm.passed);

    let dp_skew = [1.1, 1.2, 1.2, 1.3, 1.4, 1.5, 1.5, 1.6, 1.7, 1.8, 2.0, 2.1, 2.3, 2.6, 2.9, 3.3, 3.8, 4.5, 5.4, 6.8, 8.5, 10.9, 14.2, 19.0, 26.5];
    let r_skew = dagostino_pearson_k2(&dp_skew, 0.05).unwrap();
    assert_approx_eq(r_skew.statistic, 24.110_748_828_032_06, 1e-10, 1e-10, "D'Agostino skewed K2");
    assert_approx_eq(r_skew.p_value, 5.813_228_776_448_106e-6, 1e-11, 1e-9, "D'Agostino skewed p");
    assert!(!r_skew.passed);

    // 3c. NIST/SEMATECH e-Handbook Grubbs worked example (n = 8, certified G = 2.4687, G_crit = 2.1266)
    let grubbs_nist = [199.31, 199.53, 200.19, 200.82, 201.92, 201.95, 202.18, 245.57];
    let gr = grubbs_outliers(&grubbs_nist, 0.05, false).unwrap();
    assert_eq!(gr.outliers.len(), 1);
    assert_eq!(gr.outliers[0].index, 7);
    assert_approx_eq(gr.outliers[0].value, 245.57, 1e-12, 0.0, "Grubbs NIST outlier value");
    assert_approx_eq(gr.outliers[0].score, 2.468_764_611_212, 1e-10, 1e-10, "Grubbs NIST G");
    assert_approx_eq(gr.outliers[0].threshold_or_p, 2.126_645_087_195, 1e-10, 1e-10, "Grubbs NIST G_crit");

    // 3d. Bland & Altman (Lancet 1986, Table 1, n = 17 Wright vs Mini-Wright PEFR)
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
// 4. WP-D: Exact noncentral-t and noncentral-F effect-size CIs
// ============================================================================

#[test]
fn oracle_noncentral_t_and_f_effect_size_cis() {
    // D1 Provenance: R 4.5.2 (2025-10-31), pt(t, df, ncp) & pf(F, df1, df2, ncp) + uniroot, 2026-11-23
    // 4a. Unpaired two-group moderate effect
    let ea = [12.4, 13.1, 11.8, 12.9, 13.5, 12.2, 12.7];
    let eb = [10.1, 10.8, 9.7, 11.0, 10.4, 10.2, 9.9, 10.6];
    let t_mod = unpaired_t_test(&ea, &eb, true, conf_95()).unwrap();
    let es = &t_mod.effect_sizes;
    assert_approx_eq(es.cohens_d, 4.551_960_375_228, 1e-10, 1e-10, "unpaired Cohen's d");
    let d_ci = es.cohens_d_ci.expect("cohens_d_ci");
    assert_approx_eq(d_ci.0, 2.525_161_580_857, 1e-6, 1e-6, "unpaired Cohen's d CI lo");
    assert_approx_eq(d_ci.1, 6.534_660_813_414, 1e-6, 1e-6, "unpaired Cohen's d CI hi");
    assert_approx_eq(es.hedges_g, 4.283_314_905_712, 1e-10, 1e-10, "unpaired Hedges' g");
    let g_ci = es.hedges_g_ci.expect("hedges_g_ci");
    assert_approx_eq(g_ci.0, 2.376_132_775_119, 1e-6, 1e-6, "unpaired Hedges' g CI lo");
    assert_approx_eq(g_ci.1, 6.149_001_256_293, 1e-6, 1e-6, "unpaired Hedges' g CI hi");

    // 4b. Unpaired near-zero effect (t == 0 -> symmetric CI around 0)
    let za = [10.0, 10.5, 9.5, 10.2, 9.8, 10.1];
    let zb = [10.1, 9.9, 10.4, 9.6, 10.2, 9.9];
    let t_zero = unpaired_t_test(&za, &zb, true, conf_95()).unwrap();
    assert_approx_eq(t_zero.effect_sizes.cohens_d, 0.0, 1e-12, 0.0, "near-zero Cohen's d");
    let dz_ci = t_zero.effect_sizes.cohens_d_ci.expect("near-zero cohens_d_ci");
    assert_approx_eq(dz_ci.0, -1.131_585_734_076, 1e-6, 1e-6, "near-zero d CI lo");
    assert_approx_eq(dz_ci.1, 1.131_585_734_076, 1e-6, 1e-6, "near-zero d CI hi");

    // 4c. Paired Hedges' g_z CI
    let pa = [120.0, 132.0, 118.0, 140.0, 125.0, 135.0, 128.0, 144.0];
    let pb = [112.0, 124.0, 115.0, 129.0, 121.0, 126.0, 120.0, 133.0];
    let pt = paired_t_test(&pa, &pb, true, conf_95()).unwrap();
    assert_approx_eq(pt.cohens_dz, 2.658_228_069_709, 1e-10, 1e-10, "paired dz");
    assert_approx_eq(pt.hedges_gz.expect("hedges_gz"), 2.361_045_900_773, 1e-10, 1e-10, "paired gz");
    let gz_ci = pt.hedges_gz_ci.expect("hedges_gz_ci");
    assert_approx_eq(gz_ci.0, 0.991_429_193_722, 1e-6, 1e-6, "paired gz CI lo");
    assert_approx_eq(gz_ci.1, 3.703_489_594_935, 1e-6, 1e-6, "paired gz CI hi");

    // 4d. One-way ANOVA noncentral-F eta_squared_ci (significant F >> 1)
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

    // 4e. One-way ANOVA low-F (0 < F < 1 -> lower CI clamped to 0.0, upper CI > 0)
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
