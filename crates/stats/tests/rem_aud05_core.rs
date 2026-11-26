#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression and D1 reference-oracle tests for the 18 AUD-05 core, two-group, multiplicity,
//! multivariate, and power remediation findings:
//! `AUD-05-08`, `AUD-05-09`, `AUD-05-11`, `AUD-05-12`, `AUD-05-30`, `AUD-05-35`, `AUD-05-36`,
//! `AUD-05-37`, `AUD-05-38`, `AUD-05-39`, `AUD-05-44`, `AUD-05-46`, `AUD-05-47`, `AUD-05-48`,
//! `AUD-05-49`, `AUD-05-50`, `AUD-05-54`, and `AUD-05-60` (Part 3).
//!
//! # References
//! - Barnard, J., & Rubin, D. B. (1999). Small-sample degrees of freedom with multiple imputation.
//!   *Biometrika*, 86(4), 948-955.
//! - Benjamini, Y., Krieger, A. M., & Yekutieli, D. (2006). Adaptive linear step-up procedures that
//!   control the false discovery rate. *Biometrika*, 93(3), 491-507.
//! - Brown, M. B., & Forsythe, A. B. (1974). Robust tests for the equality of variances.
//!   *Journal of the American Statistical Association*, 69(346), 364-367.
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.).
//! - Fisher, R. A. (1915). Frequency distribution of the values of the correlation coefficient in
//!   samples from an indefinitely large population. *Biometrika*, 10(4), 507-521.
//! - Games, P. A., & Howell, J. F. (1976). Pairwise multiple comparison procedures with unequal n's
//!   and/or variances. *Journal of Educational Statistics*, 1(2), 113-125.
//! - Hotelling, H. (1953). New light on the correlation coefficient and its transforms.
//!   *Journal of the Royal Statistical Society, Series B*, 15(2), 193-232.
//! - Johnson, D. B., & Mizoguchi, T. (1978). Selecting the Kth element in X + Y and
//!   X1 + X2 + ... + Xm. *SIAM Journal on Computing*, 7(2), 147-153.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). Springer.
//! - Kendall, M. G. (1945). The treatment of ties in ranking problems. *Biometrika*, 33(3), 239-251.
//! - Murtagh, F., & Legendre, P. (2014). Ward's hierarchical agglomerative clustering method.
//!   *Journal of Classification*, 31(3), 274-295.
//! - Owen, D. B. (1965). The power of Student's t-test. *JASA*, 60(309), 320-333.
//! - Piepho, H.-P. (2004). An algorithm for a letter-based representation of all-pairwise
//!   comparisons. *Journal of Computational and Graphical Statistics*, 13(2), 456-466.
//! - Rubin, D. B. (1987). *Multiple Imputation for Nonresponse in Surveys*. Wiley.
//! - Satterthwaite, F. E. (1946). *Biometrics Bulletin*, 2(6), 110-114.
//! - Welch, B. L. (1947). *Biometrika*, 34(1-2), 28-35.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ClusterDistance, ClusterLinkage, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec,
    MultipleVariablesTable, MultivariateSpec, OutlierSpec, PValueStackMethod, PairwiseComparisonRecord, PowerAnalysisSpec, ResultSheet, SheetId,
    SubcolumnMode, SummarizedPoint, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_isf;
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::StatsError;
use curvebook_stats::cld::{CldGroupInput, CldOptions, MAX_CLD_COLUMNS, compute_cld_from_matrix, compute_cld_from_result_sheet};
use curvebook_stats::column::{run_descriptive_analysis, run_outlier_analysis, summarize_column, summarize_pre_summarized_points};
use curvebook_stats::common::{select_kth_pairwise_sum, select_kth_upper_triangle_half_sum};
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, adjust_two_stage_bky, compare_groups_with_procedure};
use curvebook_stats::multivariate::{
    LinkageMethod, MAX_CLUSTERING_N, cophenetic_distance_matrix, cut_dendrogram, hierarchical_clustering, kendall_tau_b, kmeans_pp,
    kmeans_pp_with_distance, pca_analysis, rubin_pool_estimates, run_multivariate_analysis,
};
use curvebook_stats::power::{
    CorrelationPowerMethod, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, compute_sensitivity_table, run_power_analysis,
    solve_power,
};
use curvebook_stats::pvalue_stack::analyze_pvalue_stack;
use curvebook_stats::two_group::{multiple_row_t_tests, run_two_group_analysis, welch_t_test, welch_t_test_from_summary};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: expected {expected:.16e}, got {actual:.16e} (diff={:.3e}, tol={tol:.3e})",
        (actual - expected).abs()
    );
}

