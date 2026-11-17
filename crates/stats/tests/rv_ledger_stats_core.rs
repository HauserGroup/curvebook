//! Regression and primary-literature reference tests for `stats-core` audit ledger items
//! (`RV-01`, `RV-02`, `RV-03`, `RV-29`, `RV-30`, `RV-65`, `RV-66`, `RV-71`, `RV-72`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CiBound, ColumnDesignHint, ColumnTable, DataSheet, FactorialAnovaSpec, GroupedTable, MixedModelDfMethod,
    MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, NestedAnalysisSpec, NestedTable, OneWayAnovaSpec, OneWayMethod,
    RepeatedMeasuresDesign, ResidualCovariance, ResultCell, SubcolumnMode, SumOfSquaresType, TableArchetype, ThreeWayColumnGrouping, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{
    dunnett_two_sided_p_loadings, max_studentized_range_isf, max_studentized_range_sf, student_t_two_sided_isf, student_t_two_sided_p,
};
use curvebook_stats::StatsError;
use curvebook_stats::anova::{
    RmSphericityCorrection, fit_rm_two_way_anova, fit_rm_two_way_anova_with_sphericity, rm_one_way_anova, run_factorial_anova, run_nested_analysis,
    run_one_way_anova,
};
use curvebook_stats::categorical::{fisher_exact_2x2, fisher_exact_rxc};
use curvebook_stats::column::{one_sample_wilcoxon, rout_outliers};
use curvebook_stats::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, adjust_benjamini_hochberg, adjust_benjamini_yekutieli, adjust_bonferroni, adjust_holm_bonferroni,
    adjust_holm_sidak, adjust_p_values, adjust_sidak, adjust_two_stage_bky, linear_trend_test,
};
use curvebook_stats::two_group::{levene_variance_test, mann_whitney_u, multiple_row_t_tests};

fn assert_approx(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, ctx: &str) {
    assert!(actual.is_finite(), "{ctx}: expected finite {expected}, got {actual}");
    let diff = (actual - expected).abs();
    let scale = expected.abs().max(1e-300);
    assert!(diff <= abs_tol || diff / scale <= rel_tol, "{ctx}: actual={actual:.15e}, expected={expected:.15e}, diff={diff:.3e}");
}

// ============================================================================
// RV-01: Fisher exact 2x2 and RxC relative log-probability tolerance
// ============================================================================

#[test]
fn rv01_fisher_exact_large_n_symmetric_and_mirror_tables() {
    // Balanced 2x2 at N = 200,000: point probability at the mode equals the maximal term, so p == 1.0.
    let p_bal = fisher_exact_2x2(50_000, 50_000, 50_000, 50_000).unwrap();
    assert_eq!(p_bal, 1.0, "balanced 2x2 at N=200,000 must return exact 1.0");

    // Twin-mode hypergeometric table at N = 200,000 (r1 = 100,001, r2 = 99,999, c1 = c2 = 100,000):
    // X = 50,000 and X = 50,001 have identical exact hypergeometric probabilities, so both modes
    // must include each other despite ~4.7e-10 ULP rounding of ln(N!) at magnitude ~2.1e6.
    let p_mode_left = fisher_exact_2x2(50_000, 50_001, 50_000, 49_999).unwrap();
    let p_mode_right = fisher_exact_2x2(50_001, 50_000, 49_999, 50_000).unwrap();
    assert_eq!(p_mode_left, 1.0, "left twin mode at N=200,000 must include right twin mode");
    assert_eq!(p_mode_right, 1.0, "right twin mode at N=200,000 must include left twin mode");

    // Asymmetric row totals (80,000 vs 120,000) with equal column totals (100,000 vs 100,000):
    // X = 40,000 - k and X' = 40,000 + k have equal exact hypergeometric probabilities, while
    // left-to-right summation ln(a!) + ln(b!) + ln(c!) + ln(d!) permutes the last two summands.
    for k in [0_u64, 150, 300, 450] {
        let p1 = fisher_exact_2x2(40_000 - k, 40_000 + k, 60_000 + k, 60_000 - k).unwrap();
        let p2 = fisher_exact_2x2(40_000 + k, 40_000 - k, 60_000 - k, 60_000 + k).unwrap();
        if k == 0 {
            assert_eq!(p1, 1.0);
        }
        assert!((p1 - p2).abs() <= 1e-13, "mirror tables at k={k} must agree to machine precision: p1={p1:.16e}, p2={p2:.16e}");
    }

    // Modal 3x3 table in `fisher_exact_rxc`: every Monte Carlo permutation has probability <= modal table,
    // including symmetric permutations whose log-factorial sums differ by floating-point rounding.
    let modal_3x3 = vec![vec![1_000_u64, 1_000, 1_000], vec![1_000, 1_000, 1_000], vec![1_000, 1_000, 1_000]];
    let p_rxc = fisher_exact_rxc(&modal_3x3).unwrap();
    assert_eq!(p_rxc, 1.0, "modal 3x3 table must count every permutation as <= modal probability");
}

// ============================================================================
// RV-02: Multiplicity NaN / non-finite raw p-value guards
// ============================================================================

