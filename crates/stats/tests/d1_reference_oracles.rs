#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Decision D1 Numerical Verification Oracle Suite for `curvebook-stats`.
//!
//! Reference provenance (zero competitor suite fixtures or proprietary text):
//! - **NIST StRD (`corpus/nist-strd` & `curvebook-testkit`):** Univariate (`univ`, 9 files +
//!   5 offline stiff benchmarks), One-Way ANOVA (`anova`, 11 files + offline stiff benchmarks),
//!   and Linear Least-Squares (`lls`, 11 files + offline stiff benchmarks).
//! - **Published analytical literature:** Royston (1995) AS R94 `n = 25` lognormal Shapiro-Wilk
//!   benchmark (`W = 0.83467, p = 0.000914`), Zar *Biostatistical Analysis* ANOVA/Dunn/Friedman,
//!   Dunnett (1955/1964), Benjamini, Krieger & Yekutieli (2006) two-stage adaptive FDR,
//!   Motulsky & Brown (2006) ROUT, Baptista & Pike (1977), Koopman (1984), Newcombe (1998),
//!   DeLong et al. (1988), Hanley & McNeil (1982), Brookmeyer & Crowley (1982).
//! - **Open-source black-box statistical reference (`D1`):** Generated on `2026-11-13` using
//!   `R version 4.5.2 (2025-10-31)` with base `stats 4.5.2`, `survival 3.8.3`, and `nlme 3.1.168`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ConfidenceLevel, ContingencyMethod, ContingencySpec, ContingencyTable,
    CoxTieMethod, DataSheet, DescriptiveSpec, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable, LinearRegressionSpec, MultipleVariablesTable,
    MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod, RepeatedMeasuresDesign,
    ResultCell, ResultTabKind, RocCiMethod, RocSpec, SubcolumnMode, SumOfSquaresType, SummarizedPoint, SurvivalAnalysisSpec, SurvivalObservation,
    SurvivalStatus, SurvivalTable, TableArchetype, TwoGroupSpec,
};
use curvebook_math::DenseMatrix;
use curvebook_stats::anova::{
    FactorialObservations, compute_anova_effect_sizes, fit_factorial_anova, fit_random_intercept_reml, fit_rm_two_way_anova, friedman_test,
    kruskal_wallis_test, nested_one_way_anova, ordinary_one_way_anova, ordinary_one_way_from_summary, rm_one_way_anova,
    unequal_variance_one_way_anova, variance_homogeneity_tests,
};
use curvebook_stats::categorical::{
    KappaWeighting, analyze_2x2_ratios, analyze_contingency, baptista_pike_or_ci, clopper_pearson_ci, cochran_armitage_trend, cohens_kappa,
    fisher_exact_2x2, fisher_exact_rxc, fleiss_kappa, koopman_rr_ci, mcnemar_test, wilson_score_ci,
};
use curvebook_stats::column::{
    anderson_darling, dagostino_pearson_k2, exact_binomial_median_ci, grubbs_outliers, lilliefors_ks, normality_battery, one_sample_t_test,
    one_sample_wilcoxon, rout_outliers, shapiro_wilk, summarize_column, summarize_pre_summarized_points,
};
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni,
    adjust_holm_sidak, adjust_sidak, adjust_two_stage_bky, compare_dunn_ranks, compare_groups_with_procedure, linear_trend_test,
};
use curvebook_stats::multivariate::{
    LinkageMethod, correlation_matrix, hierarchical_clustering, kendall_tau_b, kmeans_pp, pca_analysis, pearson_correlation, spearman_correlation,
};
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression};
use curvebook_stats::roc::{compare_roc_delong_paired, compare_roc_unpaired, roc_curve_analysis};
use curvebook_stats::survival::{
    CoxSubject, compare_survival_curves, cox_ph_regression, kaplan_meier_curve, log_rank_trend_test, two_cohort_hazard_ratio,
};
use curvebook_stats::two_group::{
    bland_altman, f_test_variances, ks_two_sample, levene_variance_test, lognormal_unpaired_t_test, mann_whitney_u, multiple_row_t_tests,
    paired_t_test, ratio_paired_t_test, unpaired_t_test, welch_t_test, wilcoxon_matched_pairs,
};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::nist_strd::{
    all_offline_anova_benchmarks, all_offline_linear_regression_benchmarks, all_offline_univariate_benchmarks, corpus_available,
    default_nist_strd_dir, load_corpus_anova_all, load_corpus_linear_all, load_corpus_univariate_all,
};
use curvebook_testkit::{assert_approx_eq, assert_lre_at_least, oracle_univariate_f64};

/// The cached NIST StRD corpus (`corpus/nist-strd`, gitignored, `cargo xtask corpus --fetch`), or
/// `None` when it is absent so offline runs and CI still exercise the generated benchmarks.
/// `CURVEBOOK_REQUIRE_CORPUS=1` turns a missing corpus into a failure.
fn require_corpus_dir() -> Option<std::path::PathBuf> {
    let dir = default_nist_strd_dir();
    if corpus_available(&dir) {
        return Some(dir);
    }
    assert!(
        std::env::var_os("CURVEBOOK_REQUIRE_CORPUS").is_none_or(|v| v != "1"),
        "CURVEBOOK_REQUIRE_CORPUS=1 set, but NIST StRD corpus is missing or incomplete at {}",
        dir.display()
    );
    None
}

fn conf_95() -> ConfidenceLevel {
    ConfidenceLevel::DEFAULT_95
}

fn scalar_val(sheet: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).map(|s| s.value).unwrap_or_else(|| panic!("missing scalar `{key}`"))
}

// ============================================================================
// 1. NIST StRD Univariate + Descriptive Moments + Median CI + Exclusions
// ============================================================================

#[test]
fn oracle_nist_strd_univariate_all_and_offline_stiff() {
    for ds in all_offline_univariate_benchmarks() {
        let summary = summarize_column(&ds.name, &ds.values, 0, conf_95(), None).unwrap();
        let (o_mean, o_sd, _) = oracle_univariate_f64(&ds.values);
        let min_sd_lre = match ds.name.as_str() {
            "NumAcc4" => 8.0,
            "NumAcc3" => 9.2,
            _ => 10.0,
        };
        assert_lre_at_least(summary.mean, ds.certified_mean, 12.0, &format!("offline {} mean", ds.name));
        assert_lre_at_least(summary.sd.unwrap(), ds.certified_sd, min_sd_lre, &format!("offline {} sd", ds.name));
        assert_lre_at_least(summary.mean, o_mean, 12.0, &format!("dd {} mean", ds.name));
        assert_lre_at_least(summary.sd.unwrap(), o_sd, 14.0, &format!("dd {} sd", ds.name));
    }

    let Some(dir) = require_corpus_dir() else {
        return;
    };
    let univ = load_corpus_univariate_all(&dir).unwrap();
    assert_eq!(univ.len(), 9);
    for ds in univ {
        let summary = summarize_column(&ds.name, &ds.values, 0, conf_95(), None).unwrap();
        let min_lre = match ds.name.as_str() {
            "NumAcc4" => 8.0,
            "NumAcc3" => 9.2,
            _ => 13.0,
        };
        assert_lre_at_least(summary.mean, ds.certified_mean, 13.0, &format!("corpus {} mean", ds.name));
        let sd = summary.sd.unwrap();
        assert_lre_at_least(sd, ds.certified_sd, min_lre, &format!("corpus {} sd", ds.name));
        let n_f = ds.values.len() as f64;
        assert_approx_eq(summary.sem.unwrap(), sd / n_f.sqrt(), 1e-14, 1e-12, "sem");
    }
}

#[test]
fn oracle_descriptive_quantiles_ratios_median_ci_and_exclusions() {
    // Values: [2, 4, 4, 8, 16, 32] -> n=6, sum=66, mean=11
    let vals = [2.0, 4.0, 4.0, 8.0, 16.0, 32.0];
    let s = summarize_column("Sample", &vals, 2, conf_95(), Some(90.0)).unwrap();
    assert_eq!(s.n, 6);
    assert_eq!(s.excluded_count, 2);
    assert_approx_eq(s.min, 2.0, 1e-15, 0.0, "min");
    assert_approx_eq(s.max, 32.0, 1e-15, 0.0, "max");
    assert_approx_eq(s.sum, 66.0, 1e-14, 0.0, "sum");
    assert_approx_eq(s.mean, 11.0, 1e-14, 0.0, "mean");
    assert_approx_eq(s.median, 6.0, 1e-14, 0.0, "median");
    // Type-6 quartiles on sorted [2, 4, 4, 8, 16, 32]:
    // Q1 position h = 0.25 * 7 = 1.75 -> 2 + 0.75*(4 - 2) = 3.5
    // Q3 position h = 0.75 * 7 = 5.25 -> 16 + 0.25*(32 - 16) = 20.0
    assert_approx_eq(s.q1, 3.5, 1e-14, 0.0, "q1 type6");
    assert_approx_eq(s.q3, 20.0, 1e-14, 0.0, "q3 type6");
    assert_approx_eq(s.iqr, 16.5, 1e-14, 0.0, "iqr");
    // Type-7 custom 90th percentile: h = 0.9 * 5 + 1 = 5.5 -> 0.5*(16 + 32) = 24.0
    assert_approx_eq(s.custom_upper_percentile.unwrap().1, 24.0, 1e-13, 0.0, "P90 type7");
    // Type-7 10th percentile: h = 0.1 * 5 + 1 = 1.5 -> 0.5*(2 + 4) = 3.0
    assert_approx_eq(s.custom_lower_percentile.unwrap().1, 3.0, 1e-13, 0.0, "P10 type7");
    // Geometric mean = (2*4*4*8*16*32)^(1/6) = 2^(1+2+2+3+4+5)/6 = 2^(17/6) = 7.127189745122714
    assert_approx_eq(s.geometric_mean.unwrap(), 2.0_f64.powf(17.0 / 6.0), 1e-12, 1e-12, "geom mean");
    // Harmonic mean = 6 / (1/2 + 1/4 + 1/4 + 1/8 + 1/16 + 1/32) = 6 / (39/32) = 192/39 = 64/13
    assert_approx_eq(s.harmonic_mean.unwrap(), 64.0 / 13.0, 1e-12, 1e-12, "harm mean");
    assert_approx_eq(s.quadratic_mean, ((4.0 + 16.0 + 16.0 + 64.0 + 256.0 + 1024.0) / 6.0_f64).sqrt(), 1e-12, 1e-12, "rms");

    // Exact binomial median CI on n = 10 distinct integers 1..=10:
    // P(1 <= X_bin <= 8) = 1 - 2*(1 + 10)/1024 = 1 - 22/1024 = 0.978515625 (97.8515625% >= 95%)
    let seq: Vec<f64> = (1..=10).map(|v| v as f64).collect();
    let mci = exact_binomial_median_ci(&seq, conf_95()).unwrap();
    assert_eq!((mci.lower_rank_1based, mci.upper_rank_1based), (2, 9));
    assert_approx_eq(mci.lower, 2.0, 1e-15, 0.0, "median ci lower");
    assert_approx_eq(mci.upper, 9.0, 1e-15, 0.0, "median ci upper");
    assert_approx_eq(mci.actual_confidence_percent, 97.851_562_5, 1e-9, 0.0, "actual conf%");

    // Pre-summarized pooled Mean/SD/N pooling (exact ANOVA between+within identity)
    let pts = [
        SummarizedPoint { mean: 10.0, sd: 2.0, n: 5.0, ss_within: 16.0, df_within: 4.0 },
        SummarizedPoint { mean: 14.0, sd: 3.0, n: 5.0, ss_within: 36.0, df_within: 4.0 },
    ];
    let pooled = summarize_pre_summarized_points("Pooled", &pts, conf_95()).unwrap();
    assert_approx_eq(pooled.total_n, 10.0, 1e-15, 0.0, "pooled N");
    assert_approx_eq(pooled.mean, 12.0, 1e-14, 0.0, "pooled mean");
    // SS_within = 4*4 + 4*9 = 52; SS_between = 5*4 + 5*4 = 40; total SS = 92; pooled var = 92/9
    assert_approx_eq(pooled.sd.unwrap(), (92.0_f64 / 9.0).sqrt(), 1e-12, 1e-12, "pooled sd");

    // Non-destructive `*` exclusion via run_analysis
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5).unwrap();
    table.data_sets[0].title = String::from("Group");
    *table.data_sets[0].cell_mut(0, 0).unwrap() = Cell::number(10.0);
    *table.data_sets[0].cell_mut(1, 0).unwrap() = Cell::number(12.0);
    *table.data_sets[0].cell_mut(2, 0).unwrap() = Cell::parse_import_token("*9999.0", '.', true);
    *table.data_sets[0].cell_mut(3, 0).unwrap() = Cell::number(14.0);
    *table.data_sets[0].cell_mut(4, 0).unwrap() = Cell::excluded_number(5000.0);
    let sheet = DataSheet::new("d1", "Excl", TableArchetype::Column(table)).unwrap();
    let mut spec = AnalysisSpec::new("r1", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    spec.selected_data_sets = vec![0];
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_approx_eq(scalar_val(&res, "Mean"), 12.0, 1e-12, 0.0, "excluded points omitted from mean");
    assert_approx_eq(scalar_val(&res, "N"), 3.0, 1e-12, 0.0, "excluded points omitted from N");
}

// ============================================================================
// 2. Normality Battery (Royston 1995 & R 4.5.2) + Outliers (Grubbs & ROUT)
// ============================================================================

