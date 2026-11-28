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
//! - Generalized power-mean CIs (`harmonic_mean_ci`, `quadratic_mean_ci` in
//!   `summarize_column`) against `R 4.5.2 stats::t.test` and independent Python
//!   Simpson quadrature on the Student-*t* PDF (Bland & Altman 1996; Lam, Hung &
//!   Perrier 1985; Casella & Berger 2002; Bullen 2003).
//! - Remaining base-R `4.5.2` statistical oracles (`Brown–Forsythe F*` with
//!   Satterthwaite df, ANOVA $\omega^2$ & Steiger noncentral-*F* CIs, `mean_ci`
//!   vs `t.test`, `Dunn` unequal-`n` tied pairs, `Fisher LSD`, `Newman–Keuls`
//!   with step-down clamping, `Tamhane T2`, `multiple_row_t_tests`, `paired_sign_test`
//!   vs `binom.test`, `baptista_pike_or_ci` via `uniroot`, fixed-centre Lloyd `k-means`,
//!   Hyndman–Fan Type-6 quantiles, exact `binom.test`, column ROUT RSDR/$p$,
//!   `norm.qq` Blom convention, published Royston 1995 AS R94 `n = 25` lognormal
//!   vector, Altman 1998 Tramèr NNT, and NIST `GEAR.DAT` Bartlett/Levene/ANOVA).
//!
//! D1 Provenance:
//! - `R 4.5.2 (2025-10-31)` via `crates/stats/tests/reference/stats_extended_oracles.R` (2026-11-23; extended 2026-11-27).
//! - `R 4.5.2 (2025-10-31)` `stats::t.test` cross-checked by independent Python 3 Simpson quadrature on the Student-*t* PDF (2026-11-26) for harmonic and quadratic mean CIs.
//! - `SciPy 1.18.1` + `statsmodels 0.15.0` via `crates/stats/tests/reference/scipy_statsmodels_oracles.py` (2026-11-23).
//! - NIST/SEMATECH e-Handbook of Statistical Methods `GEAR.DAT` (§§1.3.5.4, 1.3.5.7, 1.3.5.10; Bartlett 1937; Levene 1960; Brown & Forsythe 1974a,b).
//! - Royston, P. (1995), Remark AS R94, *Applied Statistics* 44(4):547–551 (doi:10.2307/2986146).
//! - Altman, D. G. (1998), Confidence intervals for the number needed to treat, *BMJ* 317(7168):1309–1312 (doi:10.1136/bmj.317.7168.1309).
//! - Baptista, J., & Pike, M. C. (1977), Algorithm AS 115, *Applied Statistics* 26(2):214–220 (doi:10.2307/2347041).
//! - Hyndman, R. J., & Fan, Y. (1996), *American Statistician* 50(4):361–365 (doi:10.1080/00031305.1996.10473566).
//! - Blom, G. (1958), *Statistical Estimates and Transformed Beta-Variables*, Almqvist & Wiksell.
//! - Motulsky, H. J., & Brown, R. E. (2006), *BMC Bioinformatics* 7:123 (doi:10.1186/1471-2105-7-123).
//! - Lloyd, S. P. (1982), *IEEE Trans Inform Theory* 28(2):129–137 (doi:10.1109/tit.1982.1056489).
//! - Dixon, W. J., & Mood, A. M. (1946), *JASA* 41(236):557–566; Conover, W. J. (1999), *Practical Nonparametric Statistics* (3rd ed.), Wiley.
//! - Tamhane, A. C. (1979), *JASA* 74(366):471–480; Newman (1939) & Keuls (1952); Dunn, O. J. (1964), *Technometrics* 6(3):241–252.
//! - Bland & Altman (1996), *BMJ* 312(7038):1079 (doi:10.1136/bmj.312.7038.1079);
//!   Lam, Hung & Perrier (1985), *J Pharm Sci* 74(2):229–231 (doi:10.1002/jps.2600740229);
//!   Casella & Berger (2002), *Statistical Inference* (2nd ed.), Duxbury, §9.2.1;
//!   Bullen (2003), *Handbook of Means and Their Inequalities* (2nd ed.), Springer, Ch. III (doi:10.1007/978-94-017-0399-4).
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

use curvebook_doc::{KappaWeighting, MultiplicityMethod, MultiplicitySpec};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::anova::{kruskal_wallis_test, ordinary_one_way_anova, unequal_variance_one_way_anova, variance_homogeneity_tests};
use curvebook_stats::categorical::{NntKind, altman_nnt_from_rd, baptista_pike_or_ci, cohens_kappa, exact_binomial_test};
use curvebook_stats::column::{
    anderson_darling, dagostino_pearson_k2, grubbs_outliers, lilliefors_ks, qq_expected_quantiles, rout_outliers, shapiro_wilk, summarize_column,
};
use curvebook_stats::common::quantile_type6_sorted;
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_two_stage_bky, compare_dunn_ranks, compare_groups_with_procedure,
};
use curvebook_stats::multivariate::{KMeansAlgorithm, kmeans_with_centers};
use curvebook_stats::two_group::{
    bland_altman, ks_two_sample, levene_variance_test, multiple_row_t_tests, paired_sign_test, paired_t_test, unpaired_t_test,
};
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

// ============================================================================
// 7. Harmonic and Quadratic (RMS) Mean Confidence Intervals (GAP-06-04 / R19.1)
// ============================================================================