#[test]
fn rv02_multiplicity_adjusters_preserve_nan_and_exclude_nonfinite_from_family() {
    // Reproducer from RV-02: `[0.01, NaN]` in `adjust_holm_sidak` must never propagate `running_max`
    // into the NaN slot, and `m_finite = 1` leaves the single valid entry at 0.01.
    for input in [vec![0.01, f64::NAN], vec![f64::NAN, 0.01]] {
        let hs = adjust_holm_sidak(&input);
        let finite_idx = if input[0].is_finite() { 0 } else { 1 };
        let nan_idx = 1 - finite_idx;
        assert!(hs[nan_idx].is_nan(), "NaN slot must stay NaN in adjust_holm_sidak, got {:?}", hs);
        assert_approx(hs[finite_idx], 0.01, 1e-15, 1e-12, "single finite entry in adjust_holm_sidak");
    }

    let mixed = [0.01, f64::NAN, 0.04, f64::INFINITY, f64::NEG_INFINITY];
    let finite_only = [0.01, 0.04];

    for (name, got, want) in [
        ("bonferroni", adjust_bonferroni(&mixed), adjust_bonferroni(&finite_only)),
        ("sidak", adjust_sidak(&mixed), adjust_sidak(&finite_only)),
        ("holm_bonferroni", adjust_holm_bonferroni(&mixed), adjust_holm_bonferroni(&finite_only)),
        ("holm_sidak", adjust_holm_sidak(&mixed), adjust_holm_sidak(&finite_only)),
        ("bh", adjust_benjamini_hochberg(&mixed), adjust_benjamini_hochberg(&finite_only)),
        ("by", adjust_benjamini_yekutieli(&mixed), adjust_benjamini_yekutieli(&finite_only)),
        ("bky", adjust_two_stage_bky(&mixed, 0.05), adjust_two_stage_bky(&finite_only, 0.05)),
    ] {
        assert_approx(got[0], want[0], 1e-15, 1e-12, name);
        assert!(got[1].is_nan(), "{name}: index 1 (NaN) must remain NaN, got {}", got[1]);
        assert_approx(got[2], want[1], 1e-15, 1e-12, name);
        assert!(got[3].is_nan(), "{name}: index 3 (+Inf) must become NaN, got {}", got[3]);
        assert!(got[4].is_nan(), "{name}: index 4 (-Inf) must become NaN, got {}", got[4]);
    }

    for proc_kind in [
        MultiplicityProcedure::None,
        MultiplicityProcedure::FisherLsd,
        MultiplicityProcedure::Bonferroni,
        MultiplicityProcedure::Sidak,
        MultiplicityProcedure::HolmBonferroni,
        MultiplicityProcedure::HolmSidak,
        MultiplicityProcedure::BenjaminiHochbergFdr,
        MultiplicityProcedure::BenjaminiYekutieliFdr,
        MultiplicityProcedure::TwoStageBkyFdr,
    ] {
        let (adj, _is_fdr) = adjust_p_values(&mixed, proc_kind, 0.05);
        assert!(adj[1].is_nan(), "{proc_kind:?}: NaN entry must stay NaN");
        assert!(adj[3].is_nan(), "{proc_kind:?}: +Inf entry must become NaN");
        assert!(adj[4].is_nan(), "{proc_kind:?}: -Inf entry must become NaN");
        assert!(adj[0].is_finite() && adj[0] <= 0.05, "{proc_kind:?}: p=0.01 must remain finite & <= 0.05");
    }

    let (all_nan, _) = adjust_p_values(&[f64::NAN, f64::NAN], MultiplicityProcedure::HolmSidak, 0.05);
    assert!(all_nan.iter().all(|v| v.is_nan()));

    // Caller guard check on `multiple_row_t_tests`: an under-populated row alongside valid rows
    // is skipped cleanly without poisoning valid rows' adjusted p-values.
    let labels = vec![String::from("Row1"), String::from("Degenerate"), String::from("Row3")];
    let ga = vec![vec![10.0, 10.2, 9.8, 10.1], vec![5.0], vec![10.0, 10.5, 9.5, 10.0]];
    let gb = vec![vec![14.0, 14.2, 13.9, 14.1], vec![6.0], vec![10.1, 10.4, 9.6, 10.0]];
    let mr = multiple_row_t_tests(&labels, &ga, &gb, false, MultiplicityProcedure::HolmSidak, 0.05).unwrap();
    assert_eq!(mr.len(), 2);
    assert!(mr.iter().all(|r| r.adjusted_p_or_q.is_finite()));
}

// ============================================================================
// RV-03: Empty-input safety without eager `s[0]` indexing
// ============================================================================

#[test]
fn rv03_empty_slices_return_typed_errors_without_panicking() {
    let conf = ConfidenceLevel::DEFAULT_95;
    assert!(matches!(mann_whitney_u(&[], &[1.0, 2.0, 3.0], true, conf), Err(StatsError::EmptyInput)));
    assert!(matches!(mann_whitney_u(&[1.0, 2.0, 3.0], &[], true, conf), Err(StatsError::EmptyInput)));
    assert!(matches!(levene_variance_test(&[&[], &[1.0, 2.0, 3.0]], true), Err(StatsError::EmptyInput)));
    assert!(matches!(rout_outliers(&[], 1.0), Err(StatsError::EmptyInput)));
    assert!(matches!(one_sample_wilcoxon(&[], 0.0, true, conf), Err(StatsError::EmptyInput)));
}

// ============================================================================
// RV-29: Lecoutre (1991) `(N - g + 1)` split-plot Huynh-Feldt correction
// ============================================================================

#[test]
fn rv29_split_plot_huynh_feldt_matches_lecoutre_1991_across_three_groups() {
    // g = 3 independent groups, s = 4 subjects/group -> N = 12, df_e = N - g = 9, k = 4 repeated levels (p = 3).
    // Residual profiles have strongly unequal variances across contrasts so eps_GG is around 0.45
    // and Huynh-Feldt stays strictly inside (eps_GG, 1.0).
    let base: [[f64; 4]; 4] = [[1.0, 2.0, 3.0, 11.0], [2.0, 3.0, 2.0, 15.0], [0.0, 1.0, 4.0, 8.0], [3.0, 2.0, 3.0, 16.0]];
    let g0 = base;
    let mut g1 = base;
    let mut g2 = base;
    for s in 0..4 {
        for t in 0..4 {
            g1[s][t] += 5.0 + 0.5 * (t as f64) + if s % 2 == 0 { 0.3 * (t as f64) } else { -0.3 * (t as f64) };
            g2[s][t] += 10.0 + (t as f64) + if s < 2 { 0.2 * ((t * t) as f64) } else { -0.2 * ((t * t) as f64) };
        }
    }
    let mut subject_data = Vec::with_capacity(4);
    for s in 0..4 {
        subject_data.push(vec![g0[s].to_vec(), g1[s].to_vec(), g2[s].to_vec()]);
    }

    let (_fac, sph_opt) = fit_rm_two_way_anova(&subject_data, "Group", "Time", RepeatedMeasuresDesign::ColumnFactorMatched).unwrap();
    let sph = sph_opt.unwrap();

    // Independent computation of Lecoutre (1991, J. Educ. Stat. 16:371-372):
    // eps_tilde = ((N - g + 1) * p * eps_hat - 2) / (p * (df_e - p * eps_hat)), with N = 12, g = 3, df_e = 9, p = 3.
    let n_total = 12.0_f64;
    let g_count = 3.0_f64;
    let df_e = n_total - g_count;
    let p = 3.0_f64;
    let eps_gg = sph.greenhouse_geisser_eps;
    let expected_lecoutre = (((n_total - g_count + 1.0) * p * eps_gg - 2.0) / (p * (df_e - p * eps_gg))).clamp(eps_gg, 1.0);
    let uncorrected_hf = ((n_total * p * eps_gg - 2.0) / (p * (df_e - p * eps_gg))).clamp(eps_gg, 1.0);

    assert!(eps_gg > 0.34 && eps_gg < 0.80, "test design must produce intermediate eps_GG, got {eps_gg}");
    assert!((expected_lecoutre - uncorrected_hf).abs() > 0.02, "Lecoutre and uncorrected HF must be clearly separated");
    assert_approx(sph.huynh_feldt_eps, expected_lecoutre, 1e-12, 1e-12, "Lecoutre (1991) split-plot HF epsilon");
}

// ============================================================================
// RV-30: RM ANOVA with `n = 2` (`df_e = 1`) and `k > 2` uses `1 / (k - 1)` lower bound
// ============================================================================