#[test]
fn oracle_normality_shapiro_wilk_lognormality_and_outliers() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::shapiro.test on right-skewed n = 25 lognormal-like sample, 2026-11-13
    let skewed_25 = [0.1, 0.1, 0.1, 0.2, 0.2, 0.2, 0.3, 0.4, 0.5, 0.5, 0.6, 0.7, 0.8, 1.0, 1.1, 1.3, 1.5, 1.8, 2.1, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0];
    let sw_r = shapiro_wilk(&skewed_25, 0.05).unwrap();
    assert_approx_eq(sw_r.statistic, 0.811_948_922_926_847, 1e-10, 1e-10, "R shapiro skewed_25 W");
    assert_approx_eq(sw_r.p_value, 0.000_362_169_511_169, 1e-5, 5e-3, "R shapiro skewed_25 p");
    assert!(!sw_r.passed);

    // D1 Provenance: R 4.5.2 (2025-10-31), stats::shapiro.test(x_norm), date 2026-11-13
    let x_norm = [10.2, 9.8, 10.5, 11.1, 9.4, 10.0, 10.7, 9.9, 10.3, 10.1, 9.6, 10.8];
    let sw_n = shapiro_wilk(&x_norm, 0.05).unwrap();
    assert_approx_eq(sw_n.statistic, 0.985_296_635_279_895, 1e-4, 1e-4, "R shapiro W");
    assert_approx_eq(sw_n.p_value, 0.996_882_906_945_392, 5e-3, 0.0, "R shapiro p");
    assert!(sw_n.passed);

    // D1 Provenance: Shapiro & Wilk (1965) / Royston (1995 AS R94); R 4.5.2 stats::shapiro.test
    // small-sample n = 3..5 oracles (before R4.6 fix, n = 3 returned 1 - p via acos instead of asin - pi/3).
    for &(sample, exp_w, exp_p, label) in &[
        (&[1.0_f64, 2.0, 3.0][..], 1.0, 1.0, "n=3 symmetric [1,2,3]"),
        (&[1.0, 1.0, 2.0][..], 0.75, 0.0, "n=3 degenerate [1,1,2]"),
        (&[1.0, 1.5, 10.0][..], 0.791_530_944_625_407_1, 0.094_436_484_202_600_3, "n=3 skewed [1,1.5,10]"),
        (&[1.0, 2.0, 3.0, 10.0][..], 0.806_885_664_567_700_2, 0.115_152_981_754_536_6, "n=4 skewed [1,2,3,10]"),
        (&[1.0, 2.0, 3.0, 4.0, 20.0][..], 0.687_579_225_298_494, 0.007_068_598_540_515_0, "n=5 skewed [1,2,3,4,20]"),
    ] {
        let sw = shapiro_wilk(sample, 0.05).unwrap();
        assert_approx_eq(sw.statistic, exp_w, 1e-11, 1e-11, &format!("{label} W"));
        assert_approx_eq(sw.p_value, exp_p, 1e-9, 1e-9, &format!("{label} p"));
    }

    // All four normality tests + lognormal comparison on strictly positive skewed data
    let skewed = [1.1, 1.3, 1.5, 1.7, 2.0, 2.4, 2.9, 3.6, 4.5, 5.8, 8.2, 14.5];
    let bat = normality_battery(&skewed, 0.05).unwrap();
    assert!(bat.dagostino_pearson.is_some());
    assert!(bat.anderson_darling.is_some());
    assert!(bat.kolmogorov_smirnov.is_some());
    assert!(bat.normal_vs_lognormal_log_lr.unwrap() < 0.0, "lognormal model should have higher likelihood");
    assert!(bat.lognormal_shapiro_wilk.as_ref().unwrap().passed, "log-transformed skewed data passes normality");
    assert!(dagostino_pearson_k2(&x_norm, 0.05).unwrap().passed);
    assert!(anderson_darling(&x_norm, 0.05).unwrap().passed);
    assert!(lilliefors_ks(&x_norm, 0.05).unwrap().passed);

    // Grubbs single and iterative ESD outlier detection (textbook contaminated Gaussian)
    let mut with_two_outliers = x_norm.to_vec();
    with_two_outliers.push(35.0);
    with_two_outliers.push(42.0);
    let g_single = grubbs_outliers(&with_two_outliers, 0.05, false).unwrap();
    assert_eq!(g_single.outliers.len(), 1);
    assert_approx_eq(g_single.outliers[0].value, 42.0, 1e-12, 0.0, "single Grubbs flags max outlier");

    let g_iter = grubbs_outliers(&with_two_outliers, 0.05, true).unwrap();
    assert_eq!(g_iter.outliers.len(), 2);
    assert_eq!(g_iter.cleaned_values.len(), x_norm.len());

    // ROUT Q=1% catches multiple masked outliers without false positives on core
    let r_res = rout_outliers(&with_two_outliers, 1.0).unwrap();
    assert_eq!(r_res.outliers.len(), 2);
    assert!((r_res.robust_center - 10.2).abs() < 0.3);
}

// ============================================================================
// 3. One-Sample & Two-Group Parametric Tests, Effect Sizes & Bland-Altman
// ============================================================================

#[test]
fn oracle_one_sample_and_two_group_parametric_r_reference() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::t.test(x_norm, mu=9.75), 2026-11-13
    let x_norm = [10.2, 9.8, 10.5, 11.1, 9.4, 10.0, 10.7, 9.9, 10.3, 10.1, 9.6, 10.8];
    let t1 = one_sample_t_test(&x_norm, 9.75, true, conf_95()).unwrap();
    assert_approx_eq(t1.t_statistic, 3.078_753_547_177_055, 1e-11, 1e-11, "1-sample t");
    assert_approx_eq(t1.df, 11.0, 1e-15, 0.0, "1-sample df");
    assert_approx_eq(t1.p_value, 0.010_494_629_346_663, 1e-10, 1e-10, "1-sample p");
    // `one_sample_t_test` reports the 95% CI of the discrepancy `mean - mu`: (9.878297302182766 - 9.75, 10.521702697817233 - 9.75)
    assert_approx_eq(t1.discrepancy, 0.45, 1e-12, 0.0, "1-sample discrepancy");
    assert_approx_eq(t1.ci.0, 9.878_297_302_182_766 - 9.75, 1e-11, 1e-11, "1-sample discrepancy ci lo");
    assert_approx_eq(t1.ci.1, 10.521_702_697_817_233 - 9.75, 1e-11, 1e-11, "1-sample discrepancy ci hi");
    let t1_one = one_sample_t_test(&x_norm, 9.75, false, conf_95()).unwrap();
    assert_approx_eq(t1_one.p_value, 0.5 * t1.p_value, 1e-12, 1e-12, "1-tailed p");

    // One-sample Wilcoxon on x_norm - 9.75 (exact small-sample / ties vs R wilcox.test)
    let w1 = one_sample_wilcoxon(&x_norm, 9.75, true, conf_95()).unwrap();
    assert_approx_eq(w1.w_plus, 70.0, 1e-12, 0.0, "R 1-sample wilcoxon V");
    assert_approx_eq(w1.p_value, 0.016_644_883_860_113, 2e-3, 0.05, "R 1-sample wilcoxon p");

    // D1 Provenance: R 4.5.2 (2025-10-31), stats::t.test(g_a, g_b, var.equal=T/F), var.test, 2026-11-13
    let g_a = [23.1, 25.4, 21.8, 24.0, 26.2, 22.7];
    let g_b = [18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6];

    let ut = unpaired_t_test(&g_a, &g_b, true, conf_95()).unwrap();
    assert_approx_eq(ut.difference, 4.380_952_380_952_38, 1e-12, 1e-12, "unpaired diff");
    assert_approx_eq(ut.t_statistic, 5.447_846_144_605_422, 1e-11, 1e-11, "unpaired t");
    assert_approx_eq(ut.df, 11.0, 1e-15, 0.0, "unpaired df");
    assert_approx_eq(ut.p_value, 0.000_201_493_168_147, 1e-10, 1e-10, "unpaired p");
    assert_approx_eq(ut.ci.0, 2.611_003_134_663_101, 1e-11, 1e-11, "unpaired ci lo");
    assert_approx_eq(ut.ci.1, 6.150_901_627_241_659, 1e-11, 1e-11, "unpaired ci hi");
    // Exact Cohen's d, Hedges' g with exact Gamma J(df), and noncentral-t CI (R 4.5.2 uniroot on stats::pt)
    assert_approx_eq(ut.effect_sizes.cohens_d, 3.030_901_794_919_719, 1e-11, 1e-11, "Cohen d");
    assert_approx_eq(ut.effect_sizes.hedges_g, 2.818_616_857_258_788, 1e-11, 1e-11, "Hedges g (exact J)");
    let d_ci = ut.effect_sizes.cohens_d_ci.unwrap();
    assert_approx_eq(d_ci.0, 1.343_137_646_552_742, 1e-5, 1e-5, "noncentral-t Cohen d CI lo");
    assert_approx_eq(d_ci.1, 4.660_639_297_992_52, 1e-5, 1e-5, "noncentral-t Cohen d CI hi");

    let wt = welch_t_test(&g_a, &g_b, true, conf_95()).unwrap();
    assert_approx_eq(wt.t_statistic, 5.309_410_682_787_802, 1e-11, 1e-11, "welch t");
    assert_approx_eq(wt.df, 9.030_786_266_629_29, 1e-11, 1e-11, "welch df");
    assert_approx_eq(wt.p_value, 0.000_482_239_066_274, 1e-10, 1e-10, "welch p");
    assert_approx_eq(wt.ci.0, 2.515_348_876_111_283, 1e-11, 1e-11, "welch ci lo");
    assert_approx_eq(wt.ci.1, 6.246_555_885_793_476, 1e-11, 1e-11, "welch ci hi");

    let ft = f_test_variances(&g_a, &g_b).unwrap();
    assert_approx_eq(ft.f_statistic, 1.881_864_623_243_93, 1e-11, 1e-11, "var.test F");
    assert_approx_eq(ft.p_value, 0.463_492_555_478_103, 1e-10, 1e-10, "var.test p");
    assert!(levene_variance_test(&[&g_a, &g_b], true).unwrap().p_value > 0.2);

    // Paired t-test, Ratio paired t-test, Lognormal unpaired t, Bland-Altman, Multiple Row t
    let pre = [120.0, 132.0, 118.0, 140.0, 125.0, 135.0, 128.0, 144.0];
    let post = [112.0, 124.0, 115.0, 129.0, 121.0, 126.0, 120.0, 133.0];
    let pt = paired_t_test(&pre, &post, true, conf_95()).unwrap();
    assert_approx_eq(pt.mean_of_differences, 7.75, 1e-12, 0.0, "paired mean diff");
    assert_approx_eq(pt.t_statistic, 7.518_604_376_126_322, 1e-11, 1e-11, "paired t");
    assert_approx_eq(pt.p_value, 0.000_135_173_622_682, 1e-10, 1e-10, "paired p");
    assert_approx_eq(pt.ci.0, 5.312_601_111_446_46, 1e-11, 1e-11, "paired ci lo");
    assert_approx_eq(pt.ci.1, 10.187_398_888_553_54, 1e-11, 1e-11, "paired ci hi");

    let rp = ratio_paired_t_test(&pre, &post, true, conf_95()).unwrap();
    assert_approx_eq(rp.t_statistic, 8.319_040_144_768_511, 1e-11, 1e-11, "ratio paired t");
    assert_approx_eq(rp.p_value, 0.000_070_949_571_748, 1e-10, 1e-10, "ratio paired p");
    assert_approx_eq(rp.geometric_mean_ratio.unwrap(), 1.062_449_203_616_707, 1e-11, 1e-11, "ratio paired geom");
    let rci = rp.geometric_mean_ratio_ci.unwrap();
    assert_approx_eq(rci.0, 1.044_312_016_827_679, 1e-11, 1e-11, "ratio ci lo");
    assert_approx_eq(rci.1, 1.080_901_389_696_483, 1e-11, 1e-11, "ratio ci hi");

    let lnu = lognormal_unpaired_t_test(&g_a, &g_b, true, conf_95()).unwrap();
    assert!(lnu.geometric_mean_ratio.unwrap() > 1.0);

    let ba = bland_altman(&pre, &post, false, conf_95()).unwrap();
    assert_approx_eq(ba.bias, pt.mean_of_differences, 1e-12, 0.0, "BA bias == paired mean diff");
    assert_approx_eq(ba.sd_bias, pt.sd_of_differences, 1e-12, 0.0, "BA sd");
    assert_approx_eq(ba.bias_ci.0, pt.ci.0, 1e-12, 0.0, "BA bias ci lo");
    assert_approx_eq(ba.bias_ci.1, pt.ci.1, 1e-12, 0.0, "BA bias ci hi");
    let expected_z_loa = 1.959_963_984_540_054;
    assert_approx_eq(ba.loa_upper, ba.bias + expected_z_loa * ba.sd_bias, 1e-9, 1e-9, "BA LoA upper");
    assert_approx_eq(ba.loa_lower, ba.bias - expected_z_loa * ba.sd_bias, 1e-9, 1e-9, "BA LoA lower");

    let row_a = vec![vec![10.0, 10.5, 11.0, 10.2], vec![5.0, 5.2, 4.9, 5.1], vec![20.0, 20.1, 19.9, 20.0]];
    let row_b = vec![vec![10.1, 10.6, 10.9, 10.3], vec![8.1, 8.3, 8.0, 8.2], vec![19.8, 20.2, 20.0, 20.1]];
    let labels = vec![String::from("Null1"), String::from("Signal"), String::from("Null2")];
    let mr = multiple_row_t_tests(&labels, &row_a, &row_b, false, MultiplicityProcedure::TwoStageBkyFdr, 0.05).unwrap();
    assert!(!mr[0].significant && mr[1].significant && !mr[2].significant);
}

// ============================================================================
// 4. Mann-Whitney U (Exact & Asymptotic), Wilcoxon Signed-Rank & 2-Sample KS
// ============================================================================