fn make_column_sheet(id: &str, cols: &[(&str, &[f64])]) -> DataSheet {
    let n_rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(1).max(1);
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), n_rows).unwrap();
    for (c_idx, (title, values)) in cols.iter().enumerate() {
        table.data_sets[c_idx].title = (*title).to_owned();
        for (r_idx, &v) in values.iter().enumerate() {
            table.data_sets[c_idx].cells[r_idx][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::Column(table)).unwrap()
}

fn make_mv_sheet(id: &str, headers: &[&str], rows: &[Vec<f64>]) -> DataSheet {
    let mut mv = MultipleVariablesTable::new(headers.len(), rows.len()).unwrap();
    for (c, &h) in headers.iter().enumerate() {
        mv.variables[c].title = h.to_owned();
        for (r, row) in rows.iter().enumerate() {
            mv.variables[c].cells[r] = Cell::number(row[c]);
        }
    }
    DataSheet::new(SheetId::new(id), id, TableArchetype::MultipleVariables(mv)).unwrap()
}

// ============================================================================
// AUD-05-08: BKY two-stage FDR must not floor fdr_q at 1e-6
// ============================================================================

#[test]
fn aud_05_08_bky_fdr_accepts_sub_1e6_q_and_rejects_out_of_range() {
    // Benjamini, Krieger & Yekutieli (2006) Definition 6 exact oracle:
    // m = 10, 9 p-values at 9.5e-8 and 1 at 0.5, target Q = 1e-7.
    // Stage 1: q' = Q / (1 + Q) = 9.9999990000001e-8; BH q-values are 10 * 9.5e-8 / 9 = 1.0555555555555555e-7 > q'.
    // Thus r1 = 0, m0_hat = 10, and adj[0..9] = bh1[i] * (1 + Q) = 1.055555661111111e-7 > 1e-7 (0 discoveries).
    let mut raw_p = vec![9.5e-8; 9];
    raw_p.push(0.5);
    let q = 1e-7;
    let adj = adjust_two_stage_bky(&raw_p, q);
    let expected_first = (10.0 * 9.5e-8 / 9.0) * (1.0 + q);
    for (i, &val) in adj.iter().take(9).enumerate() {
        assert_close(val, expected_first, 1e-18, &format!("BKY q=1e-7 idx={i}"));
        assert!(val > q, "must not fabricate discovery at Q=1e-7 (idx={i}, val={val:e})");
    }

    // Live pvalue_stack compatibility check at Q = 1e-7.
    let stack = analyze_pvalue_stack(&raw_p, PValueStackMethod::TwoStageBky, q).unwrap();
    assert_eq!(stack.discovery_count, 0);

    // When signals genuinely pass Stage 1 at Q = 1e-7, Stage 2 scales by m0_hat = 5.
    let mut strong = vec![1e-9; 5];
    strong.extend([0.5; 5]);
    let strong_adj = adjust_two_stage_bky(&strong, q);
    let expected_strong = 1e-9 * 5.0 * (1.0 + q) / 5.0;
    for (i, &val) in strong_adj.iter().take(5).enumerate() {
        assert_close(val, expected_strong, 1e-20, &format!("BKY strong idx={i}"));
        assert!(val <= q);
    }

    // Out-of-range and non-finite fdr_q must return NaN rather than silently clamping.
    for bad_q in [0.0, -0.05, 1.0, 1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let out = adjust_two_stage_bky(&raw_p, bad_q);
        assert_eq!(out.len(), raw_p.len());
        assert!(out.iter().all(|v| v.is_nan()), "expected all NaN for bad fdr_q={bad_q}");
    }
}

// ============================================================================
// AUD-05-09: Unequal-variance post-hoc n >= 2 guard and overflow-safe Welch df
// ============================================================================

#[test]
fn aud_05_09_unequal_variance_posthoc_rejects_n1_and_scales_welch_df() {
    let n1_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("G1"), n: 1.0, mean: 10.0, sd: 0.0 },
        GroupComparisonInput { index: 1, label: String::from("G2"), n: 1.0, mean: 11.0, sd: 0.0 },
        GroupComparisonInput { index: 2, label: String::from("G3"), n: 5.0, mean: 12.0, sd: 1.0 },
    ];
    for proc_kind in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        let err = compare_groups_with_procedure(&n1_groups, 1.0, 4.0, proc_kind, 0.05, true).unwrap_err();
        assert!(
            matches!(err, StatsError::InsufficientSampleSize { required: 2, actual: 1, .. }),
            "{proc_kind:?} must reject n=1 with InsufficientSampleSize, got {err:?}"
        );
    }

    // Fractional 1.5 < 2.0 also rejected.
    let frac_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("A"), n: 1.5, mean: 1.0, sd: 0.5 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 5.0, mean: 2.0, sd: 0.5 },
    ];
    assert!(matches!(
        compare_groups_with_procedure(&frac_groups, 1.0, 4.0, MultiplicityProcedure::GamesHowell, 0.05, false),
        Err(StatsError::InsufficientSampleSize { required: 2, .. })
    ));

    // Extreme scale sd = 1e100, equal n = 20 -> exact Welch df = 38.0 (not NaN -> 1.0).
    let huge_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("A"), n: 20.0, mean: 0.0, sd: 1e100 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 20.0, mean: 1e100, sd: 1e100 },
    ];
    let expected_huge_se = 1e100 * (2.0_f64 / 20.0).sqrt();
    for proc_kind in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        let res = compare_groups_with_procedure(&huge_groups, 0.0, 1.0, proc_kind, 0.05, true).unwrap();
        assert_eq!(res.len(), 1);
        assert_close(res[0].df, 38.0, 1e-12, &format!("{proc_kind:?} huge df"));
        assert!((res[0].se_diff - expected_huge_se).abs() / expected_huge_se < 1e-12);
    }

    // Extreme tiny scale sd_a = 1e-100, sd_b = 1e-102, n = 20 -> exact Welch df ≈ 19.0038 (not pooled 38.0).
    let tiny_groups = vec![
        GroupComparisonInput { index: 0, label: String::from("A"), n: 20.0, mean: 0.0, sd: 1e-100 },
        GroupComparisonInput { index: 1, label: String::from("B"), n: 20.0, mean: 1e-100, sd: 1e-102 },
    ];
    let r_ratio = 1e-4_f64; // (1e-102 / 1e-100)^2
    let expected_tiny_df = (1.0 + r_ratio).powi(2) / (1.0 / 19.0 + (r_ratio * r_ratio) / 19.0);
    for proc_kind in [MultiplicityProcedure::GamesHowell, MultiplicityProcedure::TamhaneT2, MultiplicityProcedure::DunnettT3] {
        let res = compare_groups_with_procedure(&tiny_groups, 0.0, 1.0, proc_kind, 0.05, true).unwrap();
        assert_close(res[0].df, expected_tiny_df, 1e-10, &format!("{proc_kind:?} tiny df"));
    }
}

// ============================================================================
// AUD-05-11: Contextual minimum integer n in power.rs
// ============================================================================