#[test]
fn rv30_rm_anova_two_subjects_uses_rank1_lower_bound_and_nan_mauchly_p() {
    // n = 2 subjects (`df_e = 1`), k = 4 repeated conditions (`p = 3` orthonormal contrasts):
    // sample covariance has rank 1, so exact GG and HF epsilons equal `1 / p = 1/3` and Mauchly's p is NaN.
    let subj_non_deg = vec![vec![10.0, 12.0, 15.0, 22.0], vec![11.0, 15.0, 14.0, 30.0]];
    let rm = rm_one_way_anova(&subj_non_deg).unwrap();
    assert_approx(rm.sphericity.greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "n=2, k=4 rank-1 GG eps");
    assert_approx(rm.sphericity.huynh_feldt_eps, 1.0 / 3.0, 1e-12, 1e-12, "n=2, k=4 rank-1 HF eps");
    assert!(rm.sphericity.mauchly_p.is_nan(), "Mauchly p is inestimable for df_e=1 < p=3, got {}", rm.sphericity.mauchly_p);

    // Degenerate zero-residual covariance (parallel subject profiles -> contrast covariance trace == 0)
    // must also return conservative lower bound `1 / (k - 1)` rather than `1.0`.
    let subj_zero_cov = vec![vec![10.0, 12.0, 15.0, 22.0], vec![20.0, 22.0, 25.0, 32.0]];
    let rm_zero = rm_one_way_anova(&subj_zero_cov).unwrap();
    assert_approx(rm_zero.sphericity.greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "zero-cov GG lower bound");
    assert_approx(rm_zero.sphericity.huynh_feldt_eps, 1.0 / 3.0, 1e-12, 1e-12, "zero-cov HF lower bound");
    assert!(rm_zero.sphericity.mauchly_p.is_nan(), "zero-cov Mauchly p must be NaN");

    // Two-way `BothFactorsMatched` with n = 2 subjects on 2 x 4 design:
    // Row factor (2 levels, p = 1) has exact epsilon = 1.0, while Column (4 levels, p = 3) and Interaction have epsilon = 1/3.
    let two_way_n2 =
        vec![vec![vec![10.0, 12.0, 16.0, 25.0], vec![14.0, 18.0, 21.0, 33.0]], vec![vec![11.0, 15.0, 15.0, 31.0], vec![16.0, 17.0, 26.0, 40.0]]];
    let fit_gg = fit_rm_two_way_anova_with_sphericity(
        &two_way_n2,
        "Row",
        "Col",
        RepeatedMeasuresDesign::BothFactorsMatched,
        RmSphericityCorrection::GreenhouseGeisser,
    )
    .unwrap();
    let row_sph = fit_gg.sphericities.row.unwrap();
    let col_sph = fit_gg.sphericities.column.unwrap();
    let int_sph = fit_gg.sphericities.interaction.unwrap();
    assert_approx(row_sph.greenhouse_geisser_eps, 1.0, 1e-12, 1e-12, "2-level factor has exact sphericity 1.0");
    assert_approx(col_sph.greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "4-level col factor at n=2 has GG 1/3");
    assert_approx(col_sph.huynh_feldt_eps, 1.0 / 3.0, 1e-12, 1e-12, "4-level col factor at n=2 has HF 1/3");
    assert!(col_sph.mauchly_p.is_nan());
    assert_approx(int_sph.greenhouse_geisser_eps, 1.0 / 3.0, 1e-12, 1e-12, "interaction at n=2 has GG 1/3");
    assert!(int_sph.mauchly_p.is_nan());
}

// ============================================================================
// RV-65: Post-hoc CI uses `1 - confidence_level` while `threshold` governs significance
// ============================================================================

fn extract_first_ci(sheet: &curvebook_doc::ResultSheet, tab_id: &str) -> (String, (f64, f64)) {
    let tab = sheet.tabs.iter().find(|t| t.id == tab_id).expect("expected post-hoc tab");
    let ci_col = tab.column_headers.iter().position(|h| h.contains("CI")).expect("expected CI column");
    let header = tab.column_headers[ci_col].clone();
    let pair = match tab.rows[0][ci_col] {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => Some((lo, hi)),
        _ => None,
    }
    .expect("expected finite ConfidenceInterval cell");
    (header, pair)
}

#[test]
fn rv65_posthoc_ci_tracks_confidence_level_independently_of_significance_threshold() {
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap();
    for (c, (name, vals)) in
        [("Ctrl", [10.0, 10.4, 9.6, 10.2, 9.8]), ("Dose1", [10.8, 11.2, 10.4, 11.0, 10.6]), ("Dose2", [14.0, 14.4, 13.6, 14.2, 13.8])]
            .into_iter()
            .enumerate()
    {
        col_tbl.data_sets[c].title = String::from(name);
        for (r, v) in vals.into_iter().enumerate() {
            col_tbl.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let sheet = DataSheet::new("col-rv65", "RV65 OneWay", TableArchetype::Column(col_tbl)).unwrap();

    for (ow_method, mult_method) in [
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::TukeyHsd),
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::Bonferroni),
        (OneWayMethod::OrdinaryAnova, MultiplicityMethod::Dunnett { control_column: 0 }),
        (OneWayMethod::BrownForsytheAndWelch, MultiplicityMethod::GamesHowell),
        (OneWayMethod::BrownForsytheAndWelch, MultiplicityMethod::DunnettT3),
    ] {
        let ow_05 = OneWayAnovaSpec {
            method: ow_method,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: mult_method, threshold: 0.05, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        };
        let ow_001 = OneWayAnovaSpec {
            method: ow_method,
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: mult_method, threshold: 0.001, report_confidence_intervals: true },
            ..OneWayAnovaSpec::default()
        };
        let mut spec_95_t05 = AnalysisSpec::new("an1", "col-rv65", AnalysisKind::OneWayAnova(ow_05.clone())).unwrap();
        spec_95_t05.confidence_level = ConfidenceLevel::DEFAULT_95;
        let mut spec_95_t001 = AnalysisSpec::new("an2", "col-rv65", AnalysisKind::OneWayAnova(ow_001.clone())).unwrap();
        spec_95_t001.confidence_level = ConfidenceLevel::DEFAULT_95;
        let mut spec_99_t05 = AnalysisSpec::new("an3", "col-rv65", AnalysisKind::OneWayAnova(ow_05.clone())).unwrap();
        spec_99_t05.set_confidence_percent(99.0).unwrap();

        let res_95_t05 = run_one_way_anova(&sheet, &spec_95_t05, &ow_05).unwrap();
        let res_95_t001 = run_one_way_anova(&sheet, &spec_95_t001, &ow_001).unwrap();
        let res_99_t05 = run_one_way_anova(&sheet, &spec_99_t05, &ow_05).unwrap();

        let (hdr_95_a, ci_95_a) = extract_first_ci(&res_95_t05, "multiple-comparisons");
        let (hdr_95_b, ci_95_b) = extract_first_ci(&res_95_t001, "multiple-comparisons");
        let (hdr_99, ci_99) = extract_first_ci(&res_99_t05, "multiple-comparisons");

        assert!(hdr_95_a.contains("95% CI") && hdr_95_b.contains("95% CI"));
        assert!(hdr_99.contains("99% CI"));
        // Same confidence (95%) with different thresholds (0.05 vs 0.001) must produce identical 95% CIs.
        assert_approx(ci_95_a.0, ci_95_b.0, 1e-13, 1e-13, "95% CI lower independent of threshold");
        assert_approx(ci_95_a.1, ci_95_b.1, 1e-13, 1e-13, "95% CI upper independent of threshold");
        // 99% CI must be strictly wider than 95% CI.
        assert!((ci_99.1 - ci_99.0) > (ci_95_a.1 - ci_95_a.0) + 1e-4, "{mult_method:?}: 99% CI must be wider than 95% CI");
        // Significance flags must remain identical when changing confidence (95% -> 99%) at fixed threshold 0.05.
        let sig_95: Vec<bool> = res_95_t05.pairwise_comparisons.iter().map(|c| c.significant).collect();
        let sig_99: Vec<bool> = res_99_t05.pairwise_comparisons.iter().map(|c| c.significant).collect();
        assert_eq!(sig_95, sig_99, "{mult_method:?}: significance depends on threshold, not CI confidence level");
        // Tightening threshold from 0.05 to 0.001 flips the moderate Ctrl vs Dose1 comparison (`diff = 0.8`, `t ≈ 4.0`).
        assert!(
            res_95_t05.pairwise_comparisons[0].significant && !res_95_t001.pairwise_comparisons[0].significant,
            "{mult_method:?}: threshold=0.05 vs 0.001 must flip significance on Ctrl vs Dose1 (p={})",
            res_95_t05.pairwise_comparisons[0].raw_p_or_q
        );
    }
}