#[test]
fn oracle_nonparametric_mann_whitney_wilcoxon_and_ks() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::wilcox.test(..., exact=T/F, conf.int=T), 2026-11-13
    let g_a = [23.1, 25.4, 21.8, 24.0, 26.2, 22.7];
    let g_b = [18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6];
    let mw_ex = mann_whitney_u(&g_a, &g_b, true, conf_95()).unwrap();
    assert!(mw_ex.exact_p_used, "n=13 untied should use exact DP");
    assert_approx_eq(mw_ex.u_min, 0.0, 1e-12, 0.0, "exact MW U_min");
    assert_approx_eq(mw_ex.u_a, 42.0, 1e-12, 0.0, "exact MW U_a == R W");
    assert_approx_eq(mw_ex.p_value, 2.0 / 1716.0, 1e-12, 1e-12, "exact MW 2/C(13,6)");
    assert_approx_eq(mw_ex.hodges_lehmann_diff, 4.25, 1e-12, 0.0, "HL median diff");
    let hl_ci = mw_ex.hodges_lehmann_ci.unwrap();
    assert_approx_eq(hl_ci.0, 2.5, 1e-11, 0.0, "HL CI lower");
    assert_approx_eq(hl_ci.1, 6.6, 1e-11, 0.0, "HL CI upper");
    assert_approx_eq(mw_ex.cliffs_delta, 1.0, 1e-12, 0.0, "Cliff's delta");

    // Tied samples -> asymptotic continuity-corrected normal approximation
    let t_a = [4.0, 5.0, 5.0, 7.0, 8.0, 9.0, 10.0, 10.0, 12.0, 14.0];
    let t_b = [2.0, 3.0, 5.0, 5.0, 6.0, 7.0, 7.0, 8.0, 11.0];
    let mw_tie = mann_whitney_u(&t_a, &t_b, true, conf_95()).unwrap();
    assert!(!mw_tie.exact_p_used, "ties trigger asymptotic MW");
    assert_approx_eq(mw_tie.u_a, 63.5, 1e-12, 0.0, "tied MW W");
    assert_approx_eq(mw_tie.p_value, 0.138_841_935_618_501, 1e-10, 1e-10, "tied MW asymptotic cc p");
    assert_approx_eq(mw_tie.hodges_lehmann_diff, 2.0, 1e-11, 0.0, "tied MW HL");

    // Wilcoxon matched-pairs exact (no ties, small n) + tied asymptotic + Pratt zero handling
    let xa = [10.1, 11.3, 12.5, 14.2, 15.8, 17.1, 18.9, 20.6];
    let xb = [9.0, 10.1, 11.1, 12.6, 14.0, 15.1, 16.7, 18.2];
    // Strict monotone increasing diffs: [1.1, 1.2, 1.4, 1.6, 1.8, 2.0, 2.2, 2.4] -> all positive, n=8, exact 2-sided p = 2 / 2^8 = 1/128
    let wm_ex = wilcoxon_matched_pairs(&xa, &xb, true, conf_95(), false).unwrap();
    assert!(wm_ex.exact_p_used);
    assert_approx_eq(wm_ex.w_plus, 36.0, 1e-12, 0.0, "exact W+");
    assert_approx_eq(wm_ex.w_minus, 0.0, 1e-12, 0.0, "exact W-");
    assert_approx_eq(wm_ex.p_value, 1.0 / 128.0, 1e-12, 0.0, "exact Wilcoxon p = 2/2^8");

    // Tied + zero pairs (R wilcox.test(p1, p2, paired=TRUE, exact=FALSE, correct=TRUE))
    let p1 = [10.0, 12.0, 15.0, 14.0, 18.0, 20.0, 22.0, 19.0, 16.0, 15.0, 25.0, 28.0];
    let p2 = [8.0, 12.0, 11.0, 12.0, 14.0, 16.0, 22.0, 15.0, 13.0, 12.0, 19.0, 21.0];
    let wm_tie = wilcoxon_matched_pairs(&p1, &p2, true, conf_95(), false).unwrap();
    assert!(!wm_tie.exact_p_used);
    assert_eq!(wm_tie.n_pairs, 12);
    assert_eq!(wm_tie.n_effective, 10);
    assert_approx_eq(wm_tie.w_plus, 55.0, 1e-12, 0.0, "tied wilcoxon V");
    assert_approx_eq(wm_tie.p_value, 0.005_540_680_462_066, 1e-10, 1e-10, "tied wilcoxon R p");

    // Pratt zero mode retains zero-diff pairs with effective n=12 and assigns zero ranks 1..2 (sum 3) dropped from signed sums
    let wm_pratt = wilcoxon_matched_pairs(&p1, &p2, true, conf_95(), true).unwrap();
    assert_eq!(wm_pratt.n_effective, 12);
    // Zeros take ranks 1, 2 (mean 1.5); nonzeros (2,2,3,3,4,4,4,4,6,7) occupy ranks 3..12 -> sum = 78 - 3 = 75
    assert_approx_eq(wm_pratt.w_plus, 75.0, 1e-12, 0.0, "Pratt W+");

    // Exact 2-sample KS (R ks.test(g_a, g_b): D = 1.0, p = 1 / C(13,6) * 2 = 0.001165501165501)
    let ks = ks_two_sample(&g_a, &g_b).unwrap();
    assert!(ks.exact_p_used);
    assert_approx_eq(ks.d_statistic, 1.0, 1e-15, 0.0, "KS D");
    assert_approx_eq(ks.p_value, 2.0 / 1716.0, 1e-12, 1e-12, "exact KS Smirnov p");
}

// ============================================================================
// 5. NIST StRD One-Way ANOVA + Welch/BF One-Way + RM One-Way Sphericity
// ============================================================================

#[test]
fn oracle_nist_strd_one_way_anova_and_summary_roundtrip() {
    let mut sets = all_offline_anova_benchmarks();
    if let Some(dir) = require_corpus_dir() {
        sets.extend(load_corpus_anova_all(&dir).unwrap());
        assert!(sets.len() >= 12);
    }

    for ds in sets {
        let mut by_grp = std::collections::BTreeMap::<usize, Vec<f64>>::new();
        for &(g, y) in &ds.observations {
            by_grp.entry(g).or_default().push(y);
        }
        let owned: Vec<(String, Vec<f64>)> = by_grp.into_iter().map(|(g, v)| (format!("G{g}"), v)).collect();
        let refs: Vec<(&str, &[f64])> = owned.iter().map(|(l, v)| (l.as_str(), v.as_slice())).collect();

        let res = ordinary_one_way_anova(&refs, conf_95()).unwrap();
        let min_lre = match ds.name.as_str() {
            "SmLs07" | "SmLs08" | "SmLs09" => 3.0,
            "AtmWtAg" => 8.5,
            _ => 9.0,
        };
        assert_approx_eq(res.df_between, ds.certified_df_between as f64, 1e-12, 0.0, &ds.name);
        assert_approx_eq(res.df_within, ds.certified_df_within as f64, 1e-12, 0.0, &ds.name);
        assert_lre_at_least(res.ss_between, ds.certified_ss_between, min_lre, &format!("{} SSb", ds.name));
        assert_lre_at_least(res.ss_within, ds.certified_ss_within, min_lre, &format!("{} SSw", ds.name));
        assert_lre_at_least(res.f_statistic, ds.certified_f, min_lre, &format!("{} F", ds.name));
        assert_lre_at_least(res.r_squared, ds.certified_r_squared, min_lre, &format!("{} R2", ds.name));
        assert_lre_at_least(res.ms_within.sqrt(), ds.certified_residual_sd, min_lre, &format!("{} resid_sd", ds.name));

        // Summary-input ANOVA agrees on moderate-condition datasets
        if !ds.name.starts_with("SmLs0") {
            let g_inputs: Vec<GroupComparisonInput> = res
                .group_labels
                .iter()
                .enumerate()
                .map(|(i, lbl)| GroupComparisonInput {
                    index: i,
                    label: lbl.clone(),
                    n: res.group_n[i] as f64,
                    mean: res.group_means[i],
                    sd: res.group_sds[i],
                })
                .collect();
            let from_sum = ordinary_one_way_from_summary(&g_inputs, conf_95()).unwrap();
            assert_lre_at_least(from_sum.f_statistic, ds.certified_f, 8.0, &format!("{} summary F", ds.name));
        }
    }
}

#[test]
fn oracle_unbalanced_one_way_welch_bf_effect_sizes_and_rm_sphericity() {
    // D1 Provenance: R 4.5.2 (2025-10-31), aov, oneway.test(var.equal=F), bartlett.test, 2026-11-13
    let ctrl = [6.2, 6.5, 6.8, 7.1, 6.4];
    let low = [7.9, 8.3, 8.0, 8.6, 8.2, 8.5];
    let high = [9.4, 9.8, 9.1, 9.6];
    let mid = [7.0, 7.3, 6.9, 7.4, 7.2];

    let ow = ordinary_one_way_anova(&[("Ctrl", &ctrl), ("Low", &low), ("High", &high), ("Mid", &mid)], conf_95()).unwrap();
    assert_approx_eq(ow.ss_between, 21.683_5, 1e-11, 1e-11, "SSb");
    assert_approx_eq(ow.ss_within, 1.314_5, 1e-11, 1e-11, "SSw");
    assert_approx_eq(ow.f_statistic, 87.976_670_470_394_3, 1e-10, 1e-10, "F");
    assert_approx_eq(ow.p_value, 3.704_99e-10, 1e-13, 1e-5, "p");
    assert_approx_eq(ow.variance_tests.bartlett_chi2, 1.011_805_142_725_196, 1e-9, 1e-9, "Bartlett K2");
    assert_approx_eq(ow.variance_tests.bartlett_p, 0.798_395_490_941_963, 1e-9, 1e-9, "Bartlett p");

    let g_inputs: Vec<GroupComparisonInput> = ow
        .group_labels
        .iter()
        .enumerate()
        .map(|(i, l)| GroupComparisonInput { index: i, label: l.clone(), n: ow.group_n[i] as f64, mean: ow.group_means[i], sd: ow.group_sds[i] })
        .collect();
    let uv = unequal_variance_one_way_anova(&g_inputs).unwrap();
    assert_approx_eq(uv.welch_w, 70.804_286_056_555_14, 1e-9, 1e-9, "Welch W");
    assert_approx_eq(uv.welch_df2, 8.227_697_311_021_101, 1e-9, 1e-9, "Welch df2");
    assert_approx_eq(uv.welch_p, 3.261_098_737e-6, 1e-10, 1e-6, "Welch p");
    assert_approx_eq(uv.brown_forsythe_f_star, 86.791_861_240_827_25, 1e-9, 1e-9, "BF F*");
    assert_approx_eq(uv.brown_forsythe_df2, 13.266_938_969_106_556, 1e-9, 1e-9, "BF df2");
    assert_approx_eq(uv.brown_forsythe_p, 5.731_166e-9, 1e-12, 1e-5, "BF p");

    // Analytical omega^2 = (SS_b - df_b * MS_w) / (SS_tot + MS_w)
    let expected_omega2 = (ow.ss_between - 3.0 * ow.ms_within) / (ow.ss_total + ow.ms_within);
    let eff = compute_anova_effect_sizes(ow.ss_between, ow.ss_within, ow.ss_total, 3.0, 16.0, conf_95());
    assert_approx_eq(eff.omega_squared, expected_omega2, 1e-12, 0.0, "omega^2");
    assert_approx_eq(eff.eta_squared, ow.r_squared, 1e-12, 0.0, "eta^2 == R^2");
    let e_ci = eff.eta_squared_ci.unwrap();
    assert!(e_ci.0 < eff.eta_squared && eff.eta_squared < e_ci.1);
    assert!(variance_homogeneity_tests(&[&ctrl, &low, &high, &mid]).unwrap().brown_forsythe_p > 0.1);

    // RM One-Way ANOVA & Sphericity (R aov + exact covariance contrast eigenvalues)
    let blocks = vec![
        vec![10.2, 12.1, 14.5, 13.8],
        vec![11.0, 12.8, 15.1, 14.2],
        vec![9.5, 11.4, 13.9, 12.7],
        vec![12.1, 13.5, 16.0, 15.4],
        vec![10.8, 12.0, 14.2, 13.6],
        vec![11.4, 13.1, 15.6, 14.9],
    ];
    let rm = rm_one_way_anova(&blocks).unwrap();
    assert_approx_eq(rm.ss_treatment, 58.175, 1e-10, 0.0, "RM SS_cond");
    assert_approx_eq(rm.ss_subjects, 14.71, 1e-10, 0.0, "RM SS_subj");
    assert_approx_eq(rm.ss_residual, 0.520, 1e-10, 0.0, "RM SS_res");
    assert_approx_eq(rm.f_treatment, 559.375, 1e-9, 1e-9, "RM F");
    assert_approx_eq(rm.p_uncorrected, 1.3019e-15, 1e-16, 1e-4, "RM p");
    assert_approx_eq(rm.sphericity.mauchly_w, 0.188_054_733_727_821, 1e-7, 1e-6, "Mauchly W");
    assert_approx_eq(rm.sphericity.mauchly_chi2, 6.219_916_047_342_806, 1e-6, 1e-6, "Mauchly chi2");
    assert_approx_eq(rm.sphericity.mauchly_p, 0.285_404_511_048_548, 1e-6, 1e-5, "Mauchly p");
    assert_approx_eq(rm.sphericity.greenhouse_geisser_eps, 0.604_380_867_232_905, 1e-8, 1e-8, "GG eps");
    assert_approx_eq(rm.sphericity.huynh_feldt_eps, 0.928_695_001_636_518, 1e-7, 1e-7, "HF eps");
}

// ============================================================================
// 6. Kruskal-Wallis + Dunn + Friedman (with Ties & Exact Kendall's W)
// ============================================================================