#[test]
fn aud_05_11_power_contextual_min_integer_n_correlation_and_unpaired_t() {
    // 1. Correlation ConditionalSlopeT at n = 3 (df = 1): must not clamp to n = 4.
    let corr_pow_req = PowerAnalysisRequest::correlation(0.99, 0.05, 0.80)
        .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(3.0);
    let corr_pow = solve_power(&corr_pow_req).unwrap();
    assert_eq!(corr_pow.n_per_group, 3);
    assert_eq!(corr_pow.total_n, 3);
    assert_eq!(corr_pow.actual_df, Some(1.0));
    assert_close(corr_pow.achieved_power, corr_pow.power_at_continuous_n, 1e-12, "Correlation n=3 power");
    assert!(corr_pow.achieved_power < 0.70, "n=3 power for r=0.99 must be ~0.48, not ~0.98 at n=4");
    assert_eq!(corr_pow.power_curve.first().unwrap().n, 3.0);

    // 2. Correlation ExactTTransform SampleSize at r = 0.999 -> n = 3 already exceeds 0.80 power.
    let corr_n_req = PowerAnalysisRequest::correlation(0.999, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
    let corr_n = solve_power(&corr_n_req).unwrap();
    assert_eq!(corr_n.n_per_group, 3);
    assert_eq!(corr_n.total_n, 3);
    assert_eq!(corr_n.actual_df, Some(1.0));
    assert!(corr_n.achieved_power >= 0.80);

    // 3. Correlation ConditionalSlopeT DetectableEffect at n = 3 -> MDE and achieved_power match n = 3.
    let corr_mde_req = PowerAnalysisRequest::new(PowerTestFamily::Correlation)
        .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(3.0)
        .with_alpha(0.05)
        .with_target_power(0.80);
    let corr_mde = solve_power(&corr_mde_req).unwrap();
    assert_eq!(corr_mde.n_per_group, 3);
    assert_eq!(corr_mde.actual_df, Some(1.0));
    assert_close(corr_mde.achieved_power, 0.80, 1e-9, "Correlation MDE n=3 achieved power");

    // 4. FisherZ still requires n >= 4 (n = 3 rejected).
    let fz_bad = PowerAnalysisRequest::correlation(0.99, 0.05, 0.80)
        .with_correlation_method(CorrelationPowerMethod::FisherZ)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(3.0);
    assert!(solve_power(&fz_bad).is_err());

    // 5. Homoscedastic TwoSampleUnpairedT at (n1 = 1, n2 = 10), df = 9: must not clamp to (2, 10).
    let t_pow_req =
        PowerAnalysisRequest::unpaired_t(1.0, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(1.0).with_sample_size_group2(10.0);
    let t_pow = solve_power(&t_pow_req).unwrap();
    assert_eq!(t_pow.n_per_group, 1);
    assert_eq!(t_pow.n_group2, Some(10));
    assert_eq!(t_pow.total_n, 11);
    assert_eq!(t_pow.actual_df, Some(9.0));
    assert_close(t_pow.achieved_power, t_pow.power_at_continuous_n, 1e-12, "UnpairedT (1,10) power");
    assert_eq!(t_pow.power_curve.first().unwrap().n, 1.0);

    // 6. Homoscedastic TwoSampleUnpairedT SampleSize with fixed n2 = 10 and large effect -> n1 = 1 suffices.
    let t_n_req = PowerAnalysisRequest::unpaired_t(6.0, 0.05, 0.80).with_sample_size_group2(10.0);
    let t_n = solve_power(&t_n_req).unwrap();
    assert_eq!(t_n.n_per_group, 1);
    assert_eq!(t_n.n_group2, Some(10));
    assert_eq!(t_n.total_n, 11);
    assert_eq!(t_n.actual_df, Some(9.0));
    assert!(t_n.achieved_power >= 0.80);

    // 7. Welch TwoSampleUnpairedT still rejects n1 = 1.0.
    let welch_bad = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(1.0)
        .with_sample_size_group2(10.0)
        .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 1.0, sd: 1.0, sd_group2: Some(2.0), within_pair_correlation: None });
    assert!(solve_power(&welch_bad).is_err());

    // Keep public const min_integer_n API intact.
    assert_eq!(PowerTestFamily::Correlation.min_integer_n(), 4);
    assert_eq!(PowerTestFamily::TwoSampleUnpairedT.min_integer_n(), 2);
}

// ============================================================================
// AUD-05-12: nn_chain_linkage topological sort & monotonic merge height clamp
// ============================================================================

fn regular_tetrahedron(side: f64) -> Vec<Vec<f64>> {
    let s3 = 3.0_f64.sqrt();
    vec![
        vec![0.0, 0.0, 0.0],
        vec![side, 0.0, 0.0],
        vec![0.5 * side, 0.5 * side * s3, 0.0],
        vec![0.5 * side, side * s3 / 6.0, side * (2.0_f64 / 3.0).sqrt()],
    ]
}

fn assert_valid_topological_dendrogram(n: usize, merges: &[curvebook_stats::multivariate::DendrogramMerge], label: &str) {
    assert_eq!(merges.len(), n - 1, "{label}: expected n-1 merges");
    for (step, m) in merges.iter().enumerate() {
        if m.left >= 0 {
            assert!((m.left as usize) < step, "{label}: forward left ref {} at step {step}", m.left);
        }
        if m.right >= 0 {
            assert!((m.right as usize) < step, "{label}: forward right ref {} at step {step}", m.right);
        }
    }
    for w in merges.windows(2) {
        assert!(w[0].distance <= w[1].distance, "{label}: non-monotonic merge heights {} > {}", w[0].distance, w[1].distance);
    }
}

#[test]
fn aud_05_12_nn_chain_tetrahedron_upgma_and_ward_topological() {
    for (side, legacy_linkage, doc_linkage, label) in [
        (0.7, LinkageMethod::Average, ClusterLinkage::Upgma, "tetrahedron-0.7-UPGMA"),
        (1.7, LinkageMethod::WardD2, ClusterLinkage::Ward, "tetrahedron-1.7-Ward"),
    ] {
        let pts = regular_tetrahedron(side);
        let merges = hierarchical_clustering(&pts, legacy_linkage).unwrap();
        assert_valid_topological_dendrogram(4, &merges, label);

        let coph = cophenetic_distance_matrix(4, &merges).unwrap();
        for i in 0..4 {
            for j in (i + 1)..4 {
                assert_close(coph[i][j], side, 1e-12, &format!("{label} coph[{i},{j}]"));
            }
        }
        for k in 1..=4 {
            let clusters = cut_dendrogram(4, &merges, k).unwrap();
            assert_eq!(clusters.len(), 4);
        }

        // Verify end-to-end live run_multivariate_analysis(Clustering) succeeds cleanly.
        let sheet = make_mv_sheet(label, &["X", "Y", "Z"], &pts);
        let mv_spec =
            MultivariateSpec::Clustering { k_means_k: Some(2), distance: ClusterDistance::Euclidean, linkage: doc_linkage, k_sweep_max: None };
        let spec = AnalysisSpec::new(SheetId::new("res-cl"), sheet.id.clone(), AnalysisKind::Multivariate(mv_spec.clone())).unwrap();
        let res = run_multivariate_analysis(&sheet, &spec, &mv_spec).unwrap();
        assert!(res.tabs.iter().any(|t| t.id == "hierarchical-dendrogram"));
    }
}