// ============================================================================
// RV-66: Factorial Dunnett per-factor `control_column` bounds & single target
// ============================================================================

fn build_grouped_sheet(id: &str, n_rows: usize, n_cols: usize, n_rep: usize) -> DataSheet {
    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(n_rep), n_cols, n_rows).unwrap();
    for c in 0..n_cols {
        grouped.data_sets[c].title = format!("B{c}");
        for r in 0..n_rows {
            for s in 0..n_rep {
                let val = 10.0 + 3.0 * (r as f64) + 2.0 * (c as f64) + 0.4 * (s as f64) + 0.1 * ((r * c) as f64);
                grouped.data_sets[c].cells[r][s] = Cell::number(val);
            }
        }
    }
    for r in 0..n_rows {
        grouped.row_titles[r] = format!("A{r}");
    }
    DataSheet::new(id, id, TableArchetype::Grouped(grouped)).unwrap()
}

#[test]
fn rv66_factorial_dunnett_handles_asymmetric_factor_levels_and_avoids_cross_factor_leakage() {
    let dunnett_ctrl2 = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 2 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..FactorialAnovaSpec::default()
    };

    // 1. 3x2 design (3 rows on Factor A, 2 columns on Factor B) with control_column = 2:
    // Must succeed on Factor A (and simple A within B) without failing on 2-level Factor B.
    let sheet_3x2 = build_grouped_sheet("g3x2", 3, 2, 3);
    let spec_3x2 = AnalysisSpec::new("an-3x2", "g3x2", AnalysisKind::FactorialAnova(dunnett_ctrl2.clone())).unwrap();
    let res_3x2 = run_factorial_anova(&sheet_3x2, &spec_3x2, &dunnett_ctrl2).unwrap();
    assert!(res_3x2.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-a"));
    assert!(res_3x2.tabs.iter().any(|t| t.id == "multiple-comparisons-simple-a-within-b"));
    assert!(!res_3x2.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));
    assert!(!res_3x2.tabs.iter().any(|t| t.id == "multiple-comparisons-simple-b-within-a"));

    // 2. 2x3 design (2 rows on Factor A, 3 columns on Factor B) with control_column = 2:
    // Factor A has only 2 levels, so Dunnett cleanly resolves to Factor B (where index 2 is valid).
    let sheet_2x3 = build_grouped_sheet("g2x3", 2, 3, 3);
    let spec_2x3 = AnalysisSpec::new("an-2x3", "g2x3", AnalysisKind::FactorialAnova(dunnett_ctrl2.clone())).unwrap();
    let res_2x3 = run_factorial_anova(&sheet_2x3, &spec_2x3, &dunnett_ctrl2).unwrap();
    assert!(!res_2x3.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-a"));
    assert!(res_2x3.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));
    assert!(res_2x3.tabs.iter().any(|t| t.id == "multiple-comparisons-simple-b-within-a"));

    // 3. 3x3 design with control_column = 2: targets primary Factor A only rather than treating
    // unrelated Factor B level 2 as a second control group.
    let sheet_3x3 = build_grouped_sheet("g3x3", 3, 3, 3);
    let spec_3x3 = AnalysisSpec::new("an-3x3", "g3x3", AnalysisKind::FactorialAnova(dunnett_ctrl2.clone())).unwrap();
    let res_3x3 = run_factorial_anova(&sheet_3x3, &spec_3x3, &dunnett_ctrl2).unwrap();
    let fac_a_tab = res_3x3.tabs.iter().find(|t| t.id == "multiple-comparisons-factor-a").unwrap();
    assert!(fac_a_tab.footnotes.iter().any(|n| n.contains("index 2")));
    assert!(!res_3x3.tabs.iter().any(|t| t.id == "multiple-comparisons-factor-b"));

    // 4. Out-of-bounds control_column for ALL factors still returns `InvalidParameter`.
    let dunnett_ctrl5 = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 5 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..FactorialAnovaSpec::default()
    };
    let spec_err = AnalysisSpec::new("an-err", "g3x2", AnalysisKind::FactorialAnova(dunnett_ctrl5.clone())).unwrap();
    assert!(matches!(
        run_factorial_anova(&sheet_3x2, &spec_err, &dunnett_ctrl5),
        Err(StatsError::InvalidParameter { name: "control_column", value }) if (value - 5.0).abs() < 1e-12
    ));
}

// ============================================================================
// RV-71: Simple-effects family size consistency & 2-level Newman-Keuls step-down
// ============================================================================

fn cell_num(cell: &ResultCell) -> f64 {
    match *cell {
        ResultCell::Number { value, .. } => Some(value),
        _ => None,
    }
    .expect("expected ResultCell::Number")
}

fn cell_p(cell: &ResultCell) -> f64 {
    match *cell {
        ResultCell::PValue { raw, .. } => Some(raw),
        _ => None,
    }
    .expect("expected ResultCell::PValue")
}