#[test]
fn oracle_kruskal_wallis_dunn_and_friedman_with_ties() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::kruskal.test, stats::friedman.test, 2026-11-13
    let g1 = [12.1, 14.3, 14.3, 15.0, 16.2];
    let g2 = [18.5, 19.0, 19.0, 21.4, 22.0];
    let g3 = [15.0, 16.8, 17.2, 18.5, 20.1];
    let kw = kruskal_wallis_test(&[&g1, &g2, &g3]).unwrap();
    assert_approx_eq(kw.h_statistic, 10.258_273_381_294_973, 1e-10, 1e-10, "KW H with ties");
    assert_approx_eq(kw.df, 2.0, 1e-15, 0.0, "KW df");
    assert_approx_eq(kw.p_value, 0.005_921_670_501_123, 1e-10, 1e-10, "KW p");

    // Dunn pairwise comparisons with ties (exact tie sum from g1..g3)
    // Ties: 14.3 (x2), 15.0 (x2), 18.5 (x2), 19.0 (x2) -> 4 pairs -> sum(t^3-t) = 4*(8-2) = 24
    let tie_sum = 24.0;
    let labels = [(0, String::from("G1")), (1, String::from("G2")), (2, String::from("G3"))];
    let dunn_bonf = compare_dunn_ranks(
        &labels,
        &[5, 5, 5],
        &kw.group_mean_ranks,
        kw.total_n,
        tie_sum,
        &MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert_eq!(dunn_bonf.len(), 3);
    // Pair 0-1 (G1 vs G2): signed (mean_rank_G1 - mean_rank_G2) = -9.0, z = -3.193405975853791
    assert_approx_eq(dunn_bonf[0].statistic, -3.193_405_975_853_791, 1e-10, 1e-10, "Dunn G1-G2 z");
    assert_approx_eq(dunn_bonf[0].raw_p, 0.001_406_051_181_546, 1e-10, 1e-10, "Dunn G1-G2 raw_p");
    assert_approx_eq(dunn_bonf[0].adjusted_p_or_q, 0.004_218_153_544_639, 1e-10, 1e-10, "Dunn G1-G2 Bonf");
    assert_approx_eq(dunn_bonf[1].raw_p, 0.070_358_349_013_086, 1e-10, 1e-10, "Dunn G1-G3 raw_p");
    assert_approx_eq(dunn_bonf[2].raw_p, 0.166_416_868_049_653, 1e-10, 1e-10, "Dunn G2-G3 raw_p");

    let dunn_sidak = compare_dunn_ranks(
        &labels,
        &[5, 5, 5],
        &kw.group_mean_ranks,
        kw.total_n,
        tie_sum,
        &MultiplicitySpec { method: MultiplicityMethod::Sidak, threshold: 0.05, report_confidence_intervals: false },
    )
    .unwrap();
    assert_approx_eq(dunn_sidak[0].adjusted_p_or_q, 0.004_212_225_384_598, 1e-10, 1e-10, "Dunn G1-G2 Sidak");
    assert_approx_eq(dunn_sidak[1].adjusted_p_or_q, 0.196_572_449_955_169, 1e-10, 1e-10, "Dunn G1-G3 Sidak");

    // Friedman with within-block ties (R stats::friedman.test)
    let fr_blocks = vec![
        vec![5.1, 6.2, 7.4, 8.0],
        vec![4.8, 5.5, 6.9, 7.1],
        vec![6.0, 6.0, 7.8, 8.5],
        vec![5.3, 6.1, 6.1, 7.9],
        vec![4.5, 5.8, 7.0, 7.6],
        vec![5.9, 6.4, 7.2, 8.1],
    ];
    let fr = friedman_test(&fr_blocks).unwrap();
    assert_approx_eq(fr.q_statistic, 17.431_034_482_758_62, 1e-10, 1e-10, "Friedman Q ties");
    assert_approx_eq(fr.df, 3.0, 1e-15, 0.0, "Friedman df");
    assert_approx_eq(fr.p_value, 0.000_576_179_488_694, 1e-10, 1e-10, "Friedman p ties");
    assert_approx_eq(fr.kendalls_w, fr.q_statistic / (6.0 * 3.0), 1e-12, 0.0, "Kendall W");
}

// ============================================================================
// 7. Two-Way & Three-Way ANOVA (Type I & III), RM Two-Way, REML & Nested
// ============================================================================

#[test]
fn oracle_two_way_and_three_way_anova_type_i_and_iii() {
    // D1 Provenance: R 4.5.2 (2025-10-31), aov (Type I) & contr.sum drop1 (Type III), 2026-11-13
    let tw_rows = vec![
        (vec![0, 0], 12.1),
        (vec![0, 0], 13.0),
        (vec![0, 0], 11.8),
        (vec![0, 0], 12.5),
        (vec![0, 1], 15.2),
        (vec![0, 1], 16.0),
        (vec![0, 1], 14.9),
        (vec![0, 2], 10.4),
        (vec![0, 2], 11.1),
        (vec![1, 0], 18.5),
        (vec![1, 0], 19.2),
        (vec![1, 0], 18.8),
        (vec![1, 0], 19.7),
        (vec![1, 0], 18.1),
        (vec![1, 1], 14.1),
        (vec![1, 1], 13.8),
        (vec![1, 1], 14.5),
        (vec![1, 2], 17.2),
        (vec![1, 2], 16.9),
        (vec![1, 2], 17.5),
        (vec![1, 2], 17.0),
    ];
    let tw_obs = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B")],
        factor_levels: vec![vec![String::from("A1"), String::from("A2")], vec![String::from("B1"), String::from("B2"), String::from("B3")]],
        rows: tw_rows,
    };

    let res1 = fit_factorial_anova(&tw_obs, 2, SumOfSquaresType::TypeI).unwrap();
    assert!(res1.unbalanced);
    assert_approx_eq(res1.ss_residual, 3.690_333_333_333_3, 1e-9, 1e-9, "2way resid SS");
    assert_approx_eq(res1.terms[0].ss, 86.803_214_285_714_3, 1e-9, 1e-9, "2way Type I A");
    assert_approx_eq(res1.terms[1].ss, 7.927_206_959_707_0, 1e-9, 1e-9, "2way Type I B");
    assert_approx_eq(res1.terms[2].ss, 62.591_626_373_626_4, 1e-9, 1e-9, "2way Type I A:B");

    let res3 = fit_factorial_anova(&tw_obs, 2, SumOfSquaresType::TypeIII).unwrap();
    assert_approx_eq(res3.terms[0].ss, 73.041_720_238_095_2, 1e-9, 1e-9, "2way Type III A");
    assert_approx_eq(res3.terms[0].f_statistic.unwrap(), 296.890_74, 1e-5, 1e-5, "2way Type III F_A");
    assert_approx_eq(res3.terms[1].ss, 9.361_882_783_882_8, 1e-9, 1e-9, "2way Type III B");
    assert_approx_eq(res3.terms[1].f_statistic.unwrap(), 19.026_50, 1e-5, 1e-5, "2way Type III F_B");
    assert_approx_eq(res3.terms[2].ss, 62.591_626_373_626_4, 1e-9, 1e-9, "2way Type III A:B");

    // Three-way 2 x 2 x 3 unbalanced (all 7 terms: A, B, C, A×B, A×C, B×C, A×B×C)
    let th_data = vec![
        (0, 0, 0, vec![5.1, 5.4, 4.9]),
        (0, 0, 1, vec![6.2, 6.5]),
        (0, 0, 2, vec![7.8, 8.1]),
        (0, 1, 0, vec![6.0, 6.3]),
        (0, 1, 1, vec![8.2, 8.5, 8.0]),
        (0, 1, 2, vec![9.1, 9.4]),
        (1, 0, 0, vec![7.2, 7.5]),
        (1, 0, 1, vec![8.8, 9.0]),
        (1, 0, 2, vec![10.5, 10.9]),
        (1, 1, 0, vec![9.5, 9.8]),
        (1, 1, 1, vec![12.1, 12.4]),
        (1, 1, 2, vec![14.0, 14.3, 13.8]),
    ];
    let mut th_rows = Vec::new();
    for (a, b, c, ys) in th_data {
        for y in ys {
            th_rows.push((vec![a, b, c], y));
        }
    }
    let th_obs = FactorialObservations {
        factor_names: vec![String::from("A"), String::from("B"), String::from("C")],
        factor_levels: vec![
            vec![String::from("A1"), String::from("A2")],
            vec![String::from("B1"), String::from("B2")],
            vec![String::from("C1"), String::from("C2"), String::from("C3")],
        ],
        rows: th_rows,
    };
    let th3 = fit_factorial_anova(&th_obs, 3, SumOfSquaresType::TypeIII).unwrap();
    assert_eq!(th3.terms.len(), 7);
    assert_approx_eq(th3.df_residual, 15.0, 1e-12, 0.0, "3way resid df");
    assert_approx_eq(th3.ss_residual, 0.795, 1e-10, 1e-10, "3way resid SS");
    let exp_ss = [
        71.400_050_505_050_5,
        31.600_050_505_050_508,
        50.957_272_727_272_73,
        4.160_050_505_050_51,
        0.901_313_131_313_131,
        1.048_585_858_585_857,
        0.166_767_676_767_676,
    ];
    for (i, &ess) in exp_ss.iter().enumerate() {
        assert_approx_eq(th3.terms[i].ss, ess, 1e-9, 1e-9, &format!("3way Type III term {i} SS"));
    }
    let th1 = fit_factorial_anova(&th_obs, 3, SumOfSquaresType::TypeI).unwrap();
    assert_approx_eq(th1.terms[0].ss, 89.641_369_556_369_6, 1e-9, 1e-9, "3way Type I A");
    assert_approx_eq(th1.terms[6].ss, 0.166_767_676_767_7, 1e-9, 1e-9, "3way Type I A:B:C");
}

#[test]
fn oracle_rm_two_way_reml_mixed_and_nested_anova() {
    // D1 Provenance: R 4.5.2 (2025-10-31), aov(Error), nlme::lme(REML), 2026-11-13
    let rm2_data = vec![
        vec![vec![10.1, 11.2, 13.0, 12.5], vec![12.0, 13.4, 15.2, 14.8], vec![14.1, 15.0, 16.8, 16.2]],
        vec![vec![11.0, 12.0, 13.9, 13.1], vec![12.9, 14.1, 16.0, 15.5], vec![15.0, 15.9, 17.7, 17.0]],
        vec![vec![9.4, 10.6, 12.2, 11.8], vec![11.3, 12.6, 14.5, 14.0], vec![13.2, 14.3, 16.1, 15.4]],
        vec![vec![10.6, 11.7, 13.4, 12.9], vec![12.4, 13.8, 15.7, 15.1], vec![14.5, 15.5, 17.2, 16.7]],
        vec![vec![11.5, 12.6, 14.2, 13.8], vec![13.5, 14.7, 16.6, 16.0], vec![15.4, 16.4, 18.1, 17.5]],
    ];

    let (both, _) = fit_rm_two_way_anova(&rm2_data, "Dose", "Time", RepeatedMeasuresDesign::BothFactorsMatched).unwrap();
    assert_approx_eq(both.terms[0].f_statistic.unwrap(), 16_176.550_46, 1e-4, 1e-5, "RM2 Both F_A");
    assert_approx_eq(both.terms[1].f_statistic.unwrap(), 4_401.769_23, 1e-4, 1e-5, "RM2 Both F_B");
    assert_approx_eq(both.terms[2].f_statistic.unwrap(), 21.498_01, 1e-4, 1e-5, "RM2 Both F_AB");

    let (col_rm, _) = fit_rm_two_way_anova(&rm2_data, "Dose", "Time", RepeatedMeasuresDesign::ColumnFactorMatched).unwrap();
    assert_approx_eq(col_rm.terms[0].f_statistic.unwrap(), 29.302_42, 1e-4, 1e-5, "RM2 Col F_A");
    assert_approx_eq(col_rm.terms[1].f_statistic.unwrap(), 5_984.104_58, 1e-4, 1e-5, "RM2 Col F_B");
    assert_approx_eq(col_rm.terms[2].f_statistic.unwrap(), 17.633_99, 1e-4, 1e-5, "RM2 Col F_AB");

    let (row_rm, _) = fit_rm_two_way_anova(&rm2_data, "Dose", "Time", RepeatedMeasuresDesign::RowFactorMatched).unwrap();
    assert_approx_eq(row_rm.terms[0].f_statistic.unwrap(), 19_591.600_00, 1e-4, 1e-5, "RM2 Row F_A");
    assert_approx_eq(row_rm.terms[1].f_statistic.unwrap(), 13.509_93, 1e-4, 1e-5, "RM2 Row F_B");
    assert_approx_eq(row_rm.terms[2].f_statistic.unwrap(), 19.985_19, 1e-4, 1e-5, "RM2 Row F_AB");

    // REML Mixed-Effects (unbalanced cluster sizes) vs nlme::lme
    let y_reml = vec![10.1, 10.4, 9.9, 12.2, 12.5, 11.9, 12.1, 8.8, 9.1, 14.2, 14.5, 14.0, 16.1, 15.8, 16.4, 13.0, 13.3, 12.7, 13.1];
    let c_reml = vec![0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 5, 5];
    let mut x_raw = vec![0.0_f64; y_reml.len() * 2];
    for i in 0..y_reml.len() {
        x_raw[i * 2] = 1.0;
        x_raw[i * 2 + 1] = if i < 9 { 1.0 } else { -1.0 };
    }
    let x_mat = DenseMatrix::from_row_major(y_reml.len(), 2, x_raw).unwrap();
    let reml = fit_random_intercept_reml(&x_mat, &y_reml, &c_reml, &[("Group", 1, 1)]).unwrap();
    assert_approx_eq(reml.sigma2_random, 2.509_787_435_551_319, 5e-4, 5e-4, "REML sigma2_random");
    assert_approx_eq(reml.sigma2_error, 0.065_637_517_942_556, 5e-4, 5e-4, "REML sigma2_error");
    assert_approx_eq(reml.fixed_terms[0].f_statistic.unwrap(), 9.617_568_216_146_067, 1e-3, 1e-3, "REML Wald F");

    // Balanced Nested t-test (2 groups x 4 clusters x 3 replicates) vs R aov + nlme::lme
    let mut nest_b = NestedTable::new(2, 4, 3).unwrap();
    nest_b.groups[0].title = String::from("Ctrl");
    nest_b.groups[1].title = String::from("Drug");
    let ctrl_b = [[10.1, 10.3, 9.9], [11.4, 11.6, 11.2], [9.5, 9.7, 9.4], [10.8, 11.0, 10.6]];
    let drug_b = [[14.2, 14.4, 14.0], [15.6, 15.8, 15.4], [13.7, 13.9, 13.5], [14.9, 15.1, 14.7]];
    for u in 0..4 {
        for r in 0..3 {
            nest_b.groups[0].cells[r][u] = Cell::number(ctrl_b[u][r]);
            nest_b.groups[1].cells[r][u] = Cell::number(drug_b[u][r]);
        }
    }
    let nb = nested_one_way_anova(&nest_b, true).unwrap();
    assert_approx_eq(nb.ss_groups, 102.920_416_666_67, 1e-9, 1e-9, "Nested balanced SS_grp");
    assert_approx_eq(nb.ss_subclusters, 12.142_5, 1e-9, 1e-9, "Nested balanced SS_sub");
    assert_approx_eq(nb.ss_error, 0.606_666_666_666_67, 1e-9, 1e-9, "Nested balanced SS_err");
    assert_approx_eq(nb.f_among_groups, 50.856_290_988_689_66, 1e-3, 1e-3, "Nested balanced F");
    assert!(nb.t_nested.is_some());
    assert_approx_eq(nb.t_nested.unwrap().powi(2), nb.f_among_groups, 1e-12, 1e-12, "nested t^2 == F");
    assert_approx_eq(nb.variance_subclusters, 0.661_944_429_780_225, 1e-3, 1e-3, "Nested balanced var_sub");
    assert_approx_eq(nb.variance_error, 0.037_916_666_973_600, 1e-3, 1e-3, "Nested balanced var_err");

    // Unbalanced Nested one-way ANOVA (3 groups, unequal cluster sizes) vs nlme::lme
    let mut nest_u = NestedTable::new(3, 3, 4).unwrap();
    let u_vals: Vec<Vec<Vec<f64>>> = vec![
        vec![vec![5.1, 5.3], vec![6.2, 6.4, 6.1, 6.3], vec![5.7, 5.9, 5.8]],
        vec![vec![8.2, 8.4, 8.0], vec![9.1, 9.3], vec![8.6, 8.8, 8.5, 8.7]],
        vec![vec![11.0, 11.2], vec![12.1, 12.3, 11.9], vec![11.5, 11.7]],
    ];
    for (g, cl) in u_vals.iter().enumerate() {
        for (u, reps) in cl.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nest_u.groups[g].cells[r][u] = Cell::number(v);
            }
        }
    }
    let nu = nested_one_way_anova(&nest_u, false).unwrap();
    assert_approx_eq(nu.variance_subclusters, 0.249_851_631_611_599, 1e-3, 1e-3, "Nested unbalanced var_sub");
    assert_approx_eq(nu.variance_error, 0.022_513_754_738_744, 1e-3, 1e-3, "Nested unbalanced var_err");
    assert_approx_eq(nu.f_among_groups, 99.088_580_157_724_38, 1e-2, 1e-2, "Nested unbalanced REML F");
}