// ============================================================================
// AUD-05-30: Piepho CLD complexity cap prevents O(2^{n/2}) hang / OOM
// ============================================================================

#[test]
fn aud_05_30_cld_complexity_cap_rejects_40_group_matching_graph() {
    let n = 40;
    let groups: Vec<CldGroupInput> = (0..n).map(|i| CldGroupInput { index: i, label: format!("G{i}"), mean: (n - i) as f64 }).collect();
    // 20 disjoint significant pairs -> 2^20 = 1,048,576 maximal cliques (> MAX_CLD_COLUMNS = 4,096).
    let mut sig = vec![vec![false; n]; n];
    for p in 0..(n / 2) {
        let a = 2 * p;
        let b = 2 * p + 1;
        sig[a][b] = true;
        sig[b][a] = true;
    }

    let err = compute_cld_from_matrix(&groups, &sig, CldOptions::default()).unwrap_err();
    assert!(
        matches!(err, StatsError::InvalidParameter { name: "cld_complexity_limit", value } if value > MAX_CLD_COLUMNS as f64),
        "expected cld_complexity_limit error, got {err:?}"
    );

    // Also verify propagation through compute_cld_from_result_sheet (live chart path).
    let dummy_sheet = make_column_sheet("cld-src", &[("A", &[1.0, 2.0])]);
    let spec =
        AnalysisSpec::new(SheetId::new("res-cld"), dummy_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let mut rs = ResultSheet::new(spec).unwrap();
    for i in 0..n {
        for j in (i + 1)..n {
            let is_sig = sig[i][j];
            rs.pairwise_comparisons.push(PairwiseComparisonRecord {
                group_a: i,
                group_b: j,
                label_a: format!("G{i}"),
                label_b: format!("G{j}"),
                estimate_diff: Some((j as f64) - (i as f64)),
                raw_p_or_q: if is_sig { 0.001 } else { 0.5 },
                is_fdr_q: false,
                significant: is_sig,
            });
        }
    }
    let sheet_err = compute_cld_from_result_sheet(&rs, None, CldOptions::default()).unwrap_err();
    assert!(matches!(sheet_err, StatsError::InvalidParameter { name: "cld_complexity_limit", .. }));

    // A 40-group all-different graph has only 40 linear columns (<= MAX_CLD_COLUMNS) and succeeds.
    let mut all_diff = vec![vec![true; n]; n];
    for (i, row) in all_diff.iter_mut().enumerate() {
        row[i] = false;
    }
    let ok_res = compute_cld_from_matrix(&groups, &all_diff, CldOptions::default()).unwrap();
    assert_eq!(ok_res.letter_count, n);
}

// ============================================================================
// AUD-05-35: Correlation power asinh(t / sqrt(df)) avoids atanh(1.0) saturation
// ============================================================================

#[test]
fn aud_05_35_correlation_power_asinh_avoids_saturation_and_overflow() {
    // 1. ExactTTransform at n = 3 (df = 1), alpha = 1e-9: t_crit ≈ 6.366e8, where r_crit rounded to 1.0
    // and atanh(r_crit) saturated to +inf (producing power = 0.0). Exact asinh(t_crit / sqrt(1)) = 20.96483.
    let req_small_alpha = PowerAnalysisRequest::correlation(0.999_999_999, 1e-9, 0.80)
        .with_correlation_method(CorrelationPowerMethod::ExactTTransform)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(3.0);
    let res_small_alpha = solve_power(&req_small_alpha).unwrap();
    assert!(
        res_small_alpha.achieved_power.is_finite() && res_small_alpha.achieved_power > 0.0,
        "ExactTTransform n=3 alpha=1e-9 must produce strictly positive finite power, got {}",
        res_small_alpha.achieved_power
    );

    // 2. ExactTTransform SampleSize at alpha = 0.001 (probing n_min = 2.25 where df = 0.25, t_crit ≈ 2.7e11).
    let req_small_df = PowerAnalysisRequest::correlation(0.99, 0.001, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform);
    let res_small_df = solve_power(&req_small_df).unwrap();
    assert!(res_small_df.achieved_power >= 0.80 && res_small_df.achieved_power <= 1.0);
    assert!(res_small_df.n_per_group >= 3);

    // 3. FisherZWithTCrit at tiny df (n = 3.005, alpha = 1e-12): t_crit is huge, must not produce NaN or 0.
    let req_fz_tc = PowerAnalysisRequest::correlation(0.99999, 1e-12, 0.80)
        .with_correlation_method(CorrelationPowerMethod::FisherZWithTCrit)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(3.005);
    let res_fz_tc = solve_power(&req_fz_tc).unwrap();
    assert!(res_fz_tc.power_at_continuous_n.is_finite() && res_fz_tc.power_at_continuous_n > 0.0);
    assert!(res_fz_tc.achieved_power.is_finite() && res_fz_tc.achieved_power > 0.0);
}

// ============================================================================
// AUD-05-36: Descriptive analysis rejects non-finite hypothetical_value
// ============================================================================

#[test]
fn aud_05_36_descriptive_rejects_non_finite_hypothetical_value() {
    let sheet = make_column_sheet("desc-mu0", &[("A", &[1.0, 2.0, 3.0, 4.0, 5.0])]);
    // Construct a valid AnalysisSpec (hypothetical_value: None) and pass mismatched DescriptiveSpec
    // directly to run_descriptive_analysis to test the stats-crate boundary itself.
    let spec = AnalysisSpec::new(SheetId::new("res-desc"), sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    for bad_mu0 in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let bad_desc = DescriptiveSpec { hypothetical_value: Some(bad_mu0), ..DescriptiveSpec::default() };
        let err = run_descriptive_analysis(&sheet, &spec, &bad_desc).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "hypothetical_value", .. }),
            "expected InvalidParameter(hypothetical_value) for mu0={bad_mu0}, got {err:?}"
        );
    }
}