#[test]
fn rv71_simple_effects_adjust_across_all_slices_and_two_level_snk_is_step_down() {
    // 1. 3x3 design: 3 conditioning slices x 3 pairs/slice = 9 simple-effect comparisons.
    let sheet_3x3 = build_grouped_sheet("g3x3-se", 3, 3, 4);

    // Bonferroni across all 9 simple-effect comparisons (not 3 per slice).
    let bonf_spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::Bonferroni, threshold: 0.05, report_confidence_intervals: false },
        ..FactorialAnovaSpec::default()
    };
    let an_bonf = AnalysisSpec::new("an-se-bonf", "g3x3-se", AnalysisKind::FactorialAnova(bonf_spec.clone())).unwrap();
    let res_bonf = run_factorial_anova(&sheet_3x3, &an_bonf, &bonf_spec).unwrap();
    let se_tab_bonf = res_bonf.tabs.iter().find(|t| t.id == "multiple-comparisons-simple-a-within-b").unwrap();
    assert_eq!(se_tab_bonf.rows.len(), 9);
    assert!(se_tab_bonf.footnotes.iter().any(|n| n.contains("9 simple-effect comparisons")));

    let raw_col = se_tab_bonf.column_headers.iter().position(|h| h.contains("Raw")).unwrap();
    let adj_col = se_tab_bonf.column_headers.iter().position(|h| h.contains("Adjusted")).unwrap();
    for r in 0..9 {
        let raw_p = cell_p(&se_tab_bonf.rows[r][raw_col]);
        let bonf_p = cell_p(&se_tab_bonf.rows[r][adj_col]);
        assert_approx(bonf_p, (9.0 * raw_p).min(1.0), 1e-12, 1e-10, "3x3 simple-effects Bonferroni m=9");
    }

    // Tukey HSD simple effects on 3x3 uses Copenhaver-Holland `max_studentized_range_sf(q, 3, 3, df)`.
    let tukey_spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: true },
        ..FactorialAnovaSpec::default()
    };
    let an_tukey = AnalysisSpec::new("an-se-tukey", "g3x3-se", AnalysisKind::FactorialAnova(tukey_spec.clone())).unwrap();
    let res_tukey = run_factorial_anova(&sheet_3x3, &an_tukey, &tukey_spec).unwrap();
    let se_tab_tukey = res_tukey.tabs.iter().find(|t| t.id == "multiple-comparisons-simple-a-within-b").unwrap();
    let stat_col = se_tab_tukey.column_headers.iter().position(|h| h == "q" || h.contains("Statistic") || h.contains("t / q")).unwrap();
    let df_col = se_tab_tukey.column_headers.iter().position(|h| h.eq_ignore_ascii_case("df")).unwrap();
    let tuk_adj_col = se_tab_tukey.column_headers.iter().position(|h| h.contains("Adjusted")).unwrap();
    let q0 = cell_num(&se_tab_tukey.rows[0][stat_col]);
    let df0 = cell_num(&se_tab_tukey.rows[0][df_col]);
    let adj_p0 = cell_p(&se_tab_tukey.rows[0][tuk_adj_col]);
    let expected_tukey_se = max_studentized_range_sf(q0, 3, 3, df0).unwrap();
    assert_approx(adj_p0, expected_tukey_se, 1e-12, 1e-10, "3x3 simple-effects Tukey max_studentized_range_sf(m=3, k=3)");

    // 2. 4x2 design (4 rows on Factor A, 2 cols on Factor B): `B within A` has 4 two-level simple-effect slices.
    // Craft progressively smaller B1 - B0 differences across A0..A3 so Newman-Keuls step-down (`m - rank` = 4, 3, 2, 1)
    // is strictly less conservative on later ranks than single-step `m = 4` (Tukey/SMM).
    let mut grouped_4x2 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 2, 4).unwrap();
    let diffs = [4.0, 2.2, 1.1, 0.25];
    for r in 0..4 {
        grouped_4x2.row_titles[r] = format!("A{r}");
        for (s, &noise) in [-0.6, -0.2, 0.2, 0.6].iter().enumerate() {
            grouped_4x2.data_sets[0].cells[r][s] = Cell::number(10.0 + noise);
            grouped_4x2.data_sets[1].cells[r][s] = Cell::number(10.0 + diffs[r] + noise);
        }
    }
    let sheet_4x2 = DataSheet::new("g4x2-snk", "4x2 SNK", TableArchetype::Grouped(grouped_4x2)).unwrap();
    let snk_spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::NewmanKeuls, threshold: 0.05, report_confidence_intervals: false },
        ..FactorialAnovaSpec::default()
    };
    let tuk_4x2_spec = FactorialAnovaSpec {
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, threshold: 0.05, report_confidence_intervals: false },
        ..FactorialAnovaSpec::default()
    };
    let res_snk =
        run_factorial_anova(&sheet_4x2, &AnalysisSpec::new("s1", "g4x2-snk", AnalysisKind::FactorialAnova(snk_spec.clone())).unwrap(), &snk_spec)
            .unwrap();
    let res_tuk = run_factorial_anova(
        &sheet_4x2,
        &AnalysisSpec::new("s2", "g4x2-snk", AnalysisKind::FactorialAnova(tuk_4x2_spec.clone())).unwrap(),
        &tuk_4x2_spec,
    )
    .unwrap();

    let tab_snk = res_snk.tabs.iter().find(|t| t.id == "multiple-comparisons-simple-b-within-a").unwrap();
    let tab_tuk = res_tuk.tabs.iter().find(|t| t.id == "multiple-comparisons-simple-b-within-a").unwrap();
    let get_adj = |tab: &curvebook_doc::ResultTab, row: usize| -> f64 {
        let col = tab.column_headers.iter().position(|h| h.contains("Adjusted")).unwrap();
        cell_p(&tab.rows[row][col])
    };
    // Rank 0 (largest difference, A0) uses m_step = 4, matching Tukey's m = 4; rank 3 (smallest difference, A3)
    // uses m_step = 1 (monotone >= rank 2), strictly smaller than Tukey's single-step m = 4 adjustment.
    assert_approx(get_adj(tab_snk, 0), get_adj(tab_tuk, 0), 1e-12, 1e-10, "SNK rank 0 equals m=4 step");
    assert!(get_adj(tab_snk, 3) < get_adj(tab_tuk, 3) - 0.05, "SNK smallest effect uses step-down m_step=1, not m=4");
}

// ============================================================================
// RV-72: Factorial / REML Dunnett exact contrast covariance & SMM fallback note
// ============================================================================