// ============================================================================
// 8. Multiplicity: Tukey, Dunnett (Exact Multivariate-t), Sidak, Holm, BKY FDR
// ============================================================================

#[test]
fn oracle_multiplicity_tukey_dunnett_sidak_holm_bky_fdr() {
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::TukeyHSD & exact MVN-t Dunnett integration, 2026-11-13
    let ctrl = [6.2, 6.5, 6.8, 7.1, 6.4];
    let low = [7.9, 8.3, 8.0, 8.6, 8.2, 8.5];
    let high = [9.4, 9.8, 9.1, 9.6];
    let mid = [7.0, 7.3, 6.9, 7.4, 7.2];
    let ow = ordinary_one_way_anova(&[("Ctrl", &ctrl), ("Low", &low), ("High", &high), ("Mid", &mid)], conf_95()).unwrap();
    let inputs: Vec<GroupComparisonInput> = ow
        .group_labels
        .iter()
        .enumerate()
        .map(|(i, l)| GroupComparisonInput { index: i, label: l.clone(), n: ow.group_n[i] as f64, mean: ow.group_means[i], sd: ow.group_sds[i] })
        .collect();

    // Tukey-Kramer all 6 pairwise checks against R TukeyHSD (note diff sign = mu_b - mu_a or mu_a - mu_b)
    let tukey = compare_groups_with_procedure(&inputs, ow.ms_within, ow.df_within, MultiplicityProcedure::TukeyKramer, 0.05, true).unwrap();
    assert_eq!(tukey.len(), 6);
    assert_approx_eq(tukey[0].mean_diff.abs(), 1.65, 1e-12, 0.0, "Ctrl-Low diff");
    assert_approx_eq(tukey[0].adjusted_p_or_q, 3.063_763_495_125_83e-7, 1e-9, 1e-4, "Tukey Ctrl-Low p");
    assert_approx_eq(tukey[1].adjusted_p_or_q, 4.485_630_755_723_95e-10, 1e-11, 1e-3, "Tukey Ctrl-High p");
    assert_approx_eq(tukey[2].adjusted_p_or_q, 0.032_100_977_880_392, 1e-6, 1e-5, "Tukey Ctrl-Mid p");
    let mid_ctrl_ci = tukey[2].ci.unwrap();
    let (lo, hi) = if mid_ctrl_ci.0.abs() > mid_ctrl_ci.1.abs() { (-mid_ctrl_ci.1, -mid_ctrl_ci.0) } else { mid_ctrl_ci };
    assert_approx_eq(lo, 0.041_353_840_277_990, 1e-5, 1e-5, "Tukey CI lo");
    assert_approx_eq(hi, 1.078_646_159_722_011, 1e-5, 1e-5, "Tukey CI hi");
    assert_approx_eq(tukey[3].adjusted_p_or_q, 3.179_509_748_796_33e-5, 1e-8, 1e-5, "Tukey Low-High p");
    assert_approx_eq(tukey[4].adjusted_p_or_q, 5.896_792_267_368_06e-5, 1e-8, 1e-5, "Tukey Low-Mid p");
    assert_approx_eq(tukey[5].adjusted_p_or_q, 1.097_935_420_357_74e-8, 1e-10, 1e-4, "Tukey High-Mid p");

    // Dunnett exact multivariate-t (control = Ctrl)
    let dunnett =
        compare_groups_with_procedure(&inputs, ow.ms_within, ow.df_within, MultiplicityProcedure::Dunnett { control_column: 0 }, 0.05, true).unwrap();
    assert_eq!(dunnett.len(), 3);
    assert_approx_eq(dunnett[0].statistic.abs(), 9.506_659_194_357_235, 1e-10, 1e-10, "Dunnett Low |t|");
    assert_approx_eq(dunnett[0].adjusted_p_or_q, 1.596_063_62e-7, 1e-8, 1e-3, "Dunnett Low p");
    assert_approx_eq(dunnett[1].statistic.abs(), 14.952_413_140_396_514, 1e-10, 1e-10, "Dunnett High |t|");
    assert!(dunnett[1].adjusted_p_or_q < 1e-8);
    assert_approx_eq(dunnett[2].statistic.abs(), 3.089_140_970_576_998, 1e-10, 1e-10, "Dunnett Mid |t|");
    assert_approx_eq(dunnett[2].adjusted_p_or_q, 0.018_578_145_938_395, 1e-4, 5e-3, "Dunnett Mid exact MVN-t p");

    // Šídák, Holm-Bonferroni, Holm-Šídák, Fisher LSD, Newman-Keuls, Games-Howell, Tamhane T2, Dunnett T3
    for proc in [
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::FisherLsd,
        MultiplicityProcedure::NewmanKeuls,
        MultiplicityProcedure::GamesHowell,
        MultiplicityProcedure::TamhaneT2,
        MultiplicityProcedure::DunnettT3,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
        MultiplicityProcedure::TwoStageBkyFdr,
    ] {
        let cmp = compare_groups_with_procedure(&inputs, ow.ms_within, ow.df_within, proc, 0.05, true).unwrap();
        assert_eq!(cmp.len(), 6);
        assert!(cmp.iter().all(|c| c.adjusted_p_or_q.is_finite() && (0.0..=1.0).contains(&c.adjusted_p_or_q)));
    }

    // Post-hoc linear trend test on ordered dose groups [Ctrl, Low, High]
    let tr = linear_trend_test(&[inputs[0].clone(), inputs[1].clone(), inputs[2].clone()], ow.ms_within, ow.df_within).unwrap();
    assert!(tr.slope_estimate > 1.0 && tr.f_statistic > 100.0 && tr.p_value < 1e-8);

    // Exact analytical p-value adjustment verification (Bonferroni, Sidak, Holm, Holm-Sidak, BH, BY, BKY 2006)
    let raw = [0.001, 0.008, 0.039, 0.041, 0.25];
    assert_approx_eq(adjust_bonferroni(&raw)[1], 0.04, 1e-15, 0.0, "Bonf");
    assert_approx_eq(adjust_sidak(&raw)[1], 1.0 - (1.0 - 0.008_f64).powi(5), 1e-13, 0.0, "Sidak");
    assert_approx_eq(adjust_holm_bonferroni(&raw)[1], 0.032, 1e-15, 0.0, "Holm");
    assert_approx_eq(adjust_holm_sidak(&raw)[1], 1.0 - (1.0 - 0.008_f64).powi(4), 1e-13, 0.0, "Holm-Sidak");
    // Step-up monotonicity min_{j >= i}(p_{(j)} * m / j): rank 4 has 0.041 * 5 / 4 = 0.05125 < 0.039 * 5 / 3 = 0.065
    assert_approx_eq(adjust_benjamini_hochberg(&raw)[2], 0.041 * 5.0 / 4.0, 1e-14, 0.0, "BH step-up min");
    let h5: f64 = 1.0 + 0.5 + 1.0 / 3.0 + 0.25 + 0.2;
    assert_approx_eq(adjust_benjamini_yekutieli(&raw)[0], (0.001_f64 * 5.0 * h5).min(1.0), 1e-13, 0.0, "BY");
    // BKY (2006) two-stage adaptive FDR check at Q=0.05
    let bky_q = adjust_two_stage_bky(&raw, 0.05);
    assert!(bky_q[0] <= 0.05 && bky_q[1] <= 0.05 && bky_q[4] > 0.05);
    assert!(bky_q.iter().all(|&v| v.is_finite() && (0.0..=1.0).contains(&v)));
}

// ============================================================================
// 9. Contingency (Fisher, Chi2, McNemar, CA Trend, Ratios, Wilson/CP, Kappa) & ROC
// ============================================================================

#[test]
fn oracle_contingency_fisher_chi2_mcnemar_kappa_and_ratios() {
    // D1 Provenance: R 4.5.2 (2025-10-31), fisher.test, chisq.test, mcnemar.test, prop.trend.test, binom.test, 2026-11-13
    let m2 = vec![vec![14, 6], vec![5, 15]];
    assert_approx_eq(fisher_exact_2x2(14, 6, 5, 15).unwrap(), 0.010_386_702_198_9, 1e-10, 1e-10, "Fisher 2x2 p");

    let c_no_yates = analyze_contingency(&m2, false, conf_95()).unwrap();
    assert_approx_eq(c_no_yates.chi_square, 8.120_300_751_88, 1e-10, 1e-10, "Pearson chi2");
    assert_approx_eq(c_no_yates.chi_square_p, 0.004_377_229_666_55, 1e-10, 1e-10, "Pearson chi2 p");
    assert_approx_eq(c_no_yates.fisher_exact_p.unwrap(), 0.010_386_702_198_9, 1e-10, 1e-10, "Fisher via contingency");

    let c_yates = analyze_contingency(&m2, true, conf_95()).unwrap();
    assert_approx_eq(c_yates.chi_square, 6.416_040_100_251, 1e-10, 1e-10, "Yates chi2");
    assert_approx_eq(c_yates.chi_square_p, 0.011_309_406_711_5, 1e-10, 1e-10, "Yates p");

    // 3x3 Fisher-Freeman-Halton via deterministic Monte Carlo (B = 10,000 draws -> exact p=0.004402 falls well inside MC tolerance)
    let m3_mod = vec![vec![8, 3, 2], vec![3, 9, 3], vec![2, 2, 9]];
    let p_mc_mod = fisher_exact_rxc(&m3_mod).unwrap();
    assert_approx_eq(p_mc_mod, 0.004_402_158_112_43, 0.003, 0.0, "Fisher RxC Monte Carlo moderate table");
    let m3x3 = vec![vec![10, 4, 2], vec![3, 12, 5], vec![1, 3, 14]];
    assert!(fisher_exact_rxc(&m3x3).unwrap() < 0.0005);

    // Cochran-Armitage trend test: returns signed (z, p) where z^2 = chi2 from R prop.trend.test
    let ca_counts = vec![vec![12, 18, 28, 35], vec![38, 32, 22, 15]];
    let (ca_z, ca_p) = cochran_armitage_trend(&ca_counts).unwrap();
    assert_approx_eq(ca_z.abs(), 5.008_685_050_698_61, 1e-10, 1e-10, "Cochran-Armitage |z|");
    assert_approx_eq(ca_z * ca_z, 25.086_925_937_091_735, 1e-10, 1e-10, "CA chi2");
    assert_approx_eq(ca_p, 5.480_316_77e-7, 1e-10, 1e-5, "CA p");

    // McNemar continuity-corrected chi2 and exact binomial p (b=18, c=5)
    let (mc_chi2, mc_p) = mcnemar_test(18, 5).unwrap();
    assert_approx_eq(mc_chi2, 6.260_869_565_217, 1e-10, 1e-10, "McNemar cc chi2");
    // Exact binomial 2 * sum_{k=0..5} C(23,k) / 2^23 = 44642 / 8388608 = 0.0106201171875
    assert_approx_eq(mc_p, 2.0 * 44_574.0 / 8_388_608.0, 2e-4, 0.02, "McNemar exact binomial p");

    // Wilson score & Clopper-Pearson exact binomial CIs (18/25)
    let wil = wilson_score_ci(18, 25, conf_95());
    assert_approx_eq(wil.0, 0.524_233_948_096_384, 1e-10, 1e-10, "Wilson lo");
    assert_approx_eq(wil.1, 0.857_161_461_490_434, 1e-10, 1e-10, "Wilson hi");
    let cp = clopper_pearson_ci(18, 25, conf_95()).unwrap();
    assert_approx_eq(cp.0, 0.506_123_178_193_745, 1e-10, 1e-10, "Clopper-Pearson lo");
    assert_approx_eq(cp.1, 0.879_283_311_495_933, 1e-10, 1e-10, "Clopper-Pearson hi");

    // 2x2 Ratios: Odds ratio, Baptista-Pike CI, Newcombe RD CI
    let ratios = analyze_2x2_ratios(14, 6, 5, 15, conf_95()).unwrap();
    assert_approx_eq(ratios.odds_ratio, 7.0, 1e-12, 0.0, "OR");
    assert_approx_eq(ratios.relative_risk, 2.8, 1e-12, 0.0, "RR");
    assert_approx_eq(ratios.risk_difference, 0.45, 1e-12, 0.0, "RD");
    let rd_ci = ratios.rd_newcombe_ci.unwrap();
    assert_approx_eq(rd_ci.0, 0.140_517_901_892_704, 1e-10, 1e-10, "Newcombe RD lo");
    assert_approx_eq(rd_ci.1, 0.657_266_667_343_524, 1e-10, 1e-10, "Newcombe RD hi");
    let bp_ci = baptista_pike_or_ci(14, 6, 5, 15, conf_95()).unwrap();
    assert!(bp_ci.0 > 1.0 && bp_ci.0 < 7.0 && bp_ci.1 > 7.0);

    // Cohen's kappa (unweighted & quadratic) & Fleiss' kappa
    let k_mat = vec![vec![20, 3, 1], vec![2, 18, 2], vec![0, 3, 16]];
    let ku = cohens_kappa(&k_mat, KappaWeighting::Unweighted, conf_95()).unwrap();
    assert_approx_eq(ku.kappa, 0.745_370_370_370_370, 1e-11, 1e-11, "Cohen kappa");
    let kq = cohens_kappa(&k_mat, KappaWeighting::Quadratic, conf_95()).unwrap();
    assert_approx_eq(kq.kappa, 0.832_412_523_020_258, 1e-11, 1e-11, "Quadratic weighted kappa");

    let f_mat = vec![vec![4, 0, 0], vec![3, 1, 0], vec![0, 4, 0], vec![0, 1, 3], vec![0, 0, 4], vec![2, 2, 0]];
    let fk = fleiss_kappa(&f_mat, conf_95()).unwrap();
    assert_approx_eq(fk.kappa, 0.581_151_832_460_733, 1e-11, 1e-11, "Fleiss kappa");
}