// ============================================================================
// AUD-05-37: Unpaired checklist flags Lognormal and Levene(median) violations
// ============================================================================

#[test]
fn aud_05_37_unpaired_checklist_flags_lognormal_and_levene_violations() {
    // 1. Lognormal unpaired t-test with strongly unequal log-scale variances.
    let g_low_var: Vec<f64> = [-0.02, -0.01, 0.0, 0.01, 0.02, -0.015, 0.015, 0.005].into_iter().map(f64::exp).collect();
    let g_high_var: Vec<f64> = [-2.5, -1.8, -1.0, -0.3, 0.3, 1.0, 1.8, 2.5].into_iter().map(f64::exp).collect();
    let ln_sheet = make_column_sheet("ln-het", &[("Tight", &g_low_var), ("Wide", &g_high_var)]);
    let ln_tg = TwoGroupSpec { method: TwoGroupMethod::LognormalUnpairedT, ..TwoGroupSpec::default() };
    let ln_spec = AnalysisSpec::new(SheetId::new("res-ln"), ln_sheet.id.clone(), AnalysisKind::TwoGroupComparison(ln_tg)).unwrap();
    let ln_res = run_two_group_analysis(&ln_sheet, &ln_spec, &ln_tg).unwrap();
    let ln_var_item = ln_res
        .checklist
        .iter()
        .find(|c| c.assumption == "Homogeneity of variances" || c.assumption == "Variance homogeneity check")
        .expect("variance checklist item");
    assert_eq!(ln_var_item.status, ChecklistStatus::Violated, "lognormal unequal log-variance must be Violated, got {ln_var_item:?}");

    // 2. Unpaired Student t-test where F-test has identical variances (F = 1.0, p = 1.0 > 0.05)
    //    while Brown-Forsythe Levene(median) strongly rejects (p < 0.001 <= 0.05).
    // Group A: 10 at -1, 10 at +1 -> mean=0, median=0, var=20/19, |x - med| = 1.0 everywhere.
    // Group B: 18 at 0, 1 at -sqrt(10), 1 at +sqrt(10) -> mean=0, median=0, var=20/19, mean(|x-med|)=sqrt(10)/10.
    let mut grp_a = vec![-1.0; 10];
    grp_a.extend(std::iter::repeat_n(1.0, 10));
    let mut grp_b = vec![0.0; 18];
    grp_b.push(-10.0_f64.sqrt());
    grp_b.push(10.0_f64.sqrt());

    let lev_sheet = make_column_sheet("lev-vs-f", &[("Bimodal", &grp_a), ("HeavyTail", &grp_b)]);
    let stu_tg = TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() };
    let stu_spec = AnalysisSpec::new(SheetId::new("res-stu"), lev_sheet.id.clone(), AnalysisKind::TwoGroupComparison(stu_tg)).unwrap();
    let stu_res = run_two_group_analysis(&lev_sheet, &stu_spec, &stu_tg).unwrap();
    let stu_var_item = stu_res
        .checklist
        .iter()
        .find(|c| c.assumption == "Homogeneity of variances" || c.assumption == "Variance homogeneity check")
        .expect("variance checklist item");
    assert_eq!(stu_var_item.status, ChecklistStatus::Violated, "Levene(median) rejection must be Violated for Student t, got {stu_var_item:?}");

    // Same dataset under Welch's t-test must emit Info (not Pass!) next to Levene(median) p <= alpha.
    let welch_tg = TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() };
    let welch_spec = AnalysisSpec::new(SheetId::new("res-wel"), lev_sheet.id.clone(), AnalysisKind::TwoGroupComparison(welch_tg)).unwrap();
    let welch_res = run_two_group_analysis(&lev_sheet, &welch_spec, &welch_tg).unwrap();
    let welch_var_item = welch_res.checklist.iter().find(|c| c.assumption == "Variance homogeneity check").expect("Welch variance check");
    assert_eq!(welch_var_item.status, ChecklistStatus::Info, "Welch must emit Info on Levene rejection, got {welch_var_item:?}");
}

// ============================================================================
// AUD-05-38: kmeans_pp enforces MAX_CLUSTERING_N instead of returning 0.0 silhouette
// ============================================================================

#[test]
fn aud_05_38_kmeans_pp_enforces_max_clustering_n() {
    let n_over = MAX_CLUSTERING_N + 1;
    let mut rows = Vec::with_capacity(n_over);
    for i in 0..n_over {
        if i < n_over / 2 {
            rows.push(vec![0.0 + (i as f64) * 1e-4, 0.0]);
        } else {
            rows.push(vec![100.0 + (i as f64) * 1e-4, 100.0]);
        }
    }
    let err1 = kmeans_pp(&rows, 2, 42).unwrap_err();
    assert!(matches!(err1, StatsError::InvalidParameter { name: "clustering_n", value } if value == n_over as f64));

    let err2 = kmeans_pp_with_distance(&rows, 2, ClusterDistance::Euclidean, 42).unwrap_err();
    assert!(matches!(err2, StatsError::InvalidParameter { name: "clustering_n", value } if value == n_over as f64));

    // Valid n <= MAX_CLUSTERING_N computes accurate silhouette > 0.99.
    let small: Vec<Vec<f64>> = (0..10).map(|i| vec![(i as f64) * 0.01, 0.0]).chain((0..10).map(|i| vec![100.0 + (i as f64) * 0.01, 100.0])).collect();
    let ok = kmeans_pp(&small, 2, 42).unwrap();
    assert!(ok.mean_silhouette > 0.99);
}

// ============================================================================
// AUD-05-39 & AUD-05-60 (Part 3): Rubin pooling zero/tiny within variance & lambda
// ============================================================================