#[test]
fn rv72_dunnett_uses_exact_contrast_covariance_on_missing_cell_reml_and_emits_fallback_note_when_non_factorizable() {
    // 1. Split-plot REML (`ColumnFactorMatched` with uneven missingness) on a 3x2 design where cell (A2, B1)
    // is completely missing: individual marginal mean for A2 (`0.5*(mu_{20} + mu_{21})`) is non-estimable,
    // while `cov_vectors` carries the exact REML contrast vectors `(l, Phi_A l)`.
    // With 8 control subjects vs 2 in Trt1 and 2 in Trt2 (at T0 only), exact rho_12 << 0.5;
    // verify Dunnett uses the exact contrast correlation rather than balanced `1/sqrt(2)` loadings.
    let mut grouped_reml = GroupedTable::new(SubcolumnMode::RawReplicates(8), 2, 3).unwrap();
    grouped_reml.row_titles = vec![String::from("Ctrl"), String::from("Trt1"), String::from("Trt2")];
    grouped_reml.data_sets[0].title = String::from("T0");
    grouped_reml.data_sets[1].title = String::from("T1");

    for (s, &(y0, y1)) in
        [(10.0, 12.0), (10.5, 12.8), (9.6, 11.4), (10.2, 12.1), (9.9, 11.9), (10.3, 12.3), (9.7, 11.6), (10.1, 12.0)].iter().enumerate()
    {
        grouped_reml.data_sets[0].cells[0][s] = Cell::number(y0);
        grouped_reml.data_sets[1].cells[0][s] = Cell::number(y1);
    }
    for (s, &(y0, y1)) in [(11.3, 13.4), (10.7, 12.7)].iter().enumerate() {
        grouped_reml.data_sets[0].cells[1][s] = Cell::number(y0);
        grouped_reml.data_sets[1].cells[1][s] = Cell::number(y1);
    }
    for (s, &y0) in [11.2, 10.6].iter().enumerate() {
        grouped_reml.data_sets[0].cells[2][s] = Cell::number(y0);
    }

    let sheet_reml = DataSheet::new("g-reml-dun", "REML Missing Cell Dunnett", TableArchetype::Grouped(grouped_reml)).unwrap();
    let reml_dun_spec = FactorialAnovaSpec {
        repeated_measures: RepeatedMeasuresDesign::ColumnFactorMatched,
        residual_covariance: ResidualCovariance::Unstructured,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 0 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..FactorialAnovaSpec::default()
    };
    let an_reml = AnalysisSpec::new("an-reml-dun", "g-reml-dun", AnalysisKind::FactorialAnova(reml_dun_spec.clone())).unwrap();
    let res_reml = run_factorial_anova(&sheet_reml, &an_reml, &reml_dun_spec).unwrap();
    let tab_reml = res_reml.tabs.iter().find(|t| t.id == "multiple-comparisons-factor-a").unwrap();
    assert_eq!(tab_reml.rows.len(), 2);

    let stat_col = tab_reml.column_headers.iter().position(|h| h == "q" || h.contains("Statistic") || h.contains("t / q")).unwrap();
    let df_col = tab_reml.column_headers.iter().position(|h| h.eq_ignore_ascii_case("df")).unwrap();
    let adj_col = tab_reml.column_headers.iter().position(|h| h.contains("Adjusted")).unwrap();

    let t0 = cell_num(&tab_reml.rows[0][stat_col]);
    let df0 = cell_num(&tab_reml.rows[0][df_col]);
    let df1 = cell_num(&tab_reml.rows[1][df_col]);
    let adj_p0 = cell_p(&tab_reml.rows[0][adj_col]);
    let df_joint = df0.min(df1);
    let balanced_loadings = [std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];
    let p_if_naive_balanced = dunnett_two_sided_p_loadings(t0.abs(), df_joint, &balanced_loadings).unwrap();
    assert!(
        (adj_p0 - p_if_naive_balanced).abs() > 1e-4,
        "unbalanced missing-cell REML Dunnett must use exact contrast correlation, not balanced 1/sqrt(2): actual={adj_p0}, naive={p_if_naive_balanced}"
    );

    // 2. Non-factorizable 3-contrast correlation matrix on a 4x2 OLS table with disjoint control subsets:
    // - A0 (control) observed on both B0 and B1
    // - A1 observed ONLY on B0 -> L1 = mu_{10} - mu_{00}
    // - A2 observed ONLY on B1 -> L2 = mu_{21} - mu_{01} (uncorrelated with L1: rho_{12} = 0)
    // - A3 observed on BOTH B0 and B1 -> L3 = 0.5*(mu_{30}+mu_{31}) - 0.5*(mu_{00}+mu_{01}) (rho_{13} > 0 and rho_{23} > 0)
    // No single-factor loadings lambda_1..lambda_3 exist; `compare_contrast_dunnett` must fall back
    // to conservative Studentized Maximum Modulus and emit the diagnostic footnote.
    let mut grouped_nonfac = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 4).unwrap();
    grouped_nonfac.row_titles = vec![String::from("A0"), String::from("A1"), String::from("A2"), String::from("A3")];
    for s in 0..3 {
        let jitter = (s as f64 - 1.0) * 0.4;
        grouped_nonfac.data_sets[0].cells[0][s] = Cell::number(10.0 + jitter);
        grouped_nonfac.data_sets[1].cells[0][s] = Cell::number(11.0 + jitter);
        grouped_nonfac.data_sets[0].cells[1][s] = Cell::number(14.0 + jitter);
        grouped_nonfac.data_sets[1].cells[2][s] = Cell::number(15.5 + jitter);
        grouped_nonfac.data_sets[0].cells[3][s] = Cell::number(13.0 + jitter);
        grouped_nonfac.data_sets[1].cells[3][s] = Cell::number(14.0 + jitter);
    }
    let sheet_nonfac = DataSheet::new("g-nonfac", "Non-factorizable Dunnett", TableArchetype::Grouped(grouped_nonfac)).unwrap();
    let ols_dun_spec = FactorialAnovaSpec {
        ss_type: SumOfSquaresType::TypeIII,
        multiplicity: MultiplicitySpec {
            method: MultiplicityMethod::Dunnett { control_column: 0 },
            threshold: 0.05,
            report_confidence_intervals: true,
        },
        ..FactorialAnovaSpec::default()
    };
    let an_nonfac = AnalysisSpec::new("an-nonfac", "g-nonfac", AnalysisKind::FactorialAnova(ols_dun_spec.clone())).unwrap();
    let res_nonfac = run_factorial_anova(&sheet_nonfac, &an_nonfac, &ols_dun_spec).unwrap();
    let tab_nonfac = res_nonfac.tabs.iter().find(|t| t.id == "multiple-comparisons-factor-a").unwrap();
    assert!(
        tab_nonfac.footnotes.iter().any(|n| n.contains("Studentized Maximum Modulus")),
        "non-factorizable Dunnett correlation must emit explicit SMM fallback footnote: {:?}",
        tab_nonfac.footnotes
    );

    let se_col = tab_nonfac.column_headers.iter().position(|h| h.contains("SE")).unwrap();
    let ci_col = tab_nonfac.column_headers.iter().position(|h| h.contains("CI")).unwrap();
    let nf_stat_col = tab_nonfac.column_headers.iter().position(|h| h == "q" || h.contains("Statistic") || h.contains("t / q")).unwrap();
    let nf_df_col = tab_nonfac.column_headers.iter().position(|h| h.eq_ignore_ascii_case("df")).unwrap();
    let nf_raw_col = tab_nonfac.column_headers.iter().position(|h| h.contains("Raw")).unwrap();
    let nf_adj_col = tab_nonfac.column_headers.iter().position(|h| h.contains("Adjusted")).unwrap();

    let t_nf = cell_num(&tab_nonfac.rows[0][nf_stat_col]);
    let df_nf = cell_num(&tab_nonfac.rows[0][nf_df_col]);
    let raw_nf = cell_p(&tab_nonfac.rows[0][nf_raw_col]);
    let adj_nf = cell_p(&tab_nonfac.rows[0][nf_adj_col]);
    let se_nf = cell_num(&tab_nonfac.rows[0][se_col]);
    let (ci_lo, ci_hi) = match tab_nonfac.rows[0][ci_col] {
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(lo), upper: CiBound::Finite(hi) } => Some((lo, hi)),
        _ => None,
    }
    .expect("expected finite ConfidenceInterval CI");
    assert_approx(raw_nf, student_t_two_sided_p(t_nf, df_nf).unwrap(), 1e-12, 1e-10, "SMM fallback raw p");
    let expected_smm_p = max_studentized_range_sf(std::f64::consts::SQRT_2 * t_nf.abs(), 3, 2, df_nf).unwrap();
    assert_approx(adj_nf, expected_smm_p, 1e-12, 1e-10, "SMM fallback adjusted p");
    let smm_crit = max_studentized_range_isf(0.05, 3, 2, df_nf).unwrap() * std::f64::consts::FRAC_1_SQRT_2;
    let unadj_crit = student_t_two_sided_isf(0.05, df_nf).unwrap();
    assert!(smm_crit > unadj_crit);
    assert_approx(0.5 * (ci_hi - ci_lo), smm_crit * se_nf, 1e-12, 1e-10, "SMM fallback CI half-width");
}