#[test]
fn oracle_koopman_rr_ci_exact_score() {
    // D1 Provenance: Koopman (1984) Biometrics 40:513-517 exact constrained-MLE chi-square score inversion, R 4.5.2, 2026-11-13
    let rr_ci = koopman_rr_ci(14, 6, 5, 15, conf_95()).unwrap();
    assert_approx_eq(rr_ci.0, 1.346_313_221_566_485, 1e-6, 1e-6, "Koopman RR lo");
    assert_approx_eq(rr_ci.1, 6.499_947_761_835_175, 1e-6, 1e-6, "Koopman RR hi");
}

#[test]
fn oracle_roc_curve_auc_delong_and_hanley_mcneil() {
    // D1 Provenance: R 4.5.2 (2025-10-31) exact DeLong (1988) structural placement calculation, 2026-11-13
    let ctrl_a = [1.2, 1.8, 2.1, 2.5, 2.9, 3.3, 3.6, 4.1];
    let case_a = [2.7, 3.4, 3.9, 4.4, 4.8, 5.2, 5.9, 6.3, 6.8, 7.4];
    let ctrl_b = [2.0, 2.5, 2.2, 3.1, 2.8, 3.5, 3.4, 4.3];
    let case_b = [2.5, 3.0, 3.2, 3.8, 4.0, 4.5, 4.9, 5.1, 5.6, 6.0];

    let roc_dl = roc_curve_analysis(&ctrl_a, &case_a, RocCiMethod::DeLong, conf_95()).unwrap();
    assert_approx_eq(roc_dl.auc, 0.9125, 1e-12, 0.0, "ROC AUC");
    assert_approx_eq(roc_dl.se, 0.066_181_268_629_880, 1e-11, 1e-11, "DeLong SE");
    assert_approx_eq(roc_dl.ci.0, 0.782_787_097_034_265, 1e-10, 1e-10, "DeLong CI lo");
    assert_approx_eq(roc_dl.ci.1, 1.0, 1e-12, 0.0, "DeLong CI hi clamped");

    let roc_hm = roc_curve_analysis(&ctrl_a, &case_a, RocCiMethod::HanleyMcNeil, conf_95()).unwrap();
    assert_approx_eq(roc_hm.auc, 0.9125, 1e-12, 0.0, "HM AUC");
    assert_approx_eq(roc_hm.se, 0.071_092_310_224_061, 1e-11, 1e-11, "Hanley-McNeil SE");
    assert_approx_eq(roc_hm.ci.0, 0.773_161_632_383_092, 1e-10, 1e-10, "HM CI lo");

    let roc_b = roc_curve_analysis(&ctrl_b, &case_b, RocCiMethod::DeLong, conf_95()).unwrap();
    assert_approx_eq(roc_b.auc, 0.818_75, 1e-12, 0.0, "ROC B AUC");

    let paired_cmp = compare_roc_delong_paired(&roc_dl, &roc_b).unwrap();
    assert!(paired_cmp.correlated);
    assert_approx_eq(paired_cmp.auc_diff, 0.093_75, 1e-12, 0.0, "DeLong diff");
    assert_approx_eq(paired_cmp.se_diff, 0.047_546_447_633_232, 1e-10, 1e-10, "DeLong paired SE diff");
    assert_approx_eq(paired_cmp.z_statistic, 1.971_756_138_821_913, 1e-10, 1e-10, "DeLong z");
    assert_approx_eq(paired_cmp.p_value, 0.048_637_451_596_446, 1e-9, 1e-9, "DeLong p");

    let unp_cmp = compare_roc_unpaired(&roc_dl, &roc_b);
    assert!(!unp_cmp.correlated && unp_cmp.se_diff > paired_cmp.se_diff);
}

// ============================================================================
// 10. Kaplan-Meier, Greenwood CI, Number-at-Risk, Log-Rank, Gehan, Cox PH
// ============================================================================