#[test]
fn aud_05_39_and_60_rubin_pool_zero_within_variance_and_lambda_identity() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Exact u_bar == 0, b > 0 with complete_data_df = Some(20.0):
    //    q_bar = 2.0, b = 1.0, T = (1 + 1/3) * 1.0 = 4/3, SE = sqrt(4/3),
    //    df = (m - 1).min(20) = 2.0, riv = +inf, lambda = 1.0, fmi = 1.0.
    let res = rubin_pool_estimates("param", &[1.0, 2.0, 3.0], &[0.0, 0.0, 0.0], Some(20.0), conf).unwrap();
    assert!(res.riv.is_infinite() && res.riv.is_sign_positive(), "expected riv=+inf, got {}", res.riv);
    assert_close(res.lambda, 1.0, 1e-15, "lambda at u_bar=0");
    assert_close(res.fmi, 1.0, 1e-15, "fmi at u_bar=0");
    assert_close(res.df, 2.0, 1e-12, "df at u_bar=0");
    let expected_se = (4.0_f64 / 3.0).sqrt();
    let expected_t_crit = student_t_two_sided_isf(0.05, 2.0).unwrap();
    assert_close(res.pooled_se, expected_se, 1e-14, "pooled_se");
    assert_close(res.ci.0, 2.0 - expected_t_crit * expected_se, 1e-12, "ci lower");
    assert_close(res.ci.1, 2.0 + expected_t_crit * expected_se, 1e-12, "ci upper");
    assert!(res.p_value.is_finite() && res.p_value > 0.0 && res.p_value < 1.0);

    // 2. When complete_data_df < m - 1, df is capped at complete_data_df.
    let res_cap = rubin_pool_estimates("param", &[1.0, 2.0, 3.0, 4.0, 5.0], &[0.0; 5], Some(2.5), conf).unwrap();
    assert_close(res_cap.df, 2.5, 1e-12, "df capped at vc");
    assert!(res_cap.ci.0.is_finite() && res_cap.ci.1.is_finite());

    // 3. Negligible within-imputation variance u_bar / b <= 1e-12 must not fail inc_beta or collapse df to 1e-6.
    let res_tiny = rubin_pool_estimates("tiny", &[1.0, 2.0, 3.0], &[1e-14, 1e-14, 1e-14], Some(20.0), conf).unwrap();
    assert!(res_tiny.df >= 1.999_999 && res_tiny.df <= 2.000_001, "expected df≈2.0, got {}", res_tiny.df);
    assert!(res_tiny.ci.0.is_finite() && res_tiny.ci.1.is_finite());
    assert!(res_tiny.riv.is_finite() && res_tiny.riv > 1e12);

    // 4. Intermediate tiny u_bar (1e-8) where Barnard-Rubin df is tiny must not error with NotConverged.
    let res_small_u = rubin_pool_estimates("small_u", &[1.0, 2.0, 3.0], &[1e-8, 1e-8, 1e-8], Some(20.0), conf).unwrap();
    assert!(res_small_u.p_value.is_finite());

    // 5. AUD-05-60 Part 3: verify Rubin's lambda = (1 + 1/m) B / T = r / (1 + r) != FMI.
    let norm = rubin_pool_estimates("Slope", &[1.80, 2.05, 1.95, 2.20, 2.00], &[0.04, 0.05, 0.045, 0.06, 0.055], Some(30.0), conf).unwrap();
    let expected_lambda_from_b = (1.0 + 1.0 / 5.0) * norm.between_variance / norm.total_variance;
    let expected_lambda_from_r = norm.riv / (1.0 + norm.riv);
    assert_close(norm.lambda, expected_lambda_from_b, 1e-14, "lambda = (1+1/m)B/T");
    assert_close(norm.lambda, expected_lambda_from_r, 1e-14, "lambda = r/(1+r)");
    assert!((norm.fmi - norm.lambda).abs() > 1e-3, "FMI differs from lambda by (1-lambda)*2/(df+3)");
}

// ============================================================================
// AUD-05-44: Monotone kth-selection validates ascending order without hanging
// ============================================================================

#[test]
fn aud_05_44_monotone_kth_selection_rejects_unsorted_inputs() {
    let asc: Vec<f64> = (1..=25).map(|i| i as f64).collect();
    let desc: Vec<f64> = (1..=25).rev().map(|i| i as f64).collect();

    // Unsorted xs or ys (rem = 625 > 256) must immediately return InvalidParameter, never hang.
    let err_x = select_kth_pairwise_sum(&desc, &asc, 300).unwrap_err();
    assert!(matches!(err_x, StatsError::InvalidParameter { name: "sorted_xs", .. }));
    let err_y = select_kth_pairwise_sum(&asc, &desc, 300).unwrap_err();
    assert!(matches!(err_y, StatsError::InvalidParameter { name: "sorted_ys", .. }));

    // Unsorted x in upper-triangle Walsh selection (rem = 325 > 256) must immediately error.
    let err_u = select_kth_upper_triangle_half_sum(&desc, 150).unwrap_err();
    assert!(matches!(err_u, StatsError::InvalidParameter { name: "sorted_x", .. }));

    // Sorted inputs match brute-force oracle exactly.
    let mut brute_sums = Vec::with_capacity(asc.len() * asc.len());
    for &x in &asc {
        for &y in &asc {
            brute_sums.push(x + y);
        }
    }
    brute_sums.sort_by(f64::total_cmp);
    assert_close(select_kth_pairwise_sum(&asc, &asc, 312).unwrap(), brute_sums[312], 1e-14, "pairwise kth");

    let mut brute_walsh = Vec::new();
    for i in 0..asc.len() {
        for j in i..asc.len() {
            brute_walsh.push(0.5 * (asc[i] + asc[j]));
        }
    }
    brute_walsh.sort_by(f64::total_cmp);
    assert_close(select_kth_upper_triangle_half_sum(&asc, 162).unwrap(), brute_walsh[162], 1e-14, "upper-tri kth");
}

// ============================================================================
// AUD-05-46: Scaled Welch df at 1e±100 and multiple_row_t_tests threshold check
// ============================================================================