#[test]
fn oracle_harmonic_and_quadratic_mean_cis() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::t.test(1/x)` & `stats::t.test(x^2)` back-transformed
    // endpoints, independently cross-checked to >= 14 digits by Python 3 Simpson quadrature on the
    // Student-t PDF (2026-11-26). Citations: Bland & Altman (1996), Lam, Hung & Perrier (1985),
    // Casella & Berger (2002) §9.2.1, Bullen (2003) Ch. III.
    let conf_90 = ConfidenceLevel::from_percent(90.0).unwrap();
    let conf_99 = ConfidenceLevel::from_percent(99.0).unwrap();

    // 1. Positive skewed pharmacokinetic half-lives across 95%, 90%, and 99% confidence levels.
    let pk = [2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4];

    let s95 = summarize_column("PK95", &pk, 0, conf_95(), None).unwrap();
    assert_approx_eq(s95.harmonic_mean.expect("pk95 hm"), 4.434_547_725_670_957, 1e-12, 1e-12, "pk95 hm");
    let hm95 = s95.harmonic_mean_ci.expect("pk95 hm_ci");
    assert_approx_eq(hm95.0, 3.158_989_777_088_647, 1e-11, 1e-11, "pk95 hm_ci lo");
    assert_approx_eq(hm95.1, 7.437_853_985_145_783, 1e-11, 1e-11, "pk95 hm_ci hi");
    assert_approx_eq(s95.quadratic_mean, 6.243_997_117_231_878, 1e-12, 1e-12, "pk95 rms");
    let rms95 = s95.quadratic_mean_ci.expect("pk95 rms_ci");
    assert_approx_eq(rms95.0, 1.983_567_450_639_152, 1e-11, 1e-11, "pk95 rms_ci lo");
    assert_approx_eq(rms95.1, 8.604_676_645_218_278, 1e-11, 1e-11, "pk95 rms_ci hi");
    // Monotone back-transformed RMS CI is asymmetric around `rms` (unlike symmetric linearized delta-method bounds).
    assert!(((s95.quadratic_mean - rms95.0) - (rms95.1 - s95.quadratic_mean)).abs() > 1.0);

    let s90 = summarize_column("PK90", &pk, 0, conf_90, None).unwrap();
    let hm90 = s90.harmonic_mean_ci.expect("pk90 hm_ci");
    assert_approx_eq(hm90.0, 3.350_568_491_928_193, 1e-11, 1e-11, "pk90 hm_ci lo");
    assert_approx_eq(hm90.1, 6.555_336_760_697_462, 1e-11, 1e-11, "pk90 hm_ci hi");
    let rms90 = s90.quadratic_mean_ci.expect("pk90 rms_ci");
    assert_approx_eq(rms90.0, 3.301_885_950_082_799, 1e-11, 1e-11, "pk90 rms_ci lo");
    assert_approx_eq(rms90.1, 8.189_783_218_904_259, 1e-11, 1e-11, "pk90 rms_ci hi");

    // At 99%, `t.test(pk^2)` lower endpoint is -12.8885 < 0, so RMS CI lower bound clamps to 0.0 while HM CI remains strictly positive.
    let s99 = summarize_column("PK99", &pk, 0, conf_99, None).unwrap();
    let hm99 = s99.harmonic_mean_ci.expect("pk99 hm_ci");
    assert_approx_eq(hm99.0, 2.775_796_199_768_79, 1e-11, 1e-11, "pk99 hm_ci lo");
    assert_approx_eq(hm99.1, 11.019_613_639_982_806, 1e-11, 1e-11, "pk99 hm_ci hi");
    let rms99 = s99.quadratic_mean_ci.expect("pk99 rms_ci");
    assert_approx_eq(rms99.0, 0.0, 1e-15, 0.0, "pk99 rms_ci clamped lo");
    assert_approx_eq(rms99.1, 9.532_234_806_434_092, 1e-11, 1e-11, "pk99 rms_ci hi");

    // 2. Signed / zero-crossing observations: harmonic mean & CI are undefined; quadratic mean & CI stay well-defined.
    let signed = [-4.5, -1.2, 0.0, 2.3, 3.8, 5.1, -6.0];
    let sg95 = summarize_column("Signed95", &signed, 0, conf_95(), None).unwrap();
    assert_eq!(sg95.harmonic_mean, None);
    assert_eq!(sg95.harmonic_mean_ci, None);
    assert_approx_eq(sg95.quadratic_mean, 3.843_919_131_006_047, 1e-12, 1e-12, "signed95 rms");
    let sg_rms95 = sg95.quadratic_mean_ci.expect("signed95 rms_ci");
    assert_approx_eq(sg_rms95.0, 1.514_500_707_269_609, 1e-11, 1e-11, "signed95 rms_ci lo");
    assert_approx_eq(sg_rms95.1, 5.220_892_278_060_18, 1e-11, 1e-11, "signed95 rms_ci hi");

    let sg99 = summarize_column("Signed99", &signed, 0, conf_99, None).unwrap();
    let sg_rms99 = sg99.quadratic_mean_ci.expect("signed99 rms_ci");
    assert_approx_eq(sg_rms99.0, 0.0, 1e-15, 0.0, "signed99 rms_ci clamped lo");
    assert_approx_eq(sg_rms99.1, 5.804_116_301_904_712, 1e-11, 1e-11, "signed99 rms_ci hi");

    // 3. Reciprocal-CI zero crossing (singularity): `harmonic_mean` exists, `harmonic_mean_ci` must be `None`,
    //    and `quadratic_mean_ci` clamps its lower endpoint to `0.0`.
    let pair = [1.0, 100.0];
    let pr95 = summarize_column("Pair95", &pair, 0, conf_95(), None).unwrap();
    assert_approx_eq(pr95.harmonic_mean.expect("pair hm"), 1.980_198_019_801_980_2, 1e-12, 1e-12, "pair hm");
    assert_eq!(pr95.harmonic_mean_ci, None, "reciprocal CI crossing <= 0 must return None");
    assert_approx_eq(pr95.quadratic_mean, 70.714_213_564_176_76, 1e-11, 1e-11, "pair rms");
    let pr_rms95 = pr95.quadratic_mean_ci.expect("pair rms_ci");
    assert_approx_eq(pr_rms95.0, 0.0, 1e-15, 0.0, "pair rms_ci clamped lo");
    assert_approx_eq(pr_rms95.1, 261.773_128_067_999_7, 1e-10, 1e-10, "pair rms_ci hi");

    let skew4 = [0.1, 0.2, 0.3, 15.0];
    let sk95 = summarize_column("Skew4", &skew4, 0, conf_95(), None).unwrap();
    assert_approx_eq(sk95.harmonic_mean.expect("skew4 hm"), 0.217_391_304_347_826_1, 1e-12, 1e-12, "skew4 hm");
    assert_eq!(sk95.harmonic_mean_ci, None);
    let sk_rms95 = sk95.quadratic_mean_ci.expect("skew4 rms_ci");
    assert_approx_eq(sk_rms95.0, 0.0, 1e-15, 0.0, "skew4 rms_ci clamped lo");
    assert_approx_eq(sk_rms95.1, 15.338_203_409_601_526, 1e-11, 1e-11, "skew4 rms_ci hi");

    // 4. Sign invariance for RMS and extreme-scale (1e-200 / 1e200) overflow/underflow-safe scaling.
    let base = [10.0, 12.0, 14.0, 16.0, 18.0];
    let exp_hm = 13.411_389_036_721_66;
    let exp_hm_ci = (10.370_175_470_115_683, 18.976_556_221_719_093);
    let exp_rms = 14.282_856_857_085_7;
    let exp_rms_ci = (9.678_142_021_597_955, 17.729_454_786_027_12);

    for scale in [1.0, 1e-200, 1e200] {
        let scaled: Vec<f64> = base.iter().map(|&v| v * scale).collect();
        let res = summarize_column("Scaled", &scaled, 0, conf_95(), None).unwrap();
        assert_approx_eq(res.harmonic_mean.expect("scaled hm"), exp_hm * scale, 0.0, 1e-11, "scaled hm");
        let hm_ci = res.harmonic_mean_ci.expect("scaled hm_ci");
        assert_approx_eq(hm_ci.0, exp_hm_ci.0 * scale, 0.0, 1e-11, "scaled hm_ci lo");
        assert_approx_eq(hm_ci.1, exp_hm_ci.1 * scale, 0.0, 1e-11, "scaled hm_ci hi");
        assert_approx_eq(res.quadratic_mean, exp_rms * scale, 0.0, 1e-11, "scaled rms");
        let rms_ci = res.quadratic_mean_ci.expect("scaled rms_ci");
        assert_approx_eq(rms_ci.0, exp_rms_ci.0 * scale, 0.0, 1e-11, "scaled rms_ci lo");
        assert_approx_eq(rms_ci.1, exp_rms_ci.1 * scale, 0.0, 1e-11, "scaled rms_ci hi");
    }

    let negated: Vec<f64> = base.iter().map(|&v| -v).collect();
    let neg_res = summarize_column("Negated", &negated, 0, conf_95(), None).unwrap();
    assert_eq!((neg_res.harmonic_mean, neg_res.harmonic_mean_ci), (None, None));
    assert_approx_eq(neg_res.quadratic_mean, exp_rms, 1e-12, 1e-12, "negated rms");
    let neg_rms_ci = neg_res.quadratic_mean_ci.expect("negated rms_ci");
    assert_approx_eq(neg_rms_ci.0, exp_rms_ci.0, 1e-11, 1e-11, "negated rms_ci lo");
    assert_approx_eq(neg_rms_ci.1, exp_rms_ci.1, 1e-11, 1e-11, "negated rms_ci hi");

    // 5. Degenerate cases: constant nonzero, alternating constant magnitude, all-zero, and n = 1.
    let constant = summarize_column("Const", &[5.0, 5.0, 5.0, 5.0], 0, conf_95(), None).unwrap();
    assert_eq!(constant.harmonic_mean, Some(5.0));
    assert_eq!(constant.harmonic_mean_ci, Some((5.0, 5.0)));
    assert_eq!(constant.quadratic_mean, 5.0);
    assert_eq!(constant.quadratic_mean_ci, Some((5.0, 5.0)));

    let alt = summarize_column("Alt", &[-5.0, 5.0, -5.0, 5.0], 0, conf_95(), None).unwrap();
    assert_eq!((alt.harmonic_mean, alt.harmonic_mean_ci), (None, None));
    assert_eq!((alt.quadratic_mean, alt.quadratic_mean_ci), (5.0, Some((5.0, 5.0))));

    let zeros = summarize_column("Zero", &[0.0, 0.0, 0.0], 0, conf_95(), None).unwrap();
    assert_eq!((zeros.harmonic_mean, zeros.harmonic_mean_ci, zeros.quadratic_mean, zeros.quadratic_mean_ci), (None, None, 0.0, None));

    let single = summarize_column("One", &[42.0], 0, conf_95(), None).unwrap();
    assert_eq!((single.harmonic_mean, single.harmonic_mean_ci, single.quadratic_mean, single.quadratic_mean_ci), (Some(42.0), None, 42.0, None));
}

// ============================================================================
// 8. Brown-Forsythe F* with Satterthwaite df (heteroscedastic & balanced)
// ============================================================================

#[test]
fn oracle_brown_forsythe_f_star_and_satterthwaite_df() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::oneway.test(..., var.equal=FALSE)` &
    // Brown & Forsythe (1974b) `F*` + Satterthwaite df + `stats::pf(..., lower.tail=FALSE)`, 2026-11-27.
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];
    let het_inputs = vec![group_input(0, "G1", &g1), group_input(1, "G2", &g2), group_input(2, "G3", &g3), group_input(3, "G4", &g4)];
    let uv = unequal_variance_one_way_anova(&het_inputs).unwrap();
    assert_approx_eq(uv.welch_w, 1_383.102_180_147_434, 1e-8, 1e-9, "het Welch W");
    assert_approx_eq(uv.welch_df1, 3.0, 1e-12, 0.0, "het Welch df1");
    assert_approx_eq(uv.welch_df2, 10.124_470_300_778, 1e-9, 1e-9, "het Welch df2");
    assert_approx_eq(uv.welch_p, 1.597_405_155_454e-13, 1e-16, 1e-6, "het Welch p");
    assert_approx_eq(uv.brown_forsythe_f_star, 89.887_067_108_391, 1e-9, 1e-9, "het BF F*");
    assert_approx_eq(uv.brown_forsythe_df1, 3.0, 1e-12, 0.0, "het BF df1");
    assert_approx_eq(uv.brown_forsythe_df2, 6.597_953_665_105, 1e-9, 1e-9, "het BF Satterthwaite df2");
    assert_approx_eq(uv.brown_forsythe_p, 1.004_349_798_359e-5, 1e-11, 1e-7, "het BF p");

    // Balanced design (equal n = 5): Brown-Forsythe F* identically equals ordinary ANOVA F,
    // while Satterthwaite df2 < N - k reflects heteroscedasticity.
    let b1 = [10.0, 10.2, 9.8, 10.1, 9.9];
    let b2 = [12.0, 14.5, 11.2, 13.8, 12.5];
    let b3 = [15.0, 19.0, 14.2, 17.8, 16.0];
    let b4 = [20.0, 20.5, 19.7, 20.3, 19.5];
    let ow_bal = ordinary_one_way_anova(&[("G1", &b1[..]), ("G2", &b2[..]), ("G3", &b3[..]), ("G4", &b4[..])], conf_95()).unwrap();
    let bal_inputs = vec![group_input(0, "G1", &b1), group_input(1, "G2", &b2), group_input(2, "G3", &b3), group_input(3, "G4", &b4)];
    let uv_bal = unequal_variance_one_way_anova(&bal_inputs).unwrap();
    assert_approx_eq(ow_bal.f_statistic, 63.891_708_967_851, 1e-9, 1e-9, "bal ordinary ANOVA F");
    assert_approx_eq(uv_bal.brown_forsythe_f_star, ow_bal.f_statistic, 1e-11, 1e-11, "balanced BF F* == ordinary F");
    assert_approx_eq(uv_bal.brown_forsythe_df2, 7.504_177_420_178, 1e-9, 1e-9, "bal BF Satterthwaite df2");
    assert_approx_eq(uv_bal.brown_forsythe_p, 1.071_966_073_042e-5, 1e-11, 1e-7, "bal BF p");
}