// ============================================================================
// RV-91: three_way = true rejects unsupported Grouped/MV layouts cleanly
// ============================================================================

#[test]
fn rv91_three_way_rejects_unsupported_column_counts_and_preserves_2x2xk_rm_reml() {
    for n_cols in [3_usize, 5, 6] {
        let mut g = GroupedTable::new(SubcolumnMode::RawReplicates(2), n_cols, 2).unwrap();
        for c in 0..n_cols {
            for r in 0..2 {
                for s in 0..2 {
                    g.data_sets[c].cells[r][s] = Cell::number(10.0 + (c as f64) + (r as f64) * 2.0 + (s as f64) * 0.3);
                }
            }
        }
        let sheet = DataSheet::new("g-bad-3w", "Unsupported 3-way", TableArchetype::Grouped(g)).unwrap();
        for rm in [RepeatedMeasuresDesign::None, RepeatedMeasuresDesign::BothFactorsMatched] {
            let fac = FactorialAnovaSpec { three_way: true, max_interaction_order: 3, repeated_measures: rm, ..FactorialAnovaSpec::default() };
            let spec = AnalysisSpec::new("an-bad-3w", "g-bad-3w", AnalysisKind::FactorialAnova(fac.clone())).unwrap();
            let err = run_factorial_anova(&sheet, &spec, &fac).expect_err("non-4-col Grouped three_way must fail");
            assert!(
                matches!(err, StatsError::InvalidParameter { name: "three_way_column_count", value } if (value - n_cols as f64).abs() < 1e-12),
                "unexpected error for n_cols={n_cols}: {err:?}"
            );
        }
    }

    // MultipleVariables with three_way = true and != 3 multifactor_columns also returns InvalidParameter.
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].role = VariableRole::Continuous;
    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::text(if r < 3 { "A1" } else { "A2" });
        mv.variables[1].cells[r] = Cell::text(if r % 2 == 0 { "B1" } else { "B2" });
        mv.variables[2].cells[r] = Cell::number(10.0 + r as f64);
    }
    let mv_sheet = DataSheet::new("mv-bad-3w", "MV 2-factor", TableArchetype::MultipleVariables(mv)).unwrap();
    let mv_fac = FactorialAnovaSpec {
        three_way: true,
        max_interaction_order: 3,
        response_column: Some(2),
        multifactor_columns: vec![0, 1],
        ..FactorialAnovaSpec::default()
    };
    let mv_spec = AnalysisSpec::new("an-mv-bad-3w", "mv-bad-3w", AnalysisKind::FactorialAnova(mv_fac.clone())).unwrap();
    let mv_err = run_factorial_anova(&mv_sheet, &mv_spec, &mv_fac).expect_err("MV three_way with 2 factors must fail");
    assert!(matches!(mv_err, StatsError::InvalidParameter { name: "three_way_factor_count", .. }), "unexpected error: {mv_err:?}");

    // Positive preservation check: 4-column 2x2x2 RM mixed-effects REML still succeeds.
    let mut g4 = GroupedTable::new(SubcolumnMode::RawReplicates(4), 4, 2).unwrap();
    g4.three_way_grouping = Some(ThreeWayColumnGrouping {
        factor_1_name: String::from("Drug"),
        factor_1_levels: [String::from("Veh"), String::from("Active")],
        factor_2_name: String::from("Sex"),
        factor_2_levels: [String::from("F"), String::from("M")],
    });
    for c in 0..4 {
        for r in 0..2 {
            for s in 0..4 {
                if !(c == 3 && r == 1 && s == 3) {
                    g4.data_sets[c].cells[r][s] = Cell::number(12.0 + (c as f64) * 2.2 + (r as f64) * 3.1 + (s as f64) * 0.7);
                }
            }
        }
    }
    let sheet4 = DataSheet::new("g-ok-3w", "Valid 3-way RM", TableArchetype::Grouped(g4)).unwrap();
    let fac4 = FactorialAnovaSpec {
        three_way: true,
        max_interaction_order: 3,
        repeated_measures: RepeatedMeasuresDesign::BothFactorsMatched,
        df_method: MixedModelDfMethod::KenwardRoger,
        ..FactorialAnovaSpec::default()
    };
    let spec4 = AnalysisSpec::new("an-ok-3w", "g-ok-3w", AnalysisKind::FactorialAnova(fac4.clone())).unwrap();
    let res4 = run_factorial_anova(&sheet4, &spec4, &fac4).expect("2x2x2 3-way RM REML must remain supported");
    assert!(
        res4.tabs.iter().any(|t| t.id == "mixed-reml"),
        "2x2x2 3-way RM REML should emit variance components tab: {:?}",
        res4.tabs.iter().map(|t| (&t.id, &t.title)).collect::<Vec<_>>()
    );
    let anova_tab = res4.tabs.iter().find(|t| t.id == "factorial-anova").expect("3-way RM REML should emit fixed-effects ANOVA tab");
    assert_eq!(anova_tab.title, "Three-way ANOVA");
}