#[test]
fn aud_05_46_welch_extreme_scale_and_multiple_row_threshold_validation() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let base = welch_t_test(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0], true, conf).unwrap();

    for scale in [1e100_f64, 1e-100_f64] {
        let a: Vec<f64> = [1.0, 2.0, 3.0].iter().map(|&x| x * scale).collect();
        let b: Vec<f64> = [4.0, 5.0, 6.0].iter().map(|&x| x * scale).collect();
        let out = welch_t_test(&a, &b, true, conf).unwrap();
        assert_close(out.df, base.df, 1e-12, &format!("welch raw df scale={scale:e}"));
        assert_close(out.t_statistic, base.t_statistic, 1e-12, &format!("welch raw t scale={scale:e}"));
        assert_close(out.p_value, base.p_value, 1e-12, &format!("welch raw p scale={scale:e}"));
    }

    let base_sum = welch_t_test_from_summary((1.0, 1.0, 10.0), (2.0, 2.0, 15.0), true, conf).unwrap();
    for scale in [1e100_f64, 1e-100_f64, 1e200_f64, 1e-200_f64] {
        let out = welch_t_test_from_summary((1.0 * scale, 1.0 * scale, 10.0), (2.0 * scale, 2.0 * scale, 15.0), true, conf).unwrap();
        assert_close(out.df, base_sum.df, 1e-12, &format!("welch summary df scale={scale:e}"));
        assert_close(out.t_statistic, base_sum.t_statistic, 1e-12, &format!("welch summary t scale={scale:e}"));
        assert_close(out.p_value, base_sum.p_value, 1e-12, &format!("welch summary p scale={scale:e}"));
    }

    // multiple_row_t_tests must validate threshold strictly in (0, 1).
    let labels = vec![String::from("Gene1")];
    let rows_a = vec![vec![1.0, 2.0, 3.0]];
    let rows_b = vec![vec![4.0, 5.0, 6.0]];
    for bad_t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -0.05, 1.0, 1.5] {
        let err = multiple_row_t_tests(&labels, &rows_a, &rows_b, true, MultiplicityProcedure::TwoStageBkyFdr, bad_t).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "threshold", .. }),
            "expected InvalidParameter for threshold={bad_t}, got {err:?}"
        );
    }
}

// ============================================================================
// AUD-05-47: kendall_tau_b sign comparison immune to dx*dy underflow
// ============================================================================

#[test]
fn aud_05_47_kendall_tau_b_no_product_underflow() {
    let tiny = [1e-200, 2e-200, 3e-200, 4e-200];
    let pos = kendall_tau_b(&tiny, &tiny).unwrap();
    assert_close(pos.r, 1.0, 1e-15, "kendall tiny positive");

    let rev = [4e-200, 3e-200, 2e-200, 1e-200];
    let neg = kendall_tau_b(&tiny, &rev).unwrap();
    assert_close(neg.r, -1.0, 1e-15, "kendall tiny negative");

    // Extreme wide span must not overflow difference either.
    let wide = [-1e308, 0.0, 1e308];
    let wide_res = kendall_tau_b(&wide, &wide).unwrap();
    assert_close(wide_res.r, 1.0, 1e-15, "kendall wide positive");
}

// ============================================================================
// AUD-05-48: Unstandardized PCA avoids 1e-30 total_var floor distortion
// ============================================================================

#[test]
fn aud_05_48_unstandardized_pca_tiny_scale_preserves_variance_proportions() {
    let base_flat = vec![
        1.0, 2.0, 0.5, 2.0, 4.1, 0.9, 3.0, 5.8, 1.6, 4.0, 8.2, 2.1, 5.0, 9.9, 2.4, 6.0, 12.1, 3.2, 7.0, 13.8, 3.5, 8.0, 16.2, 4.1, 9.0, 17.9, 4.4,
        10.0, 20.1, 5.0,
    ];
    let scale = 1e-16_f64;
    let tiny_flat: Vec<f64> = base_flat.iter().map(|&v| v * scale).collect();
    let labels = vec![String::from("V1"), String::from("V2"), String::from("V3")];

    let mat_base = DenseMatrix::from_row_major(10, 3, base_flat).unwrap();
    let mat_tiny = DenseMatrix::from_row_major(10, 3, tiny_flat).unwrap();

    let pca_base = pca_analysis(&mat_base, &labels, false, None).unwrap();
    let pca_tiny = pca_analysis(&mat_tiny, &labels, false, None).unwrap();

    let sum_prop: f64 = pca_tiny.variance_proportion.iter().sum();
    assert_close(sum_prop, 1.0, 1e-12, "tiny PCA variance_proportion sum");
    assert_eq!(pca_tiny.kaiser_components, pca_base.kaiser_components);
    assert!(pca_tiny.kaiser_components >= 1);
    for (k, (&vp_t, &vp_b)) in pca_tiny.variance_proportion.iter().zip(&pca_base.variance_proportion).enumerate() {
        assert_close(vp_t, vp_b, 1e-12, &format!("PC{} variance proportion", k + 1));
    }

    // Constant matrix (zero eigenvalues) still returns ZeroVariance cleanly.
    let const_mat = DenseMatrix::from_row_major(5, 2, vec![3.0; 10]).unwrap();
    assert!(matches!(pca_analysis(&const_mat, &[String::from("A"), String::from("B")], false, None), Err(StatsError::ZeroVariance { .. })));
}

// ============================================================================
// AUD-05-49: Sensitivity matrix keeps achievable cells & notes omitted ones
// ============================================================================