// ============================================================================
// 9. Column mean_ci vs stats::t.test (+ sd_ci & geometric_mean_ci)
// ============================================================================

#[test]
fn oracle_mean_sd_and_geometric_mean_cis_vs_r_t_test() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::t.test(x, conf.level=...)`,
    // `exp(t.test(log(x), conf.level=...)$conf.int)`, and `qchisq` SD CI, 2026-11-27.
    let pk = [2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4];
    for (pct, exp_m_lo, exp_m_hi, exp_g_lo, exp_g_hi, exp_sd_lo, exp_sd_hi) in [
        (90.0, 3.561_433_184_763, 7.588_566_815_237, 3.503_450_870_992, 6.995_293_488_079, 2.120_530_474_945, 5.402_352_448_374),
        (95.0, 3.061_866_519_700, 8.088_133_480_300, 3.215_455_714_623, 7.621_833_182_835, 1.987_532_664_251, 6.118_160_802_589),
        (99.0, 1.855_733_130_265, 9.294_266_869_735, 2.613_961_597_476, 9.375_679_844_461, 1.766_190_954_657, 7.996_375_006_932),
    ] {
        let conf = ConfidenceLevel::from_percent(pct).unwrap();
        let s = summarize_column("PK", &pk, 0, conf, None).unwrap();
        assert_approx_eq(s.mean, 5.575, 1e-12, 0.0, "pk mean");
        let mci = s.mean_ci.expect("mean_ci");
        assert_approx_eq(mci.0, exp_m_lo, 1e-10, 1e-10, &format!("mean_ci({pct}%) lo"));
        assert_approx_eq(mci.1, exp_m_hi, 1e-10, 1e-10, &format!("mean_ci({pct}%) hi"));
        let gci = s.geometric_mean_ci.expect("geometric_mean_ci");
        assert_approx_eq(gci.0, exp_g_lo, 1e-10, 1e-10, &format!("gmean_ci({pct}%) lo"));
        assert_approx_eq(gci.1, exp_g_hi, 1e-10, 1e-10, &format!("gmean_ci({pct}%) hi"));
        let sdci = s.sd_ci.expect("sd_ci");
        assert_approx_eq(sdci.0, exp_sd_lo, 1e-10, 1e-10, &format!("sd_ci({pct}%) lo"));
        assert_approx_eq(sdci.1, exp_sd_hi, 1e-10, 1e-10, &format!("sd_ci({pct}%) hi"));
    }

    let signed = [-4.5, -1.2, 0.0, 2.3, 3.8, 5.1, -6.0];
    let sg = summarize_column("Signed", &signed, 0, conf_95(), None).unwrap();
    let sg_mci = sg.mean_ci.expect("signed mean_ci");
    assert_approx_eq(sg.mean, -0.071_428_571_429, 1e-11, 1e-11, "signed mean");
    assert_approx_eq(sg_mci.0, -3.910_639_275_919, 1e-10, 1e-10, "signed mean_ci lo");
    assert_approx_eq(sg_mci.1, 3.767_782_133_061, 1e-10, 1e-10, "signed mean_ci hi");
}

// ============================================================================
// 10. Dunn unequal-n tied per-pair R 4.5.2 provenance
// ============================================================================

#[test]
fn oracle_dunn_unequal_n_ties_r_provenance() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `rank(..., ties.method="average")` +
    // `2*pnorm(-abs(z))` + `p.adjust(..., "bonferroni" / "holm")` + Šídák, 2026-11-27.
    let g0 = [4.0, 5.0, 5.0, 6.0, 7.0];
    let g1 = [7.0, 8.0, 8.0, 8.0, 9.0, 10.0, 11.0];
    let g2 = [5.0, 6.0, 9.0, 12.0];
    let g3 = [11.0, 12.0, 13.0, 14.0, 14.0, 16.0];
    let kw = kruskal_wallis_test(&[&g0, &g1, &g2, &g3]).unwrap();
    let labels = [(0, "G0".to_owned()), (1, "G1".to_owned()), (2, "G2".to_owned()), (3, "G3".to_owned())];
    let ns = [5_usize, 7, 4, 6];
    let tie_sum = 84.0; // verified in R: sum(tab^3 - tab) == 84

    let mk_spec =
        |method| MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: false, control_column: None, selected_pairs: None };
    let d_bonf = compare_dunn_ranks(&labels, &ns, &kw.group_mean_ranks, kw.total_n, tie_sum, &mk_spec(MultiplicityMethod::Bonferroni)).unwrap();
    let d_sid = compare_dunn_ranks(&labels, &ns, &kw.group_mean_ranks, kw.total_n, tie_sum, &mk_spec(MultiplicityMethod::Sidak)).unwrap();
    assert_eq!(d_bonf.len(), 6);

    let expected: &[(f64, f64, f64, f64, f64, f64)] = &[
        // (diff, se, z, raw_p, bonf_p, sidak_p)
        (-7.357_142_857_143, 3.787_196_475_344, -1.942_635_642_233, 5.206_019_726_234e-2, 3.123_611_835_740e-1, 2.744_212_507_784e-1),
        (-5.625_000_000_000, 4.338_778_629_983, -1.296_447_797_804, 1.948_212_537_709e-1, 1.000_000_000_000e0, 7.275_079_629_774e-1),
        (-15.166_666_666_667, 3.916_489_357_689, -3.872_515_735_781, 1.077_177_189_973e-4, 6.463_063_139_839e-4, 6.461_322_923_743e-4),
        (1.732_142_857_143, 4.053_951_628_908, 0.427_272_699_751, 6.691_807_148_758e-1, 1.000_000_000_000e0, 9.986_891_744_043e-1),
        (-7.809_523_809_524, 3.598_390_292_674, -2.170_282_591_475, 2.998_544_459_024e-2, 1.799_126_675_415e-1, 1.669_529_969_039e-1),
        (-9.541_666_666_667, 4.174_991_683_292, -2.285_433_694_360, 2.228_741_018_913e-2, 1.337_244_611_348e-1, 1.264_912_790_277e-1),
    ];
    for (idx, &(exp_d, exp_se, exp_z, exp_rp, exp_b, exp_s)) in expected.iter().enumerate() {
        assert_approx_eq(d_bonf[idx].mean_diff, exp_d, 1e-10, 1e-10, &format!("Dunn[{idx}] diff"));
        assert_approx_eq(d_bonf[idx].se_diff, exp_se, 1e-10, 1e-10, &format!("Dunn[{idx}] se"));
        assert_approx_eq(d_bonf[idx].statistic, exp_z, 1e-10, 1e-10, &format!("Dunn[{idx}] z"));
        assert_approx_eq(d_bonf[idx].raw_p, exp_rp, 1e-11, 1e-9, &format!("Dunn[{idx}] raw_p"));
        assert_approx_eq(d_bonf[idx].adjusted_p_or_q, exp_b, 1e-11, 1e-9, &format!("Dunn[{idx}] Bonf"));
        assert_approx_eq(d_sid[idx].adjusted_p_or_q, exp_s, 1e-11, 1e-9, &format!("Dunn[{idx}] Sidak"));
    }
}

// ============================================================================
// 11. Fisher LSD, Newman-Keuls (+ step-down clamping), Tamhane T2, Multiple Row t-tests
// ============================================================================

#[test]
fn oracle_fisher_lsd_newman_keuls_tamhane_t2_and_multiple_row_t_tests() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `aov`/`pt`/`qt`, 2D studentized-range quadrature +
    // step-down coherence, `t.test(var.equal=F)` + Šídák, and `p.adjust`, 2026-11-27.

    // 11a. Fisher LSD on Ctrl/Low/High/Mid (MSE = 0.3075, df = 16)
    let ctrl = [10.2, 11.4, 9.8, 10.6, 11.0];
    let low = [14.1, 15.3, 13.8, 14.9, 14.4];
    let high = [18.2, 19.0, 17.6, 18.8, 18.4];
    let mid = [12.0, 12.6, 11.5, 12.3, 12.1];
    let ow = ordinary_one_way_anova(&[("Ctrl", &ctrl[..]), ("Low", &low[..]), ("High", &high[..]), ("Mid", &mid[..])], conf_95()).unwrap();
    let clhm = vec![group_input(0, "Ctrl", &ctrl), group_input(1, "Low", &low), group_input(2, "High", &high), group_input(3, "Mid", &mid)];
    let lsd = compare_groups_with_procedure(&clhm, ow.ms_within, ow.df_within, MultiplicityProcedure::FisherLsd, 0.05, true).unwrap();
    let lsd_exp: &[(f64, f64, f64, f64, f64)] = &[
        (-3.9, -11.120_185_995_988, 6.149_877_639_348e-9, -4.643_479_530_823, -3.156_520_469_177),
        (-7.8, -22.240_371_991_976, 1.850_611_015_499e-13, -8.543_479_530_823, -7.056_520_469_177),
        (-1.5, -4.276_994_613_842, 5.777_158_022_807e-4, -2.243_479_530_823, -0.756_520_469_177),
        (-3.9, -11.120_185_995_988, 6.149_877_639_348e-9, -4.643_479_530_823, -3.156_520_469_177),
        (2.4, 6.843_191_382_146, 3.950_590_344_981e-6, 1.656_520_469_177, 3.143_479_530_823),
        (6.3, 17.963_377_378_134, 4.977_732_240_679e-12, 5.556_520_469_177, 7.043_479_530_823),
    ];
    for (i, &(d, t, p, lo, hi)) in lsd_exp.iter().enumerate() {
        assert_approx_eq(lsd[i].mean_diff, d, 1e-12, 1e-12, &format!("LSD[{i}] diff"));
        assert_approx_eq(lsd[i].statistic, t, 1e-9, 1e-9, &format!("LSD[{i}] t"));
        assert_approx_eq(lsd[i].raw_p, p, 1e-15, 1e-8, &format!("LSD[{i}] raw_p"));
        assert_approx_eq(lsd[i].adjusted_p_or_q, p, 1e-15, 1e-8, &format!("LSD[{i}] adj_p == raw_p"));
        let ci = lsd[i].ci.expect("LSD CI");
        assert_approx_eq(ci.0, lo, 1e-9, 1e-9, &format!("LSD[{i}] ci lo"));
        assert_approx_eq(ci.1, hi, 1e-9, 1e-9, &format!("LSD[{i}] ci hi"));
    }

    // 11b. Student-Newman-Keuls: balanced 4-group + unequal-n step-down clamping
    let s0 = [8.0, 9.5, 10.0, 10.5, 12.0];
    let s1 = [9.5, 10.5, 11.5, 12.0, 13.0];
    let s2 = [10.5, 12.0, 12.8, 13.2, 14.5];
    let s3 = [12.2, 13.4, 14.0, 14.8, 16.1];
    let ow_snk = ordinary_one_way_anova(&[("G0", &s0[..]), ("G1", &s1[..]), ("G2", &s2[..]), ("G3", &s3[..])], conf_95()).unwrap();
    let snk_in = vec![group_input(0, "G0", &s0), group_input(1, "G1", &s1), group_input(2, "G2", &s2), group_input(3, "G3", &s3)];
    let snk = compare_groups_with_procedure(&snk_in, ow_snk.ms_within, ow_snk.df_within, MultiplicityProcedure::NewmanKeuls, 0.05, true).unwrap();
    let snk_exp: &[(f64, f64, f64)] = &[
        (2.018_599_470_321, 1.727_002_355_832e-1, 1.727_002_355_832e-1),
        (4.037_198_940_642, 1.146_813_832_621e-2, 2.922_047_828_711e-2),
        (6.366_352_175_627, 3.622_715_022_767e-4, 1.849_370_139_613e-3),
        (2.018_599_470_321, 1.727_002_355_832e-1, 1.727_002_355_832e-1),
        (4.347_752_705_306, 7.259_820_468_061e-3, 1.880_859_690_271e-2),
        (2.329_153_234_985, 1.190_627_156_612e-1, 1.190_627_156_612e-1),
    ];
    for (i, &(q, rp, ap)) in snk_exp.iter().enumerate() {
        assert!(snk[i].ci.is_none(), "Newman-Keuls does not define simultaneous CIs");
        assert_approx_eq(snk[i].statistic, q, 1e-9, 1e-9, &format!("SNK[{i}] q"));
        assert_approx_eq(snk[i].raw_p, rp, 1e-10, 1e-7, &format!("SNK[{i}] raw_p"));
        assert_approx_eq(snk[i].adjusted_p_or_q, ap, 1e-10, 1e-7, &format!("SNK[{i}] adj_p"));
    }

    // Unequal-n step-down clamping: inner pair (1,2) has raw_p = 2.67e-5 but enclosing span (0,3) has p = 0.075227!
    let snk_clamp_in = vec![
        GroupComparisonInput { index: 0, label: "G0".into(), n: 2.0, mean: 10.0, sd: 2.0 },
        GroupComparisonInput { index: 1, label: "G1".into(), n: 20.0, mean: 11.0, sd: 2.0 },
        GroupComparisonInput { index: 2, label: "G2".into(), n: 20.0, mean: 14.0, sd: 2.0 },
        GroupComparisonInput { index: 3, label: "G3".into(), n: 2.0, mean: 15.0, sd: 2.0 },
    ];
    let snk_c = compare_groups_with_procedure(&snk_clamp_in, 4.0, 40.0, MultiplicityProcedure::NewmanKeuls, 0.05, false).unwrap();
    // Pair index 3 is (1,2): raw_p = 2.6719e-5, clamped up to enclosing span (0,3) p = 0.07522735970622
    assert_approx_eq(snk_c[3].statistic, 6.708_203_932_499, 1e-9, 1e-9, "SNK_clamp (1,2) q");
    assert_approx_eq(snk_c[3].raw_p, 2.671_916_346_911e-5, 1e-10, 1e-6, "SNK_clamp (1,2) raw_p");
    assert_approx_eq(snk_c[3].adjusted_p_or_q, 7.522_735_970_622e-2, 1e-8, 1e-6, "SNK_clamp (1,2) clamped adj_p");
    assert!(!snk_c[3].significant, "clamped inner pair must not be declared significant");
    assert_approx_eq(snk_c[2].adjusted_p_or_q, 7.522_735_970_622e-2, 1e-8, 1e-6, "SNK_clamp (0,3) outer adj_p");

    // 11c. Tamhane T2 on heteroscedastic `het` dataset
    let g1 = [10.2, 10.8, 9.9, 11.1, 10.5, 10.3];
    let g2 = [12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1];
    let g3 = [18.5, 22.0, 16.4, 24.1, 19.8];
    let g4 = [25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9];
    let het_in = vec![group_input(0, "G1", &g1), group_input(1, "G2", &g2), group_input(2, "G3", &g3), group_input(3, "G4", &g4)];
    let t2 = compare_groups_with_procedure(&het_in, 0.0, 1.0, MultiplicityProcedure::TamhaneT2, 0.05, true).unwrap();
    let t2_exp: &[(f64, f64, f64, f64, f64, f64)] = &[
        (-5.244_734_605_970, 8.146_636_829_720, 7.344_572_343_353e-4, 4.398_659_913_940e-3, -5.502_491_142_508, -1.139_175_524_159),
        (-7.170_003_562_507, 4.138_739_889_497, 1.757_625_896_248e-3, 1.049_952_509_744e-2, -16.058_434_589_064, -3.328_232_077_602),
        (-67.742_802_188_293, 9.429_979_187_019, 5.359_979_207_033e-14, 3.217_426_325_364e-13, -15.453_416_642_646, -14.013_250_024_021),
        (-4.329_528_572_906, 5.678_651_507_584, 5.602_342_781_658e-3, 3.314_676_500_696e-2, -12.171_309_913_937, -0.573_690_086_063),
        (-18.369_316_551_836, 7.609_346_396_973, 1.407_134_166_476e-7, 8.442_802_031_938e-7, -13.598_789_652_734, -9.226_210_347_266),
        (-3.743_318_072_488, 4.072_198_222_085, 1.942_853_597_925e-2, 1.110_537_480_615e-1, -11.450_450_669_534, 1.370_450_669_534),
    ];
    for (i, &(t, df, rp, ap, lo, hi)) in t2_exp.iter().enumerate() {
        assert_approx_eq(t2[i].statistic, t, 1e-9, 1e-9, &format!("T2[{i}] t"));
        assert_approx_eq(t2[i].df, df, 1e-9, 1e-9, &format!("T2[{i}] df"));
        assert_approx_eq(t2[i].raw_p, rp, 1e-15, 1e-8, &format!("T2[{i}] raw_p"));
        assert_approx_eq(t2[i].adjusted_p_or_q, ap, 1e-15, 1e-3, &format!("T2[{i}] adj_p"));
        let ci = t2[i].ci.expect("T2 CI");
        assert_approx_eq(ci.0, lo, 1e-8, 1e-8, &format!("T2[{i}] ci lo"));
        assert_approx_eq(ci.1, hi, 1e-8, 1e-8, &format!("T2[{i}] ci hi"));
    }

    // 11d. Multiple row t-tests (6 rows, Student & Welch across Bonf/Holm/BH/BY)
    let mr_a = vec![
        vec![10.0, 10.5, 11.0, 10.2, 10.4],
        vec![5.0, 5.2, 4.9, 5.1, 5.3],
        vec![20.0, 20.4, 19.6, 20.1, 19.9],
        vec![12.0, 12.8, 11.5, 12.4, 12.1],
        vec![8.0, 8.6, 7.5, 8.2, 8.1],
        vec![15.0, 15.2, 14.9, 15.1, 14.8],
    ];
    let mr_b = vec![
        vec![10.1, 10.6, 10.9, 10.3, 10.5],
        vec![8.1, 8.5, 7.9, 8.2, 8.3],
        vec![19.7, 20.5, 19.9, 20.2, 20.1],
        vec![14.5, 15.2, 13.9, 14.8, 14.1],
        vec![9.2, 9.8, 8.9, 9.5, 9.0],
        vec![15.9, 16.4, 15.6, 16.0, 15.7],
    ];
    let lbls: Vec<String> = (0..6).map(|i| format!("Gene{i}")).collect();
    let stu_holm = multiple_row_t_tests(&lbls, &mr_a, &mr_b, false, MultiplicityProcedure::HolmBonferroni, 0.05).unwrap();
    let stu_bh = multiple_row_t_tests(&lbls, &mr_a, &mr_b, false, MultiplicityProcedure::BenjaminiHochbergFdr, 0.05).unwrap();
    let stu_by = multiple_row_t_tests(&lbls, &mr_a, &mr_b, false, MultiplicityProcedure::BenjaminiYekutieliFdr, 0.05).unwrap();
    let stu_exp: &[(f64, f64, f64, f64, f64)] = &[
        (-0.277_350_098_113, 7.885_433_926_808e-1, 1.0, 7.885_433_926_808e-1, 1.0),
        (-25.311_394_008_760, 6.357_486_835_863e-9, 3.814_492_101_518e-8, 3.814_492_101_518e-8, 9.345_505_648_719e-8),
        (-0.425_195_202_762, 6.819_005_668_180e-1, 1.0, 7.885_433_926_808e-1, 1.0),
        (-7.341_232_920_116, 8.061_616_168_694e-5, 4.030_808_084_347e-4, 2.418_484_850_608e-4, 5.925_287_883_990e-4),
        (-4.948_716_593_054, 1.122_693_000_882e-3, 3.368_079_002_645e-3, 1.684_039_501_322e-3, 4.125_896_778_240e-3),
        (-5.889_696_476_913, 3.659_733_145_130e-4, 1.463_893_258_052e-3, 7.319_466_290_260e-4, 1.793_269_241_114e-3),
    ];
    for (i, &(t, rp, holm, bh, by)) in stu_exp.iter().enumerate() {
        assert_approx_eq(stu_holm[i].statistic, t, 1e-9, 1e-9, &format!("Student row[{i}] t"));
        assert_approx_eq(stu_holm[i].raw_p, rp, 1e-13, 1e-8, &format!("Student row[{i}] raw_p"));
        assert_approx_eq(stu_holm[i].adjusted_p_or_q, holm, 1e-13, 1e-8, &format!("Student row[{i}] holm"));
        assert_approx_eq(stu_bh[i].adjusted_p_or_q, bh, 1e-13, 1e-8, &format!("Student row[{i}] BH"));
        assert_approx_eq(stu_by[i].adjusted_p_or_q, by, 1e-13, 1e-8, &format!("Student row[{i}] BY"));
    }

    let wel_holm = multiple_row_t_tests(&lbls, &mr_a, &mr_b, true, MultiplicityProcedure::HolmBonferroni, 0.05).unwrap();
    let wel_bh = multiple_row_t_tests(&lbls, &mr_a, &mr_b, true, MultiplicityProcedure::BenjaminiHochbergFdr, 0.05).unwrap();
    let wel_exp: &[(f64, f64, f64, f64)] = &[
        (7.650_691_630_571, 7.888_573_290_422e-1, 1.0, 7.888_573_290_422e-1),
        (7.200_000_000_000, 2.661_875_681_650e-8, 1.597_125_408_990e-7, 1.597_125_408_990e-7),
        (7.987_507_170_629, 6.819_177_000_849e-1, 1.0, 7.888_573_290_422e-1),
        (7.945_687_146_882, 8.346_757_304_909e-5, 4.173_378_652_455e-4, 2.504_027_191_473e-4),
        (7.963_148_924_409, 1.138_045_189_467e-3, 4.424_451_527_879e-3, 1.707_067_784_201e-3),
        (5.933_426_350_409, 1.106_112_881_970e-3, 4.424_451_527_879e-3, 1.707_067_784_201e-3),
    ];
    for (i, &(df, rp, holm, bh)) in wel_exp.iter().enumerate() {
        assert_approx_eq(wel_holm[i].df, df, 1e-9, 1e-9, &format!("Welch row[{i}] df"));
        assert_approx_eq(wel_holm[i].raw_p, rp, 1e-13, 1e-8, &format!("Welch row[{i}] raw_p"));
        assert_approx_eq(wel_holm[i].adjusted_p_or_q, holm, 1e-13, 1e-8, &format!("Welch row[{i}] holm"));
        assert_approx_eq(wel_bh[i].adjusted_p_or_q, bh, 1e-13, 1e-8, &format!("Welch row[{i}] BH"));
    }
}

// ============================================================================
// 12. Paired sign test & exact_binomial_test vs stats::binom.test
// ============================================================================

#[test]
fn oracle_paired_sign_test_and_exact_binomial_test_vs_r_binom_test() {
    // D1 Provenance: R 4.5.2 (2025-10-31) `stats::binom.test`, `stats::pbinom`,
    // `stats::prop.test(..., correct=FALSE)`, 2026-11-27.
    let sign_a = [12.5, 14.1, 11.8, 15.0, 13.2, 16.4, 10.9, 14.8, 13.5, 12.0, 15.2, 14.0, 11.5];
    let sign_b = [10.2, 13.0, 12.1, 12.5, 13.2, 14.0, 9.5, 12.2, 11.9, 12.0, 13.0, 11.8, 10.1];
    let s2 = paired_sign_test(&sign_a, &sign_b, true, conf_95()).unwrap();
    assert_eq!((s2.n_total, s2.n_positive, s2.n_negative, s2.n_ties, s2.n_nonzero), (13, 10, 1, 2, 11));
    assert_approx_eq(s2.p_value, 0.011_718_75, 1e-13, 1e-13, "sign_test 2-sided binom.test p");
    assert_approx_eq(s2.median_difference, 1.6, 1e-12, 0.0, "sign_test median diff");
    let mci = s2.ci.expect("sign_test median CI");
    assert_approx_eq(mci.0, 0.0, 1e-12, 0.0, "sign_test median CI lo");
    assert_approx_eq(mci.1, 2.4, 1e-12, 0.0, "sign_test median CI hi");
    assert_approx_eq(s2.ci_actual_confidence_percent.unwrap(), 97.753_906_25, 1e-10, 1e-10, "sign_test actual conf%");

    let s1 = paired_sign_test(&sign_a, &sign_b, false, conf_95()).unwrap();
    assert_approx_eq(s1.p_value, 0.005_859_375, 1e-13, 1e-13, "sign_test 1-sided binom.test p");

    // Exact binomial test across symmetric, asymmetric, and boundary (k = 0, k = n) cases.
    struct BinomCase {
        k: u64,
        n: u64,
        p0: f64,
        p_lo: f64,
        p_hi: f64,
        p_sterne: f64,
        p_2tail: f64,
        cp: (f64, f64),
        wilson: (f64, f64),
        ac: (f64, f64),
    }
    let cases = [
        BinomCase {
            k: 18,
            n: 25,
            p0: 0.50,
            p_lo: 0.992_683_351_039_886,
            p_hi: 0.021_642_625_331_879,
            p_sterne: 0.043_285_250_663_757,
            p_2tail: 0.043_285_250_663_757,
            cp: (0.506_123_178_194, 0.879_283_311_496),
            wilson: (0.524_233_948_096, 0.857_161_461_490),
            ac: (0.522_013_251_842, 0.859_382_157_745),
        },
        BinomCase {
            k: 7,
            n: 20,
            p0: 0.20,
            p_lo: 0.967_857_336_919_125,
            p_hi: 0.086_692_513_567_400,
            p_sterne: 0.098_221_728_613_469,
            p_2tail: 0.173_385_027_134_801,
            cp: (0.153_909_204_785, 0.592_188_534_533),
            wilson: (0.181_191_824_101, 0.567_145_723_315),
            ac: (0.179_926_361_438, 0.568_411_185_978),
        },
        BinomCase {
            k: 2,
            n: 15,
            p0: 0.35,
            p_lo: 0.061_734_095_365_285,
            p_hi: 0.985_821_215_407_943,
            p_sterne: 0.103_927_933_303_477,
            p_2tail: 0.123_468_190_730_570,
            cp: (0.016_575_913_440, 0.404_602_696_603),
            wilson: (0.037_361_259_103, 0.378_819_828_323),
            ac: (0.024_793_793_461, 0.391_387_293_965),
        },
        BinomCase {
            k: 0,
            n: 12,
            p0: 0.25,
            p_lo: 0.031_676_352_024_078,
            p_hi: 1.0,
            p_sterne: 0.045_929_133_892_059,
            p_2tail: 0.063_352_704_048_157,
            cp: (0.0, 0.264_648_469_397),
            wilson: (0.0, 0.242_494_006_655),
            ac: (0.0, 0.281_985_321_888),
        },
        BinomCase {
            k: 12,
            n: 12,
            p0: 0.80,
            p_lo: 1.0,
            p_hi: 0.068_719_476_736_000,
            p_sterne: 0.141_274_976_256_000,
            p_2tail: 0.137_438_953_472_000,
            cp: (0.735_351_530_603, 1.0),
            wilson: (0.757_505_993_345, 1.0),
            ac: (0.718_014_678_112, 1.0),
        },
    ];
    for c in &cases {
        let r = exact_binomial_test(c.k, c.n, c.p0, conf_95()).unwrap();
        let tag = format!("binom({},{},{})", c.k, c.n, c.p0);
        assert_approx_eq(r.p_value_lower, c.p_lo, 1e-11, 1e-11, &format!("{tag} p_lo"));
        assert_approx_eq(r.p_value_upper, c.p_hi, 1e-11, 1e-11, &format!("{tag} p_hi"));
        assert_approx_eq(r.p_value_sterne, c.p_sterne, 1e-11, 1e-11, &format!("{tag} p_sterne"));
        assert_approx_eq(r.p_value_doubled_tail, c.p_2tail, 1e-11, 1e-11, &format!("{tag} p_2tail"));
        assert_approx_eq(r.clopper_pearson_ci.0, c.cp.0, 1e-10, 1e-10, &format!("{tag} CP lo"));
        assert_approx_eq(r.clopper_pearson_ci.1, c.cp.1, 1e-10, 1e-10, &format!("{tag} CP hi"));
        assert_approx_eq(r.wilson_ci.0, c.wilson.0, 1e-10, 1e-10, &format!("{tag} Wilson lo"));
        assert_approx_eq(r.wilson_ci.1, c.wilson.1, 1e-10, 1e-10, &format!("{tag} Wilson hi"));
        assert_approx_eq(r.agresti_coull_ci.0, c.ac.0, 1e-10, 1e-10, &format!("{tag} AC lo"));
        assert_approx_eq(r.agresti_coull_ci.1, c.ac.1, 1e-10, 1e-10, &format!("{tag} AC hi"));
    }
}

// ============================================================================
// 13. Baptista-Pike (1977) root-finding oracle, Lloyd k-means, Type-6 quantiles
// ============================================================================

#[test]
fn oracle_baptista_pike_lloyd_kmeans_and_type6_quantiles() {
    // 13a. Baptista-Pike exact conditional OR CI via R 4.5.2 `lchoose` + `uniroot`, 2026-11-27.
    let conf_99 = ConfidenceLevel::from_percent(99.0).unwrap();
    let bp1 = baptista_pike_or_ci(14, 6, 5, 15, conf_95()).unwrap();
    assert_approx_eq(bp1.0, 1.489_289_241_422, 1e-7, 1e-7, "BP (14,6,5,15) 95% lo");
    assert_approx_eq(bp1.1, 31.301_093_258_306, 1e-7, 1e-7, "BP (14,6,5,15) 95% hi");

    let bp2 = baptista_pike_or_ci(12, 8, 4, 16, conf_99).unwrap();
    assert_approx_eq(bp2.0, 0.911_507_734_779, 1e-7, 1e-7, "BP (12,8,4,16) 99% lo");
    assert_approx_eq(bp2.1, 44.890_241_941_400, 1e-7, 1e-7, "BP (12,8,4,16) 99% hi");

    let bp3 = baptista_pike_or_ci(8, 2, 3, 12, conf_95()).unwrap();
    assert_approx_eq(bp3.0, 2.009_263_806_747, 1e-7, 1e-7, "BP (8,2,3,12) 95% lo");
    assert_approx_eq(bp3.1, 134.548_892_629_473, 1e-6, 1e-7, "BP (8,2,3,12) 95% hi");

    let bp_zero_a = baptista_pike_or_ci(0, 10, 5, 10, conf_95()).unwrap();
    assert_eq!(bp_zero_a.0, 0.0);
    assert_approx_eq(bp_zero_a.1, 1.141_344_086_592, 1e-7, 1e-7, "BP (0,10,5,10) 95% hi");

    let bp_zero_b = baptista_pike_or_ci(10, 0, 5, 10, conf_95()).unwrap();
    assert_approx_eq(bp_zero_b.0, 3.021_900_135_585, 1e-7, 1e-7, "BP (10,0,5,10) 95% lo");
    assert!(bp_zero_b.1.is_infinite() && bp_zero_b.1.is_sign_positive());

    // 13b. Fixed-centre Lloyd k-means vs R 4.5.2 `stats::kmeans(x, centers=init, algorithm="Lloyd")`.
    let km_x = vec![
        vec![0.2, 0.5],
        vec![0.8, 0.1],
        vec![1.1, 0.9],
        vec![0.4, 1.3],
        vec![1.5, 0.4],
        vec![5.2, 4.8],
        vec![5.9, 5.5],
        vec![4.6, 5.3],
        vec![6.1, 4.9],
        vec![5.4, 6.0],
        vec![1.0, 6.2],
        vec![1.6, 5.8],
        vec![0.7, 6.9],
        vec![2.0, 6.5],
        vec![1.3, 7.1],
    ];
    let km_init = vec![vec![0.0, 0.0], vec![3.0, 3.0], vec![2.5, 6.0]];
    let km = kmeans_with_centers(&km_x, &km_init, KMeansAlgorithm::Lloyd, 100).unwrap();
    assert_eq!(km.assignments, vec![0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2]);
    assert_approx_eq(km.wcss, 6.452, 1e-11, 1e-11, "Lloyd tot.withinss");
    let exp_centers = [(0.80, 0.64), (5.44, 5.30), (1.32, 6.50)];
    for (i, &(cx, cy)) in exp_centers.iter().enumerate() {
        assert_approx_eq(km.centroids[i][0], cx, 1e-12, 1e-12, &format!("Lloyd center[{i}].x"));
        assert_approx_eq(km.centroids[i][1], cy, 1e-12, 1e-12, &format!("Lloyd center[{i}].y"));
    }

    // 13c. Hyndman-Fan Type-6 quantiles vs R 4.5.2 `stats::quantile(x, probs, type = 6)`.
    let probs = [0.0, 0.05, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 1.0];
    let q_cases: &[(&str, &[f64], [f64; 9])] = &[
        ("odd7", &[2.1, 4.5, 5.0, 7.2, 9.8, 12.4, 18.0], [2.1, 2.1, 2.1, 4.5, 7.2, 12.4, 18.0, 18.0, 18.0]),
        ("even8", &[2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4], [2.4, 2.4, 2.4, 3.275, 4.6, 7.775, 11.4, 11.4, 11.4]),
        ("small4", &[10.0, 20.0, 35.0, 80.0], [10.0, 10.0, 10.0, 12.5, 27.5, 68.75, 80.0, 80.0, 80.0]),
        ("tied9", &[1.0, 2.0, 2.0, 2.0, 5.0, 7.0, 7.0, 9.0, 15.0], [1.0, 1.0, 1.0, 2.0, 5.0, 8.0, 15.0, 15.0, 15.0]),
        (
            "n15",
            &[1.2, 2.5, 3.1, 4.0, 5.5, 6.2, 7.8, 9.1, 10.4, 12.0, 14.5, 16.8, 19.2, 22.0, 27.5],
            [1.2, 1.2, 1.98, 4.0, 9.1, 16.8, 24.2, 27.5, 27.5],
        ),
    ];
    for &(label, sorted, expected) in q_cases {
        for (&p, &exp_q) in probs.iter().zip(expected.iter()) {
            let act = quantile_type6_sorted(sorted, p).unwrap();
            assert_approx_eq(act, exp_q, 1e-12, 1e-12, &format!("Type6 {label} p={p}"));
        }
        let col = summarize_column(label, sorted, 0, conf_95(), None).unwrap();
        assert_approx_eq(col.q1, expected[3], 1e-12, 1e-12, &format!("summarize {label} q1"));
        assert_approx_eq(col.median, expected[4], 1e-12, 1e-12, &format!("summarize {label} median"));
        assert_approx_eq(col.q3, expected[5], 1e-12, 1e-12, &format!("summarize {label} q3"));
    }
}

// ============================================================================
// 14. Column ROUT RSDR/p & norm.qq Blom (1958) convention check
// ============================================================================

#[test]
fn oracle_column_rout_rsdr_p_and_norm_qq_blom_convention() {
    // D1 Provenance: R 4.5.2 (2025-10-31) Cauchy IRLS + Type-7 P_68.27 RSDR + `2*pt(-t, df=n-1)`
    // and Blom (1958) `qnorm((1:n - 3/8)/(n + 1/4))`, 2026-11-27.
    let x_rout2 = [50.1, 49.4, 51.2, 50.8, 48.9, 50.5, 49.7, 51.0, 50.3, 49.6, 88.0, 12.5];
    let r2 = rout_outliers(&x_rout2, 1.0).unwrap();
    assert_eq!(r2.outliers.len(), 2);
    assert_approx_eq(r2.robust_center, 50.172_977_088_291, 1e-10, 1e-10, "ROUT2 center");
    assert_approx_eq(r2.rsdr, 1.013_388_866_301, 1e-10, 1e-10, "ROUT2 RSDR");
    // Sorted ascending by |residual|: 12.5 (|r|=37.673) then 88.0 (|r|=37.827)
    assert_eq!(r2.outliers[0].index, 11);
    assert_approx_eq(r2.outliers[0].score, 37.175_242_733_641, 1e-9, 1e-9, "ROUT2 outlier 12.5 t");
    assert_approx_eq(r2.outliers[0].threshold_or_p, 6.438_337_415_381e-13, 1e-16, 1e-8, "ROUT2 outlier 12.5 raw p");
    assert_eq!(r2.outliers[1].index, 10);
    assert_approx_eq(r2.outliers[1].score, 37.327_253_307_805, 1e-9, 1e-9, "ROUT2 outlier 88.0 t");
    assert_approx_eq(r2.outliers[1].threshold_or_p, 6.157_732_778_918e-13, 1e-16, 1e-8, "ROUT2 outlier 88.0 raw p");

    // Normal & lognormal Q-Q expected quantiles (n = 8 matches R `qqnorm` because R ppoints(n<=10) uses a = 3/8)
    let qq8 = [2.4, 3.1, 3.8, 4.2, 5.0, 6.5, 8.2, 11.4];
    let qn8 = qq_expected_quantiles(&qq8, false).unwrap();
    let exp_qn8 = [
        1.263_700_670_633,
        3.012_344_261_375,
        4.153_765_030_460,
        5.116_557_083_193,
        6.033_442_916_807,
        6.996_234_969_540,
        8.137_655_738_625,
        9.886_299_329_367,
    ];
    for (i, (&(exp_q, obs), &r_q)) in qn8.iter().zip(exp_qn8.iter()).enumerate() {
        assert_approx_eq(exp_q, r_q, 1e-9, 1e-9, &format!("QQ8 normal[{i}]"));
        assert_approx_eq(obs, qq8[i], 1e-14, 0.0, &format!("QQ8 obs[{i}]"));
    }
    let qln8 = qq_expected_quantiles(&qq8, true).unwrap();
    let exp_qln8 = [
        0.859_210_500_097,
        1.159_465_749_992,
        1.355_456_276_632,
        1.520_774_909_841,
        1.678_211_100_128,
        1.843_529_733_336,
        2.039_520_259_976,
        2.339_775_509_871,
    ];
    for (i, (&(exp_q, _), &r_q)) in qln8.iter().zip(exp_qln8.iter()).enumerate() {
        assert_approx_eq(exp_q, r_q, 1e-9, 1e-9, &format!("QQ8 lognormal[{i}]"));
    }

    // At n = 15 (> 10), CurveBook explicitly keeps Blom (1958) a = 3/8 (distinct from R ppoints a = 1/2: 9.8893 / 15.3907).
    let qq15 = [10.1, 10.8, 11.2, 11.5, 11.9, 12.0, 12.3, 12.5, 12.7, 13.0, 13.4, 13.8, 14.1, 14.7, 15.6];
    let qn15 = qq_expected_quantiles(&qq15, false).unwrap();
    assert_approx_eq(qn15[0].0, 10.031_089_425_514, 1e-9, 1e-9, "QQ15 Blom first");
    assert_approx_eq(qn15[7].0, 12.640_000_000_000, 1e-9, 1e-9, "QQ15 Blom center");
    assert_approx_eq(qn15[14].0, 15.248_910_574_486, 1e-9, 1e-9, "QQ15 Blom last");
    assert!((qn15[0].0 - 9.889_302_710_358).abs() > 0.1, "QQ15 must follow Blom a=3/8, not Hazen a=1/2");
}

// ============================================================================
// 15. Published Benchmarks: Royston (1995 AS R94) n=25 & Altman (1998) Tramèr NNT
// ============================================================================

#[test]
fn oracle_published_royston_1995_and_altman_1998_tramer_nnt() {
    // 15a. Royston, P. (1995) Remark AS R94 (Appl Stat 44(4):547-551) n = 25 lognormal example:
    // Published: W = 0.83467, p = 0.000914; R 4.5.2 `stats::shapiro.test` exact:
    //   W = 0.8346662753181682, p = 9.1349048181297036e-4.
    let royston_25 = [
        0.139, 0.157, 0.175, 0.256, 0.344, 0.413, 0.503, 0.577, 0.614, 0.655, 0.954, 1.392, 1.557, 1.648, 1.690, 1.994, 2.174, 2.206, 3.245, 3.510,
        3.571, 4.354, 4.980, 6.084, 8.351,
    ];
    let sw = shapiro_wilk(&royston_25, 0.05).unwrap();
    assert_approx_eq(sw.statistic, 0.834_666_275_318_168_2, 1e-11, 1e-11, "Royston 1995 n=25 W");
    assert_approx_eq(sw.p_value, 9.134_904_818_129_704e-4, 1e-10, 1e-8, "Royston 1995 n=25 p");
    assert!(!sw.passed);

    // 15b. Altman, D. G. (1998) BMJ 317:1309-1312 Tramèr et al. benchmark (ondansetron vs droperidol):
    // ARR = -0.08, 95% CI [-0.27, +0.11] -> NNTH = 12.5, 95% CI = NNTH 3.7 to Inf to NNTB 9.1.
    let (kind, nnt_val, pt_label, ci_label, ci_bounds) = altman_nnt_from_rd(-0.08, -0.27, 0.11);
    assert_eq!(kind, NntKind::Harm);
    assert_approx_eq(nnt_val.unwrap(), 12.5, 1e-12, 0.0, "Altman 1998 Tramèr NNTH");
    assert_eq!(pt_label, "NNTH 12.50");
    assert!(ci_bounds.is_none(), "CI spanning zero has disjoint NNT rays (no finite single interval)");
    assert_eq!(ci_label, "NNTH 3.70 to ∞ to NNTB 9.09");

    // Finite-interval benefit benchmark (ARR = +0.15, 95% CI [+0.05, +0.25] -> NNTB 6.67, CI [4.00, 20.00]).
    let (b_kind, b_val, b_pt, b_ci_lbl, b_bounds) = altman_nnt_from_rd(0.15, 0.05, 0.25);
    assert_eq!(b_kind, NntKind::Benefit);
    assert_approx_eq(b_val.unwrap(), 1.0 / 0.15, 1e-12, 1e-12, "Altman NNTB point");
    assert_eq!(b_pt, "NNTB 6.67");
    let (b_lo, b_hi) = b_bounds.expect("strictly positive RD CI yields finite NNTB bounds");
    assert_approx_eq(b_lo, 4.0, 1e-12, 0.0, "Altman NNTB CI lo");
    assert_approx_eq(b_hi, 20.0, 1e-12, 0.0, "Altman NNTB CI hi");
    assert_eq!(b_ci_lbl, "NNTB 4.00 to 20.00");
}

// ============================================================================
// 16. NIST Engineering Statistics Handbook `GEAR.DAT` Worked Examples
// ============================================================================

#[test]
fn oracle_nist_gear_dat_levene_bartlett_and_anova() {
    // D1 Provenance: Public-domain NIST/SEMATECH e-Handbook `assets/datasets/nist/GEAR.DAT`
    // (100 gear diameter measurements across 10 batches), verified against NIST §§1.3.5.4 / 1.3.5.7 / 1.3.5.10
    // (NIST float32 Dataplot prints Levene W = 1.705910, Bartlett T = 20.78580, ANOVA F = 2.2969)
    // and exact double-precision base R 4.5.2 (`bartlett.test`, `anova(lm(...))`, `aov`), 2026-11-27.
    const GEAR_DAT: &str = include_str!("../../../../assets/datasets/nist/GEAR.DAT");
    let mut batches: Vec<Vec<f64>> = vec![Vec::new(); 10];
    for line in GEAR_DAT.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let val: f64 = parts.next().expect("diameter").parse().expect("f64 diameter");
        let batch_idx: usize = parts.next().expect("batch").parse().expect("usize batch");
        assert!((1..=10).contains(&batch_idx), "unexpected batch id {batch_idx}");
        batches[batch_idx - 1].push(val);
    }
    assert!(batches.iter().all(|b| b.len() == 10), "GEAR.DAT must contain 10 batches x 10 observations");

    let slices: Vec<&[f64]> = batches.iter().map(Vec::as_slice).collect();
    let labeled: Vec<(&str, &[f64])> = ["Batch1", "Batch2", "Batch3", "Batch4", "Batch5", "Batch6", "Batch7", "Batch8", "Batch9", "Batch10"]
        .into_iter()
        .zip(slices.iter().copied())
        .collect();

    // 1. Bartlett's test & Brown-Forsythe (median Levene) via `variance_homogeneity_tests`
    let vh = variance_homogeneity_tests(&slices).unwrap();
    assert_approx_eq(vh.bartlett_chi2, 20.785_873_428_065, 1e-9, 1e-9, "GEAR.DAT Bartlett T (R 4.5.2)");
    assert_approx_eq(vh.bartlett_chi2, 20.785_80, 1e-4, 1e-5, "GEAR.DAT Bartlett T (NIST §1.3.5.7)");
    assert_approx_eq(vh.bartlett_df, 9.0, 1e-15, 0.0, "GEAR.DAT Bartlett df");
    assert_approx_eq(vh.bartlett_p, 1.363_586_327_806e-2, 1e-10, 1e-10, "GEAR.DAT Bartlett p");

    assert_approx_eq(vh.brown_forsythe_f, 1.705_917_693_001, 1e-9, 1e-9, "GEAR.DAT Levene median W (R 4.5.2)");
    assert_approx_eq(vh.brown_forsythe_f, 1.705_910, 1e-5, 1e-5, "GEAR.DAT Levene median W (NIST §1.3.5.10)");
    assert_approx_eq(vh.brown_forsythe_df1, 9.0, 1e-15, 0.0, "GEAR.DAT Levene df1");
    assert_approx_eq(vh.brown_forsythe_df2, 90.0, 1e-15, 0.0, "GEAR.DAT Levene df2");
    assert_approx_eq(vh.brown_forsythe_p, 9.908_297_555_220e-2, 1e-10, 1e-10, "GEAR.DAT Levene median p");

    // 2. Direct `levene_variance_test` (median-centered and mean-centered)
    let lev_med = levene_variance_test(&slices, true).unwrap();
    assert_approx_eq(lev_med.f_statistic, 1.705_917_693_001, 1e-9, 1e-9, "GEAR.DAT levene_variance_test(median) W");
    assert_approx_eq(lev_med.p_value, 9.908_297_555_220e-2, 1e-10, 1e-10, "GEAR.DAT levene_variance_test(median) p");

    let lev_mean = levene_variance_test(&slices, false).unwrap();
    assert_approx_eq(lev_mean.f_statistic, 2.159_459_856_473, 1e-9, 1e-9, "GEAR.DAT levene_variance_test(mean) W");
    assert_approx_eq(lev_mean.p_value, 3.223_682_655_978e-2, 1e-10, 1e-10, "GEAR.DAT levene_variance_test(mean) p");

    // 3. Ordinary one-way ANOVA (NIST §1.3.5.4: F = 2.2969)
    let aov = ordinary_one_way_anova(&labeled, conf_95()).unwrap();
    assert_approx_eq(aov.f_statistic, 2.296_912_413_359, 1e-9, 1e-9, "GEAR.DAT ANOVA F (R 4.5.2)");
    assert_approx_eq(aov.df_between, 9.0, 1e-15, 0.0, "GEAR.DAT ANOVA df1");
    assert_approx_eq(aov.df_within, 90.0, 1e-15, 0.0, "GEAR.DAT ANOVA df2");
    assert_approx_eq(aov.p_value, 2.266_081_927_864e-2, 1e-10, 1e-10, "GEAR.DAT ANOVA p");
}