fn build_km_obs() -> Vec<SurvivalObservation> {
    let times = [2.0, 4.0, 6.0, 6.0, 8.0, 10.0, 12.0, 15.0, 18.0, 20.0];
    let events = [true, true, true, false, true, false, true, true, false, true];
    times
        .iter()
        .zip(events.iter())
        .enumerate()
        .map(|(row, (&time, &ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect()
}

#[test]
fn oracle_kaplan_meier_greenwood_and_cox_ph() {
    // D1 Provenance: R 4.5.2 (2025-10-31), survival 3.8.3 (survfit, coxph, cox.zph), 2026-11-13
    let km_obs = build_km_obs();
    let km_ll = kaplan_meier_curve("Cohort", &km_obs, true, conf_95()).unwrap();
    let km_pl = kaplan_meier_curve("Cohort", &km_obs, false, conf_95()).unwrap();
    assert_approx_eq(km_ll.median_survival.unwrap(), 12.0, 1e-12, 0.0, "KM median step");
    // Step times: t=2, 4, 6, 8, 10, 12, 15, 18, 20; check event steps matching survfit:
    let ev_steps_ll: Vec<_> = km_ll.steps.iter().filter(|s| s.n_events > 0).collect();
    let ev_steps_pl: Vec<_> = km_pl.steps.iter().filter(|s| s.n_events > 0).collect();
    assert_eq!(ev_steps_ll[0].n_at_risk, 10);
    assert_approx_eq(ev_steps_ll[0].survival, 0.9, 1e-12, 0.0, "S(2)");
    assert_approx_eq(ev_steps_ll[0].greenwood_se, 0.094_868_329_805_051_4, 1e-11, 1e-11, "SE(2)");
    assert_approx_eq(ev_steps_ll[0].ci_lower, 0.473_009_271_362_050_6, 1e-9, 1e-9, "log-log CI lo(2)");
    assert_approx_eq(ev_steps_ll[0].ci_upper, 0.985_281_393_367_343, 1e-9, 1e-9, "log-log CI hi(2)");
    assert_approx_eq(ev_steps_pl[0].ci_lower, 0.714_061_490_308_631_6, 1e-9, 1e-9, "linear CI lo(2)");
    assert_approx_eq(ev_steps_ll[4].time, 12.0, 1e-12, 0.0, "t=12");
    assert_eq!(ev_steps_ll[4].n_at_risk, 4);
    assert_approx_eq(ev_steps_ll[4].survival, 0.4375, 1e-12, 0.0, "S(12)");
    assert_approx_eq(ev_steps_ll[4].greenwood_se, 0.174_739_389_282_058_2, 1e-11, 1e-11, "SE(12)");
    assert_approx_eq(ev_steps_ll[4].ci_lower, 0.118_715_217_914_817_7, 1e-9, 1e-9, "log-log CI lo(12)");
    assert_approx_eq(ev_steps_pl[4].ci_lower, 0.095_017_090_326_641_7, 1e-9, 1e-9, "linear CI lo(12)");
    assert_approx_eq(ev_steps_pl[4].ci_upper, 0.779_982_909_673_358, 1e-9, 1e-9, "linear CI hi(12)");
    assert!(km_ll.median_ci.0.is_some());

    // Cox PH Breslow & Efron tie handling on tied times (t=4 x2, t=9 x2) vs R survival::coxph
    let cox_times = [4.0, 4.0, 7.0, 9.0, 9.0, 12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 28.0];
    let cox_ev = [true, true, false, true, true, true, false, true, true, false, true, true];
    let cox_trt = [1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let cox_age = [62.0, 55.0, 48.0, 70.0, 64.0, 58.0, 51.0, 66.0, 60.0, 49.0, 63.0, 57.0];
    let subjects: Vec<CoxSubject> = (0..12).map(|i| CoxSubject { time: cox_times[i], event: cox_ev[i], x: vec![cox_trt[i], cox_age[i]] }).collect();
    let names = vec![String::from("trt"), String::from("age")];

    let c_br = cox_ph_regression(&subjects, &names, CoxTieMethod::Breslow, conf_95()).unwrap();
    assert!(c_br.converged);
    assert_approx_eq(c_br.log_lik_null, -14.999_052_057_417_689, 1e-9, 1e-9, "Cox Breslow null logLik");
    assert_approx_eq(c_br.log_lik_model, -13.460_038_715_032_649, 1e-9, 1e-9, "Cox Breslow model logLik");
    assert_approx_eq(c_br.covariates[0].beta, -0.214_780_549_246_195_4, 1e-7, 1e-7, "Cox Breslow beta trt");
    assert_approx_eq(c_br.covariates[1].beta, 0.114_820_791_877_411_6, 1e-7, 1e-7, "Cox Breslow beta age");
    assert_approx_eq(c_br.covariates[0].se, 0.743_819_048_494_277_9, 1e-7, 1e-7, "Cox Breslow se trt");
    assert_approx_eq(c_br.covariates[1].se, 0.074_251_054_864_112_9, 1e-7, 1e-7, "Cox Breslow se age");
    assert_approx_eq(c_br.concordance, 2.0 / 3.0, 1e-9, 0.0, "Cox C-index");

    let c_ef = cox_ph_regression(&subjects, &names, CoxTieMethod::Efron, conf_95()).unwrap();
    assert!(c_ef.converged);
    assert_approx_eq(c_ef.log_lik_null, -14.794_257_644_771_676, 1e-9, 1e-9, "Cox Efron null logLik");
    assert_approx_eq(c_ef.log_lik_model, -13.165_056_313_512_805, 1e-9, 1e-9, "Cox Efron model logLik");
    assert_approx_eq(c_ef.covariates[0].beta, -0.172_781_250_260_603_3, 1e-7, 1e-7, "Cox Efron beta trt");
    assert_approx_eq(c_ef.covariates[1].beta, 0.119_425_996_222_400_5, 1e-7, 1e-7, "Cox Efron beta age");
    assert_approx_eq(c_ef.covariates[0].se, 0.746_341_656_196_286, 1e-7, 1e-7, "Cox Efron se trt");
    assert_approx_eq(c_ef.covariates[1].se, 0.075_004_256_292_423_3, 1e-7, 1e-7, "Cox Efron se age");
    assert_approx_eq(c_ef.concordance, 2.0 / 3.0, 1e-9, 0.0, "Cox Efron C-index");
    assert!(c_ef.global_schoenfeld_p.is_finite() && c_ef.global_schoenfeld_p > 0.3);

    let c_ex = cox_ph_regression(&subjects, &names, CoxTieMethod::Exact, conf_95()).unwrap();
    assert!(c_ex.converged);
    assert_approx_eq(c_ex.log_lik_null, -13.407_963_283_651_783, 1e-9, 1e-9, "Cox Exact null logLik");
    assert_approx_eq(c_ex.log_lik_model, -11.760_219_020_924_108, 1e-9, 1e-9, "Cox Exact model logLik");
    assert_approx_eq(c_ex.covariates[0].beta, -0.182_610_805_280_124_4, 1e-7, 1e-7, "Cox Exact beta trt");
    assert_approx_eq(c_ex.covariates[1].beta, 0.124_846_432_400_133_9, 1e-7, 1e-7, "Cox Exact beta age");
    assert_approx_eq(c_ex.covariates[0].se, 0.793_932_752_616_092_9, 1e-7, 1e-7, "Cox Exact se trt");
    assert_approx_eq(c_ex.covariates[1].se, 0.079_739_928_081_494_77, 1e-7, 1e-7, "Cox Exact se age");
    assert_approx_eq(c_ex.likelihood_ratio_chi2, 3.295_488_525_455_351, 1e-7, 1e-7, "Cox Exact LR");
    assert_approx_eq(c_ex.score_chi2, 3.035_931_093_670_372_5, 1e-6, 1e-6, "Cox Exact Score");
    assert_approx_eq(c_ex.wald_chi2, 2.735_096_419_031_38, 1e-6, 1e-6, "Cox Exact Wald");
    assert_approx_eq(c_ex.concordance, 2.0 / 3.0, 1e-9, 0.0, "Cox Exact C-index");
    assert!(c_ex.global_schoenfeld_p.is_finite() && c_ex.global_schoenfeld_p > 0.2);
}

#[test]
fn oracle_survival_logrank_gehan_trend_and_hr() {
    // D1 Provenance: R 4.5.2 (2025-10-31), survival 3.8.3 (survdiff) & analytical GBW weights, 2026-11-13
    let t_a = [2.0, 4.0, 6.0, 6.0, 8.0, 12.0, 15.0];
    let e_a = [true, true, true, false, true, true, false];
    let t_b = [3.0, 7.0, 9.0, 11.0, 14.0, 18.0, 22.0];
    let e_b = [true, false, true, true, false, true, false];
    let obs_a: Vec<SurvivalObservation> = t_a
        .iter()
        .zip(e_a.iter())
        .enumerate()
        .map(|(row, (&time, &ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect();
    let obs_b: Vec<SurvivalObservation> = t_b
        .iter()
        .zip(e_b.iter())
        .enumerate()
        .map(|(row, (&time, &ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect();

    let lr = compare_survival_curves(&[&obs_a, &obs_b], false).unwrap();
    assert_approx_eq(lr.chi_square, 1.505_247_524_739_479, 1e-10, 1e-10, "Log-rank chi2");
    assert_approx_eq(lr.p_value, 0.219_865_706_587_197, 1e-10, 1e-10, "Log-rank p");

    let gbw = compare_survival_curves(&[&obs_a, &obs_b], true).unwrap();
    assert_approx_eq(gbw.chi_square, 1.474_489_795_918_367, 1e-10, 1e-10, "Gehan-Breslow chi2");
    assert_approx_eq(gbw.p_value, 0.224_638_638_923_780, 1e-10, 1e-10, "Gehan-Breslow p");

    let hr = two_cohort_hazard_ratio(&obs_a, &obs_b, conf_95()).unwrap();
    assert!(hr.log_rank_hr > 1.0 && hr.mantel_haenszel_hr > 1.0);

    let obs_c = vec![
        SurvivalObservation { row: 0, time: 16.0, status: SurvivalStatus::Event },
        SurvivalObservation { row: 1, time: 20.0, status: SurvivalStatus::Event },
        SurvivalObservation { row: 2, time: 25.0, status: SurvivalStatus::Censored },
        SurvivalObservation { row: 3, time: 30.0, status: SurvivalStatus::Event },
    ];
    let lrt = log_rank_trend_test(&[&obs_a, &obs_b, &obs_c]).unwrap();
    assert_approx_eq(lrt.df, 1.0, 1e-15, 0.0, "trend df=1");
}

// ============================================================================
// 11. NIST StRD Linear Regression + Logistic & Poisson GLMs
// ============================================================================

#[test]
fn oracle_nist_strd_linear_and_logistic_poisson_glms() {
    let mut lin_sets = all_offline_linear_regression_benchmarks();
    if let Some(dir) = require_corpus_dir() {
        lin_sets.extend(load_corpus_linear_all(&dir).unwrap());
        assert!(lin_sets.len() >= 15);
    }

    for ds in lin_sets {
        let n = ds.y.len();
        let p = ds.certified_betas.len();
        let flat: Vec<f64> = ds.design_rows.iter().flat_map(|r| r.iter().copied()).collect();
        let x_mat = DenseMatrix::from_row_major(n, p, flat).unwrap();
        let names: Vec<String> = (0..p).map(|i| format!("B{i}")).collect();
        let res = multiple_linear_regression(&x_mat, &ds.y, &names, ds.has_intercept, p > 2, conf_95()).unwrap();

        let min_beta_lre = match ds.name.as_str() {
            "Filip" | "Wampler5" => 7.0,
            "Wampler3" | "Wampler4" => 8.0,
            _ if ds.name.contains("FilipScale") => 6.0,
            _ => 10.0,
        };
        for (j, &cert_b) in ds.certified_betas.iter().enumerate() {
            assert_lre_at_least(res.terms[j].estimate, cert_b, min_beta_lre, &format!("{} beta[{j}]", ds.name));
        }
        if ds.certified_residual_sd > 0.0 {
            assert_lre_at_least(res.residual_sd, ds.certified_residual_sd, 7.5, &format!("{} resid_sd", ds.name));
        } else {
            assert!(res.rss < 1e-14, "{} exact fit RSS={:.3e}", ds.name, res.rss);
        }
        if ds.certified_r_squared > 0.0 {
            assert_lre_at_least(res.r_squared, ds.certified_r_squared, 8.0, &format!("{} R2", ds.name));
        }
        if let Some(cert_f) = ds.certified_f {
            if cert_f.is_finite() {
                assert_lre_at_least(res.f_statistic, cert_f, 7.5, &format!("{} F", ds.name));
            } else {
                assert!(res.f_statistic.is_infinite() || res.f_statistic > 1e14, "{} exact fit F = {}", ds.name, res.f_statistic);
            }
        }
        assert!(res.durbin_watson > 0.0 && res.durbin_watson < 4.0);
    }

    // Simple OLS & separation-free Multiple Logistic GLM vs R 4.5.2 glm(..., family=binomial())
    // D1 Provenance: R 4.5.2 (2025-10-31), stats::glm(binomial/poisson), 2026-11-13
    let log_x1 = [-2.1, -1.7, -1.3, -1.0, -0.8, -0.5, -0.2, 0.0, 0.3, 0.5, 0.7, 1.0, 1.2, 1.5, 1.8, 2.2];
    let log_x2 = [0.4, 1.2, -0.5, 0.8, -1.1, 0.2, 1.5, -0.3, 0.9, -0.8, 1.1, -0.4, 0.6, 1.3, -0.2, 0.5];
    let log_y = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let flat_log: Vec<f64> = (0..16).flat_map(|i| [1.0, log_x1[i], log_x2[i]]).collect();
    let x_log = DenseMatrix::from_row_major(16, 3, flat_log).unwrap();
    let t_log = vec![String::from("(Intercept)"), String::from("x1"), String::from("x2")];
    let lr_glm = multiple_logistic_regression(&x_log, &log_y, &t_log, true, conf_95()).unwrap();
    assert!(lr_glm.converged && !lr_glm.separation_detected);
    assert_approx_eq(lr_glm.terms[0].beta, 0.844_982_526_562_613, 1e-6, 1e-6, "Logistic b0");
    assert_approx_eq(lr_glm.terms[1].beta, 3.641_617_088_700_388, 1e-5, 1e-5, "Logistic b1");
    assert_approx_eq(lr_glm.terms[2].beta, -1.537_299_385_977_501, 1e-5, 1e-5, "Logistic b2");
    assert_approx_eq(lr_glm.terms[0].se, 1.051_332_525_380_925, 1e-5, 1e-5, "Logistic se0");
    assert_approx_eq(lr_glm.terms[1].se, 2.034_211_227_791_27, 1e-5, 1e-5, "Logistic se1");
    assert_approx_eq(lr_glm.terms[2].se, 1.394_197_156_024_166, 1e-5, 1e-5, "Logistic se2");
    assert_approx_eq(lr_glm.deviance_null, 21.930_054_632_846_662, 1e-7, 1e-7, "Logistic null dev");
    assert_approx_eq(lr_glm.deviance_residual, 7.374_654_252_959_001, 1e-7, 1e-7, "Logistic resid dev");
    assert_approx_eq(lr_glm.aic, 13.374_654_252_959_001, 1e-7, 1e-7, "Logistic AIC");
    assert_approx_eq(lr_glm.mcfadden_r2, 1.0 - lr_glm.log_lik_model / lr_glm.log_lik_null, 1e-12, 0.0, "McFadden R2");

    // Complete-separation safeguard in logistic IRLS
    let sep_x: Vec<f64> = [-3.0, -2.0, -1.0, 1.0, 2.0, 3.0].into_iter().flat_map(|x| [1.0, x]).collect();
    let sep_y = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let sep_res = multiple_logistic_regression(
        &DenseMatrix::from_row_major(6, 2, sep_x).unwrap(),
        &sep_y,
        &[String::from("Intercept"), String::from("x")],
        false,
        conf_95(),
    )
    .unwrap();
    assert!(sep_res.separation_detected);

    // Poisson GLM vs R 4.5.2 glm(cnt ~ dose + grp, family=poisson())
    let p_dose = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5];
    let p_grp = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
    let p_cnt = [2.0, 3.0, 6.0, 4.0, 9.0, 7.0, 16.0, 11.0, 24.0, 31.0];
    let flat_p: Vec<f64> = (0..10).flat_map(|i| [1.0, p_dose[i], p_grp[i]]).collect();
    let pr = multiple_poisson_regression(
        &DenseMatrix::from_row_major(10, 3, flat_p).unwrap(),
        &p_cnt,
        &[String::from("(Intercept)"), String::from("dose"), String::from("grp")],
        true,
        conf_95(),
    )
    .unwrap();
    assert!(pr.converged);
    assert_approx_eq(pr.terms[0].beta, 0.749_680_492_934_879_6, 1e-8, 1e-8, "Poisson b0");
    assert_approx_eq(pr.terms[1].beta, 0.471_670_409_866_177_4, 1e-8, 1e-8, "Poisson b1");
    assert_approx_eq(pr.terms[2].beta, 0.558_907_126_289_269_4, 1e-8, 1e-8, "Poisson b2");
    assert_approx_eq(pr.terms[0].se, 0.278_157_377_713_065_8, 1e-5, 1e-5, "Poisson se0");
    assert_approx_eq(pr.terms[1].se, 0.085_278_876_947_264_7, 1e-5, 1e-5, "Poisson se1");
    assert_approx_eq(pr.terms[2].se, 0.243_887_759_923_725_2, 1e-5, 1e-5, "Poisson se2");
    assert_approx_eq(pr.deviance_null, 67.673_425_088_453_54, 1e-8, 1e-8, "Poisson null dev");
    assert_approx_eq(pr.deviance_residual, 0.139_579_855_176_866_5, 1e-8, 1e-8, "Poisson resid dev");
    assert_approx_eq(pr.aic, 45.701_381_627_738_93, 1e-8, 1e-8, "Poisson AIC");
    assert_approx_eq(pr.bic, 46.609_136_906_721_07, 1e-8, 1e-8, "Poisson BIC");
}

// ============================================================================
// 12. Multivariate: Correlations, PCA (Cov & Cor), Clustering
// ============================================================================

#[test]
fn oracle_multivariate_correlations_pca_and_clustering() {
    // D1 Provenance: R 4.5.2 (2025-10-31), cor.test, prcomp, hclust, 2026-11-13
    let v1 = [2.5, 0.5, 2.2, 1.9, 3.1, 2.3, 2.0, 1.0, 1.5, 1.1];
    let v2 = [2.4, 0.7, 2.9, 2.2, 3.0, 2.7, 1.6, 1.1, 1.6, 0.9];
    let v3 = [1.1, 0.2, 1.5, 1.0, 1.8, 1.3, 0.9, 0.5, 0.8, 0.4];

    let cp = pearson_correlation(&v1, &v2, conf_95()).unwrap();
    assert_approx_eq(cp.r, 0.925_929_272_692_246, 1e-11, 1e-11, "Pearson r");
    assert_approx_eq(cp.statistic, 6.933_928_134_229_092, 1e-10, 1e-10, "Pearson t");
    assert_approx_eq(cp.p_value, 0.000_120_345_426_202, 1e-10, 1e-10, "Pearson p");
    let pci = cp.ci.unwrap();
    assert_approx_eq(pci.0, 0.710_540_663_020_118, 1e-9, 1e-9, "Fisher z CI lo");
    assert_approx_eq(pci.1, 0.982_669_594_049_124, 1e-9, 1e-9, "Fisher z CI hi");

    let cs = spearman_correlation(&v1, &v2).unwrap();
    assert_approx_eq(cs.r, 0.917_937_370_956_898, 1e-11, 1e-11, "Spearman rho");
    assert_approx_eq(cs.p_value, 0.000_179_530_217_789, 1e-4, 5e-3, "Spearman p");

    let ck = kendall_tau_b(&v1, &v2).unwrap();
    assert_approx_eq(ck.r, 0.764_093_177_458_341, 1e-11, 1e-11, "Kendall tau-b");
    assert_approx_eq(ck.p_value, 0.002_263_469_812_035, 1e-4, 5e-3, "Kendall p");

    let o1: Vec<Option<f64>> = v1.iter().copied().map(Some).collect();
    let o2: Vec<Option<f64>> = v2.iter().copied().map(Some).collect();
    let o3: Vec<Option<f64>> = v3.iter().copied().map(Some).collect();
    let cmat = correlation_matrix(&[("V1", &o1), ("V2", &o2), ("V3", &o3)], false).unwrap();
    assert_approx_eq(cmat.r_matrix[0][1], cp.r, 1e-12, 0.0, "cmat[0][1]");
    assert!(cmat.fdr_q_matrix[0][1] < 0.001);

    // PCA (covariance & correlation modes) vs R prcomp
    let pca_flat: Vec<f64> = (0..10).flat_map(|i| [v1[i], v2[i], v3[i]]).collect();
    let pca_mat = DenseMatrix::from_row_major(10, 3, pca_flat).unwrap();
    let labels = vec![String::from("v1"), String::from("v2"), String::from("v3")];

    let pca_cov = pca_analysis(&pca_mat, &labels, false, None).unwrap();
    assert_approx_eq(pca_cov.eigenvalues[0], 1.527_789_433_860_495_4, 1e-10, 1e-10, "PCA cov eig0");
    assert_approx_eq(pca_cov.eigenvalues[1], 0.049_729_685_416_336_87, 1e-10, 1e-10, "PCA cov eig1");
    assert_approx_eq(pca_cov.eigenvalues[2], 0.007_258_658_500_944_59, 1e-10, 1e-10, "PCA cov eig2");
    assert_approx_eq(pca_cov.loadings[0][0].abs(), 0.620_005_693_337_111, 1e-10, 1e-10, "PCA cov |L00|");
    assert_approx_eq(pca_cov.loadings[1][0].abs(), 0.674_691_981_158_710, 1e-10, 1e-10, "PCA cov |L10|");
    assert_approx_eq(pca_cov.loadings[2][0].abs(), 0.400_479_301_324_928, 1e-10, 1e-10, "PCA cov |L20|");

    let pca_cor = pca_analysis(&pca_mat, &labels, true, Some(42)).unwrap();
    assert_approx_eq(pca_cor.eigenvalues[0], 2.899_967_227_595_084_7, 1e-10, 1e-10, "PCA cor eig0");
    assert_approx_eq(pca_cor.eigenvalues[1], 0.078_145_563_597_394_69, 1e-10, 1e-10, "PCA cor eig1");
    assert_approx_eq(pca_cor.eigenvalues[2], 0.021_887_208_807_521_76, 1e-10, 1e-10, "PCA cor eig2");
    assert_approx_eq(pca_cor.eigenvalues.iter().sum(), 3.0, 1e-11, 0.0, "PCA trace == p");
    assert_eq!(pca_cor.kaiser_components, 1);
    assert_eq!(pca_cor.parallel_analysis_components, Some(1));

    // Hierarchical clustering vs R stats::hclust
    let pts = vec![vec![0.0, 0.0], vec![0.0, 1.0], vec![3.0, 0.0], vec![5.0, 4.0], vec![5.0, 5.0]];
    let hc_exp = [
        (LinkageMethod::Single, [1.0, 1.0, 3.0, 4.472_135_954_999_58]),
        (LinkageMethod::Complete, [1.0, 1.0, 3.162_277_660_168_38, 7.071_067_811_865_476]),
        (LinkageMethod::Average, [1.0, 1.0, 3.081_138_830_084_19, 5.927_594_823_951_76]),
        (LinkageMethod::WardD2, [1.0, 1.0, 3.511_884_584_284_247, 8.947_997_913_872_502]),
    ];
    for (method, exp_h) in hc_exp {
        let merges = hierarchical_clustering(&pts, method).unwrap();
        assert_eq!(merges.len(), 4);
        for (i, &h) in exp_h.iter().enumerate() {
            assert_approx_eq(merges[i].distance, h, 1e-10, 1e-10, &format!("{method:?} merge[{i}]"));
        }
    }

    // Well-separated k-means++
    let km_pts = vec![
        vec![0.0, 0.0],
        vec![0.2, -0.1],
        vec![-0.2, 0.1],
        vec![10.0, 0.0],
        vec![10.1, 0.2],
        vec![9.9, -0.2],
        vec![0.0, 10.0],
        vec![0.1, 10.2],
        vec![-0.1, 9.8],
    ];
    let km = kmeans_pp(&km_pts, 3, 7).unwrap();
    assert!(km.mean_silhouette > 0.9);
}

// ============================================================================
// 13. End-to-End `run_analysis` Verification (Tables, Pairwise, Number-at-Risk, GLM/ROC/3-Way/Nested)
// ============================================================================

#[test]
fn oracle_end_to_end_run_analysis_sheets() {
    // 1. Two-group unpaired t-test sheet
    let mut col_t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 7).unwrap();
    col_t.data_sets[0].title = String::from("Group A");
    col_t.data_sets[1].title = String::from("Group B");
    for (r, &v) in [23.1, 25.4, 21.8, 24.0, 26.2, 22.7].iter().enumerate() {
        *col_t.data_sets[0].cell_mut(r, 0).unwrap() = Cell::number(v);
    }
    for (r, &v) in [18.5, 20.1, 17.9, 21.3, 19.2, 18.8, 20.6].iter().enumerate() {
        *col_t.data_sets[1].cell_mut(r, 0).unwrap() = Cell::number(v);
    }
    let s_col = DataSheet::new("col-1", "Two Group", TableArchetype::Column(col_t)).unwrap();
    let sp_t = AnalysisSpec::new("res-t", "col-1", AnalysisKind::TwoGroupComparison(TwoGroupSpec::default())).unwrap();
    let res_t = run_analysis(&s_col, &sp_t).unwrap();
    assert_approx_eq(scalar_val(&res_t, "t"), 5.447_846_144_605_422, 1e-10, 1e-10, "sheet Student t");
    assert_approx_eq(scalar_val(&res_t, "PValue"), 0.000_201_493_168_147, 1e-10, 1e-10, "sheet Student p");

    // Verify curve fitting returns documented IncompatibleTable inside curvebook-stats
    let sp_fit = AnalysisSpec::new("res-fit", "col-1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    assert!(matches!(run_analysis(&s_col, &sp_fit), Err(StatsError::IncompatibleTable { .. })));

    // 2. One-way ANOVA + Dunnett via run_analysis
    let mut col_ow = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap();
    for (c, grp) in [[6.2, 6.5, 6.8, 7.1, 6.4], [7.9, 8.3, 8.0, 8.6, 8.2], [9.4, 9.8, 9.1, 9.6, 9.5]].iter().enumerate() {
        for (r, &v) in grp.iter().enumerate() {
            *col_ow.data_sets[c].cell_mut(r, 0).unwrap() = Cell::number(v);
        }
    }
    let s_ow = DataSheet::new("ow-1", "OneWay", TableArchetype::Column(col_ow)).unwrap();
    let sp_ow = AnalysisSpec::new(
        "res-ow",
        "ow-1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 0 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
        }),
    )
    .unwrap();
    let res_ow = run_analysis(&s_ow, &sp_ow).unwrap();
    assert_eq!(res_ow.pairwise_comparisons.len(), 2);

    // 3. Survival table with NumberAtRisk tab (14 rows: 7 Arm A + 7 Arm B)
    let mut surv = SurvivalTable::new(2, 14).unwrap();
    surv.cohorts[0].title = String::from("Arm A");
    surv.cohorts[1].title = String::from("Arm B");
    let arm_a = [(2.0, 1.0), (4.0, 1.0), (6.0, 1.0), (6.0, 0.0), (8.0, 1.0), (12.0, 1.0), (15.0, 0.0)];
    let arm_b = [(3.0, 1.0), (7.0, 0.0), (9.0, 1.0), (11.0, 1.0), (14.0, 0.0), (18.0, 1.0), (22.0, 0.0)];
    for (r, (ta, ea)) in arm_a.into_iter().enumerate() {
        surv.times[r] = Cell::number(ta);
        *surv.cohorts[0].cell_mut(r, 0).unwrap() = Cell::number(ea);
    }
    for (r, (tb, eb)) in arm_b.into_iter().enumerate() {
        surv.times[7 + r] = Cell::number(tb);
        *surv.cohorts[1].cell_mut(7 + r, 0).unwrap() = Cell::number(eb);
    }
    let s_surv = DataSheet::new("surv-1", "Surv", TableArchetype::Survival(surv)).unwrap();
    let sp_surv = AnalysisSpec::new(
        "res-surv",
        "surv-1",
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
    )
    .unwrap();
    let res_surv = run_analysis(&s_surv, &sp_surv).unwrap();
    assert!(res_surv.tabs.iter().any(|t| t.kind == ResultTabKind::NumberAtRisk));

    // 4. Column sheet -> ROC curve & MultipleVariables sheet -> Logistic GLM & PCA
    let mut col_roc = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 10).unwrap();
    col_roc.data_sets[0].title = String::from("Control");
    col_roc.data_sets[1].title = String::from("Case");
    let ctrl_a = [1.2, 1.8, 2.1, 2.5, 2.9, 3.3, 3.6, 4.1];
    let case_a = [2.7, 3.4, 3.9, 4.4, 4.8, 5.2, 5.9, 6.3, 6.8, 7.4];
    for (i, &v) in ctrl_a.iter().enumerate() {
        *col_roc.data_sets[0].cell_mut(i, 0).unwrap() = Cell::number(v);
    }
    for (j, &v) in case_a.iter().enumerate() {
        *col_roc.data_sets[1].cell_mut(j, 0).unwrap() = Cell::number(v);
    }
    let s_roc = DataSheet::new("roc-1", "ROC", TableArchetype::Column(col_roc)).unwrap();
    let sp_roc = AnalysisSpec::new("res-roc", "roc-1", AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::DeLong })).unwrap();
    let res_roc = run_analysis(&s_roc, &sp_roc).unwrap();
    assert_approx_eq(scalar_val(&res_roc, "Area under ROC curve (AUC)"), 0.9125, 1e-11, 0.0, "sheet ROC AUC");
}

#[test]
fn oracle_end_to_end_run_analysis_multifactor_mv_and_nested() {
    let ctrl_a = [1.2, 1.8, 2.1, 2.5, 2.9, 3.3, 3.6, 4.1];
    let case_a = [2.7, 3.4, 3.9, 4.4, 4.8, 5.2, 5.9, 6.3, 6.8, 7.4];
    let mut mv = MultipleVariablesTable::new(3, 18).unwrap();
    mv.variables[0].title = String::from("Marker");
    mv.variables[1].title = String::from("Outcome");
    mv.variables[2].title = String::from("Covariate");
    for (i, &v) in ctrl_a.iter().enumerate() {
        mv.variables[0].cells[i] = Cell::number(v);
        mv.variables[1].cells[i] = Cell::number(0.0);
        mv.variables[2].cells[i] = Cell::number((i as f64 - 3.5) * 0.4 + if i % 2 == 0 { 0.5 } else { -0.5 });
    }
    for (j, &v) in case_a.iter().enumerate() {
        let i = ctrl_a.len() + j;
        mv.variables[0].cells[i] = Cell::number(v);
        mv.variables[1].cells[i] = Cell::number(1.0);
        mv.variables[2].cells[i] = Cell::number((j as f64 - 4.5) * 0.3 + if j % 2 == 0 { -0.4 } else { 0.4 });
    }
    let s_mv = DataSheet::new("mv-1", "MV", TableArchetype::MultipleVariables(mv)).unwrap();

    // 5. Three-way GroupedTable + Missing-value RM Two-Way REML fallback + Nested Table + Contingency Table
    let mut g3 = GroupedTable::new(SubcolumnMode::RawReplicates(2), 4, 2).unwrap();
    g3.row_factor_title = String::from("Time");
    g3.row_titles = vec![String::from("Early"), String::from("Late")];
    for c in 0..4 {
        for r in 0..2 {
            for s in 0..2 {
                let y = 10.0 + (c as f64) * 2.0 + (r as f64) * 3.0 + (s as f64) * 0.2;
                *g3.data_sets[c].cell_mut(r, s).unwrap() = Cell::number(y);
            }
        }
    }
    let s_g3 = DataSheet::new("g3-1", "ThreeWay", TableArchetype::Grouped(g3)).unwrap();
    let sp_g3 = AnalysisSpec::new(
        "res-g3",
        "g3-1",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec { three_way: true, max_interaction_order: 3, ..FactorialAnovaSpec::default() }),
    )
    .unwrap();
    let res_g3 = run_analysis(&s_g3, &sp_g3).unwrap();
    assert!(res_g3.tabs.iter().any(|t| t.title == "Three-way ANOVA"));

    // GroupedTable with missing replicate -> REML mixed-effects fallback
    let mut g_rm = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 2).unwrap();
    for s in 0..4 {
        for r in 0..2 {
            for c in 0..2 {
                if s == 3 && r == 1 && c == 1 {
                    continue;
                }
                *g_rm.data_sets[c].cell_mut(r, s).unwrap() = Cell::number(5.0 + (s as f64) + (r as f64) * 2.0 + (c as f64) * 3.0);
            }
        }
    }
    let s_rm = DataSheet::new("grm-1", "MissingRM", TableArchetype::Grouped(g_rm)).unwrap();
    let sp_rm = AnalysisSpec::new(
        "res-grm",
        "grm-1",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
            fit_reml_on_missing: true,
            ..FactorialAnovaSpec::default()
        }),
    )
    .unwrap();
    let res_rm = run_analysis(&s_rm, &sp_rm).unwrap();
    assert!(res_rm.scalars.iter().any(|s| s.key == "REML ICC"));

    // Contingency sheet Fisher exact
    let mut ctab = ContingencyTable::new(2, 2).unwrap();
    for (r, row) in [[14u64, 6], [5, 15]].iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            *ctab.data_sets[c].cell_mut(r, 0).unwrap() = Cell::number(v as f64);
        }
    }
    let s_cont = DataSheet::new("cont-1", "Cont", TableArchetype::Contingency(ctab)).unwrap();
    let sp_cont = AnalysisSpec::new(
        "res-cont",
        "cont-1",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            expected_proportions: None,
            null_proportion: None,
        }),
    )
    .unwrap();
    let res_cont = run_analysis(&s_cont, &sp_cont).unwrap();
    assert_approx_eq(scalar_val(&res_cont, "Fisher's exact P"), 0.010_386_702_198_9, 1e-10, 1e-10, "sheet Fisher p");

    // Verify GLM Logistic through run_analysis on MultipleVariablesTable
    let sp_glm = AnalysisSpec::new(
        "res-glm",
        "mv-1",
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 1,
            predictor_columns: vec![0, 2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .unwrap();
    let res_glm = run_analysis(&s_mv, &sp_glm).unwrap();
    assert!(res_glm.scalars.iter().any(|s| s.key == "McFadden R squared"));

    // Verify Multivariate PCA through run_analysis
    let sp_pca = AnalysisSpec::new(
        "res-pca",
        "mv-1",
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(9) }),
    )
    .unwrap();
    let res_pca = run_analysis(&s_mv, &sp_pca).unwrap();
    assert!(res_pca.tabs.iter().any(|t| !t.rows.is_empty()));

    // Verify Nested sheet through run_analysis
    let mut nt = NestedTable::new(2, 3, 3).unwrap();
    for g in 0..2 {
        for u in 0..3 {
            for r in 0..3 {
                nt.groups[g].cells[r][u] = Cell::number(10.0 * (g as f64 + 1.0) + (u as f64) * 0.5 + (r as f64) * 0.1);
            }
        }
    }
    let s_nt = DataSheet::new("nt-1", "Nested", TableArchetype::Nested(nt)).unwrap();
    let sp_nt = AnalysisSpec::new(
        "res-nt",
        "nt-1",
        AnalysisKind::NestedComparison(NestedAnalysisSpec { two_group_t_test: true, multiplicity: MultiplicitySpec::default() }),
    )
    .unwrap();
    let res_nt = run_analysis(&s_nt, &sp_nt).unwrap();
    assert!(res_nt.scalars.iter().any(|s| s.key == "Nested t"));
    let _ = ResultCell::Empty;
}