#[test]
fn aud_05_49_power_sensitivity_table_retains_valid_cells_on_partial_unachievable_grid() {
    // Fixed n2 = 50, d = 0.5: asymptotic power ceiling as n1 -> inf at alpha = 0.001 is ~0.597 < 0.95,
    // while primary (0.05, 0.80) succeeds (exact noncentral-t required n1 = 88; n1 = 87 yields ~0.7991 < 0.80).
    // Sensitivity table must keep all achievable cells.
    let req = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(50.0);
    let res = solve_power(&req).unwrap();
    assert_eq!(res.n_per_group, 88);
    assert!(res.achieved_power >= 0.80);
    let below = solve_power(
        &PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(87.0).with_sample_size_group2(50.0),
    )
    .unwrap();
    assert!(below.achieved_power < 0.80);
    assert!(
        !res.sensitivity_table.is_empty() && res.sensitivity_table.len() < 16,
        "expected partial sensitivity table (1..16 cells), got {}",
        res.sensitivity_table.len()
    );
    assert!(
        res.sensitivity_table.iter().any(|c| (c.alpha - 0.05).abs() < 1e-12 && (c.target_power - 0.80).abs() < 1e-12),
        "expected (0.05, 0.80) cell to be present"
    );
    assert!(
        res.notes.iter().any(|n| n.contains("Sensitivity table omitted")),
        "expected convention note reporting omitted cells, got {:?}",
        res.notes
    );

    // Live run_power_analysis must still emit the Sensitivity Matrix tab.
    let dummy_sheet = make_column_sheet("pow-src", &[("A", &[1.0, 2.0])]);
    let doc_pow = PowerAnalysisSpec {
        family: curvebook_doc::PowerTestFamily::TwoSampleUnpairedT,
        solve_target: curvebook_doc::PowerSolveTarget::SampleSize,
        alpha: 0.05,
        target_power: Some(0.80),
        sample_size_group2: Some(50.0),
        effect: curvebook_doc::PowerEffectSpec::Standardized(0.5),
        ..PowerAnalysisSpec::default()
    };
    let spec = AnalysisSpec::new(SheetId::new("res-pow"), dummy_sheet.id.clone(), AnalysisKind::PowerAnalysis(doc_pow.clone())).unwrap();
    let sheet_res = run_power_analysis(&dummy_sheet, &spec, &doc_pow).unwrap();
    assert!(sheet_res.tabs.iter().any(|t| t.id == "power-sensitivity"));

    // Direct compute_sensitivity_table still rejects invalid alpha and all-failing grids.
    assert!(compute_sensitivity_table(&req, &[1.5], &[0.80]).is_err());
    assert!(compute_sensitivity_table(&req, &[0.001], &[0.95]).is_err());
}

// ============================================================================
// AUD-05-50: ChiSquare SampleSize enforces MAX_CONTINUOUS_N -> NotConverged
// ============================================================================

#[test]
fn aud_05_50_chisquare_enforces_max_continuous_n() {
    for tiny_w in [1e-8_f64, 1e-200_f64] {
        let req = PowerAnalysisRequest::chi_square(tiny_w, 1.0, 0.05, 0.80);
        let err = solve_power(&req).unwrap_err();
        assert!(
            matches!(err, StatsError::NotConverged { algorithm: "solve_continuous_n" }),
            "expected NotConverged(solve_continuous_n) for w={tiny_w:e}, got {err:?}"
        );
    }

    // Normal ChiSquare Cohen (1988, Table 7.3.15) w = 0.30, df = 1, alpha = 0.05, power = 0.80 -> N = 88.
    let ok = solve_power(&PowerAnalysisRequest::chi_square(0.30, 1.0, 0.05, 0.80)).unwrap();
    assert_eq!(ok.total_n, 88);
}

// ============================================================================
// AUD-05-54: column.rs sd_ci / quadratic_mean_ci on zero data & outlier error
// ============================================================================

#[test]
fn aud_05_54_column_zero_variance_cis_and_outlier_presummarized_error() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // Constant nonzero sample: sd == Some(0.0), sd_ci == None (matches rustdoc).
    let const_sum = summarize_column("Const", &[5.0, 5.0, 5.0, 5.0], 0, conf, None).unwrap();
    assert_eq!(const_sum.sd, Some(0.0));
    assert_eq!(const_sum.sd_ci, None);

    // All-zero sample: quadratic_mean == 0.0, quadratic_mean_ci == None, sd_ci == None.
    let zero_sum = summarize_column("Zero", &[0.0, 0.0, 0.0, 0.0], 0, conf, None).unwrap();
    assert_eq!(zero_sum.quadratic_mean, 0.0);
    assert_eq!(zero_sum.quadratic_mean_ci, None);
    assert_eq!(zero_sum.sd_ci, None);

    // Pre-summarized zero-variance points: sd == Some(0.0), sd_ci == None.
    let pts = vec![SummarizedPoint { mean: 10.0, sd: 0.0, n: 5.0, ss_within: 0.0, df_within: 4.0 }];
    let pooled = summarize_pre_summarized_points("ConstPool", &pts, conf).unwrap();
    assert_eq!(pooled.sd, Some(0.0));
    assert_eq!(pooled.sd_ci, None);

    // Outlier analysis on a populated MeanSdN ColumnTable must return RequiresRawReplicates (not EmptyInput).
    let mut sum_table = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 2).unwrap();
    sum_table.data_sets[0].title = String::from("GroupA");
    sum_table.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(1.5), Cell::number(6.0)];
    sum_table.data_sets[0].cells[1] = vec![Cell::number(12.0), Cell::number(2.0), Cell::number(6.0)];
    let sum_sheet = DataSheet::new(SheetId::new("sum-out"), "SumOut", TableArchetype::Column(sum_table)).unwrap();
    let outlier = OutlierSpec::Grubbs { alpha: 0.05, iterative: false };
    let spec = AnalysisSpec::new(SheetId::new("res-out"), sum_sheet.id.clone(), AnalysisKind::OutlierDetection(outlier)).unwrap();
    let err = run_outlier_analysis(&sum_sheet, &spec, outlier).unwrap_err();
    assert!(matches!(err, StatsError::RequiresRawReplicates), "expected RequiresRawReplicates, got {err:?}");

    // Empty MeanSdN ColumnTable still returns EmptyInput.
    let empty_table = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 2).unwrap();
    let empty_sheet = DataSheet::new(SheetId::new("empty-out"), "EmptyOut", TableArchetype::Column(empty_table)).unwrap();
    let empty_spec = AnalysisSpec::new(SheetId::new("res-empty-out"), empty_sheet.id.clone(), AnalysisKind::OutlierDetection(outlier)).unwrap();
    let empty_err = run_outlier_analysis(&empty_sheet, &empty_spec, outlier).unwrap_err();
    assert!(matches!(empty_err, StatsError::EmptyInput), "expected EmptyInput for blank MeanSdN table, got {empty_err:?}");
}