// ============================================================================
// RV-92: Linear-trend tab "R squared" column reports genuine R^2 in [0, 1]
// ============================================================================

#[test]
fn rv92_linear_trend_tab_reports_r_squared_in_unit_interval_not_raw_ss_trend() {
    // 1. One-way ANOVA with large SS_trend (> 400) so raw ss_trend would exceed 1.0 blatantly.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    let groups_data = [[10.0, 12.0, 11.0, 13.0], [25.0, 27.0, 26.0, 28.0], [42.0, 40.0, 43.0, 41.0]];
    for (c, vals) in groups_data.iter().enumerate() {
        col.data_sets[c].title = format!("Dose{}", c + 1);
        for (r, &v) in vals.iter().enumerate() {
            col.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let sheet = DataSheet::new("ow-lt", "OneWay LinearTrend", TableArchetype::Column(col)).unwrap();
    let ow_spec = OneWayAnovaSpec {
        method: OneWayMethod::OrdinaryAnova,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
        ..OneWayAnovaSpec::default()
    };
    let res_ow =
        run_one_way_anova(&sheet, &AnalysisSpec::new("a-ow", "ow-lt", AnalysisKind::OneWayAnova(ow_spec.clone())).unwrap(), &ow_spec).unwrap();
    let lt_tab = res_ow.tabs.iter().find(|t| t.id == "linear-trend").expect("linear-trend tab");
    let r2_col = lt_tab.column_headers.iter().position(|h| h == "R squared").expect("R squared column");
    let r2_ow = cell_num(&lt_tab.rows[0][r2_col]);
    let inputs: Vec<GroupComparisonInput> = groups_data
        .iter()
        .enumerate()
        .map(|(i, vals)| {
            let mean = vals.iter().sum::<f64>() / (vals.len() as f64);
            let var = vals.iter().map(|&v| (v - mean) * (v - mean)).sum::<f64>() / ((vals.len() - 1) as f64);
            GroupComparisonInput { index: i, label: format!("Dose{}", i + 1), n: vals.len() as f64, mean, sd: var.sqrt() }
        })
        .collect();
    let anova_tab = res_ow.tabs.iter().find(|t| t.id == "anova-table").unwrap();
    let ss_total = cell_num(&anova_tab.rows[2][0]);
    let ms_within = cell_num(&anova_tab.rows[1][2]);
    let df_within = cell_num(&anova_tab.rows[1][1]);
    let direct_trend = linear_trend_test(&inputs, ms_within, df_within).unwrap();
    assert!(direct_trend.ss_trend > 100.0, "fixture must have large raw ss_trend={}", direct_trend.ss_trend);
    assert!((0.0..=1.0).contains(&r2_ow), "one-way linear-trend R squared must be in [0, 1], got {r2_ow}");
    assert_approx(r2_ow, direct_trend.ss_trend / ss_total, 1e-12, 1e-12, "one-way linear-trend R2 == SS_trend/SS_total");
    assert_approx(direct_trend.r_squared, r2_ow, 1e-12, 1e-12, "LinearTrendResult.r_squared matches one-way SS_trend/SS_total");

    // 2. RM One-way ANOVA linear-trend tab also reports genuine SS_trend / SS_total in [0, 1].
    let mut rm_col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 4).unwrap();
    for (c, vals) in groups_data.iter().enumerate() {
        for (r, &v) in vals.iter().enumerate() {
            rm_col.data_sets[c].cells[r][0] = Cell::number(v + (r as f64) * 3.0);
        }
    }
    let rm_sheet = DataSheet::new("rm-lt", "RM LinearTrend", TableArchetype::Column(rm_col)).unwrap();
    let rm_spec = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false },
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
        ..OneWayAnovaSpec::default()
    };
    let res_rm =
        run_one_way_anova(&rm_sheet, &AnalysisSpec::new("a-rm", "rm-lt", AnalysisKind::OneWayAnova(rm_spec.clone())).unwrap(), &rm_spec).unwrap();
    let rm_lt_tab = res_rm.tabs.iter().find(|t| t.id == "linear-trend").expect("RM linear-trend tab");
    let rm_r2_col = rm_lt_tab.column_headers.iter().position(|h| h == "R squared").expect("R squared column");
    let rm_r2 = cell_num(&rm_lt_tab.rows[0][rm_r2_col]);
    assert!((0.0..=1.0).contains(&rm_r2), "RM linear-trend R squared must be in [0, 1], got {rm_r2}");

    // 3. Nested ANOVA linear-trend tab also reports genuine R^2 in [0, 1], not raw F / ss_trend (> 1.0).
    let mut nt = NestedTable::new(3, 3, 3).unwrap();
    let nested_vals = [
        [[10.0, 10.4, 9.8], [10.6, 10.2, 10.5], [9.9, 10.3, 10.1]],
        [[20.0, 20.3, 19.7], [20.5, 20.1, 20.4], [19.8, 20.2, 20.0]],
        [[30.0, 30.2, 29.9], [30.6, 30.1, 30.4], [29.7, 30.3, 30.0]],
    ];
    for (g, cl) in nested_vals.iter().enumerate() {
        for (b, reps) in cl.iter().enumerate() {
            for (r, &v) in reps.iter().enumerate() {
                nt.groups[g].cells[r][b] = Cell::number(v);
            }
        }
    }
    let nt_sheet = DataSheet::new("nt-lt", "Nested LinearTrend", TableArchetype::Nested(nt)).unwrap();
    let nt_spec = NestedAnalysisSpec {
        two_group_t_test: false,
        df_method: MixedModelDfMethod::KenwardRoger,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::LinearTrend, threshold: 0.05, report_confidence_intervals: false },
    };
    let res_nt =
        run_nested_analysis(&nt_sheet, &AnalysisSpec::new("a-nt", "nt-lt", AnalysisKind::NestedComparison(nt_spec.clone())).unwrap(), &nt_spec)
            .unwrap();
    let nt_lt_tab = res_nt.tabs.iter().find(|t| t.id == "linear-trend").expect("Nested linear-trend tab");
    let nt_r2_col = nt_lt_tab.column_headers.iter().position(|h| h == "R squared").expect("R squared column");
    let nt_f_col = nt_lt_tab.column_headers.iter().position(|h| h == "F").expect("F column");
    let nt_r2 = cell_num(&nt_lt_tab.rows[0][nt_r2_col]);
    let nt_f = cell_num(&nt_lt_tab.rows[0][nt_f_col]);
    assert!(nt_f > 50.0, "fixture must produce strong trend F={nt_f}");
    assert!((0.0..=1.0).contains(&nt_r2), "nested linear-trend R squared must be in [0, 1], got {nt_r2}");
    assert!(nt_r2 > 0.9, "strong nested linear trend should have high R^2, got {nt_r2}");
}
