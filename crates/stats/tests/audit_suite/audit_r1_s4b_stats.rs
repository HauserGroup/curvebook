//! Stage 4B regression tests for `curvebook-stats` findings (`R1-S02-12`, `R1-S02-13`, `R1-X2-08`,
//! `R1-X2-09`, and all 5 S3 title-only findings from Round 1 S02, archived in git history).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ChecklistStatus, CiBound, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec,
    ContingencyTable, CoxTieMethod, DataSheet, DescriptiveSpec, GroupedTable, KappaWeighting, MultipleVariablesTable, MultiplicityMethod,
    MultiplicitySpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec, ResultCell, ResultSheet, SheetId, SubcolumnMode, SurvivalAnalysisSpec,
    SurvivalTable, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::norm_sf;
use curvebook_stats::categorical::{baptista_pike_or_ci, cohens_kappa, fleiss_kappa};
use curvebook_stats::multiplicity::{GroupComparisonInput, MultiplicityProcedure, compare_groups_with_procedure};
use curvebook_stats::two_group::{bland_altman, mann_whitney_u, wilcoxon_matched_pairs};
use curvebook_stats::{StatsError, run_analysis};

fn assert_all_sheet_numbers_finite(sheet: &ResultSheet, label: &str) {
    for s in &sheet.scalars {
        assert!(s.value.is_finite(), "{label}: non-finite scalar {} = {}", s.key, s.value);
    }
    for tab in &sheet.tabs {
        for (r_idx, row) in tab.rows.iter().enumerate() {
            for (c_idx, cell) in row.iter().enumerate() {
                match cell {
                    ResultCell::Number { value, .. } => {
                        assert!(value.is_finite(), "{label}: non-finite tab cell ({r_idx},{c_idx}) = {value}");
                    }
                    ResultCell::PValue { raw, .. } => {
                        assert!(raw.is_finite(), "{label}: non-finite p-value at ({r_idx},{c_idx}) = {raw}");
                    }
                    ResultCell::ConfidenceInterval { lower, upper } => {
                        if let CiBound::Finite(v) = lower {
                            assert!(v.is_finite(), "{label}: non-finite CI lower at ({r_idx},{c_idx}) = {v}");
                        }
                        if let CiBound::Finite(v) = upper {
                            assert!(v.is_finite(), "{label}: non-finite CI upper at ({r_idx},{c_idx}) = {v}");
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for rec in &sheet.pairwise_comparisons {
        if let Some(d) = rec.estimate_diff {
            assert!(d.is_finite(), "{label}: non-finite pairwise diff = {d}");
        }
        assert!(rec.raw_p_or_q.is_finite(), "{label}: non-finite pairwise p = {}", rec.raw_p_or_q);
    }
    for curve in &sheet.fitted_curves {
        for pt in &curve.points {
            assert!(pt.x.is_finite() && pt.y.is_finite(), "{label}: non-finite curve pt ({}, {})", pt.x, pt.y);
            assert!(pt.ci_lower.is_none_or(f64::is_finite), "{label}: non-finite ci_lower");
            assert!(pt.ci_upper.is_none_or(f64::is_finite), "{label}: non-finite ci_upper");
            assert!(pt.pred_lower.is_none_or(f64::is_finite), "{label}: non-finite pred_lower");
            assert!(pt.pred_upper.is_none_or(f64::is_finite), "{label}: non-finite pred_upper");
        }
    }
}

fn scalar_val(sheet: &ResultSheet, key: &str, ds_idx: Option<usize>) -> Option<f64> {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index == ds_idx).map(|s| s.value)
}

fn has_warn_ignored_text(sheet: &ResultSheet) -> bool {
    let cl = sheet.checklist.iter().any(|c| {
        c.status == ChecklistStatus::Warn && c.assumption == "Numeric input validation" && c.explanation.contains("Non-numeric values ignored:")
    });
    let fn_note = sheet.tabs.first().is_some_and(|t| t.footnotes.iter().any(|f| f.contains("Non-numeric values ignored:")));
    cl && fn_note
}

fn has_info_listwise(sheet: &ResultSheet) -> bool {
    let cl = sheet.checklist.iter().any(|c| c.status == ChecklistStatus::Info && c.assumption == "Complete-case row alignment");
    let fn_note = sheet.tabs.first().is_some_and(|t| t.footnotes.iter().any(|f| f.contains("complete-case listwise alignment")));
    cl && fn_note
}

fn enumerate_two_sided_tie_mw_p(ranks: &[f64], na: usize, obs_sum_a: f64) -> f64 {
    let n = ranks.len();
    let total_sum: f64 = ranks.iter().sum();
    let expected = (na as f64) * total_sum / (n as f64);
    let diff_obs = (obs_sum_a - expected).abs();
    let mut total = 0_u64;
    let mut le_lower = 0_u64;
    let mut ge_upper = 0_u64;
    let mut tail_sym = 0_u64;
    for mask in 0_u32..(1_u32 << n) {
        if mask.count_ones() as usize != na {
            continue;
        }
        total += 1;
        let sum_a: f64 = ranks.iter().enumerate().filter(|&(i, _)| (mask & (1_u32 << i)) != 0).map(|(_, &r)| r).sum();
        if sum_a <= obs_sum_a + 1e-9 {
            le_lower += 1;
        }
        if sum_a + 1e-9 >= obs_sum_a {
            ge_upper += 1;
        }
        if (sum_a - expected).abs() + 1e-9 >= diff_obs {
            tail_sym += 1;
        }
    }
    let _ = tail_sym;
    // Matches DP minimum doubled tail rule used in `mann_whitney_exact_ties_p`:
    (2.0 * (le_lower.min(ge_upper) as f64) / (total as f64)).clamp(0.0, 1.0)
}

#[test]
fn r1_s02_12_mann_whitney_exact_ties_permutation_dp() {
    let a = [1.0, 2.0, 2.0, 4.0, 6.0];
    let b = [2.0, 3.0, 5.0, 6.0, 6.0];
    let mw = mann_whitney_u(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(mw.exact_p_used, "n_a + n_b = 10 with ties must use conditional permutation DP");

    // Independent brute-force over all C(10, 5) = 252 combinations of midranks.
    let combined = [1.0, 2.0, 2.0, 4.0, 6.0, 2.0, 3.0, 5.0, 6.0, 6.0];
    let mut indexed: Vec<(usize, f64)> = combined.iter().copied().enumerate().collect();
    indexed.sort_by(|x, y| x.1.total_cmp(&y.1));
    let mut ranks = vec![0.0; combined.len()];
    let mut pos = 0;
    while pos < indexed.len() {
        let mut end = pos + 1;
        while end < indexed.len() && indexed[end].1 == indexed[pos].1 {
            end += 1;
        }
        let mid = 0.5 * ((pos + 1) as f64 + (end as f64));
        for k in pos..end {
            ranks[indexed[k].0] = mid;
        }
        pos = end;
    }
    let obs_sum_a: f64 = ranks[..a.len()].iter().sum();
    let expected_exact_p = enumerate_two_sided_tie_mw_p(&ranks, a.len(), obs_sum_a);
    assert!((mw.p_value - expected_exact_p).abs() < 1e-12, "DP p={} vs brute={expected_exact_p}", mw.p_value);

    // Group-order invariance under two-tailed test.
    let mw_swap = mann_whitney_u(&b, &a, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!((mw.p_value - mw_swap.p_value).abs() < 1e-12);

    // One-tailed matches exact single tail.
    let mw_one = mann_whitney_u(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(mw_one.exact_p_used);
    assert!((2.0 * mw_one.p_value - mw.p_value).abs() < 1e-12);

    // Sheet checklist acknowledges exact permutation with ties for N <= 18, and normal approx for N = 19.
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    for (i, (&va, &vb)) in a.iter().zip(b.iter()).enumerate() {
        tab.data_sets[0].cells[i][0] = Cell::number(va);
        tab.data_sets[1].cells[i][0] = Cell::number(vb);
    }
    let sheet = DataSheet::new(SheetId::new("mw-ties"), "MW Ties", TableArchetype::Column(tab)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("a-mw"),
        sheet.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::MannWhitneyU, ..Default::default() }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert!(res.checklist.iter().any(|c| c.explanation.contains("Exact permutation DP")));

    let large_a: Vec<f64> = (0..10).map(|i| (i % 4) as f64).collect();
    let large_b: Vec<f64> = (0..9).map(|i| ((i % 4) + 1) as f64).collect();
    let mw_large = mann_whitney_u(&large_a, &large_b, true, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(!mw_large.exact_p_used);
}

#[test]
fn r1_s02_12_hodges_lehmann_exact_above_legacy_cutoffs() {
    // 1. Two-sample Hodges-Lehmann with n_a * n_b = 40_200 > 40_000 (legacy fallback cutoff).
    let a: Vec<f64> = (0..201).map(|i| (i * i) as f64).collect();
    let b: Vec<f64> = (0..200).map(|j| (3 * j) as f64).collect();
    let mw = mann_whitney_u(&a, &b, true, ConfidenceLevel::DEFAULT_95).unwrap();

    let mut exact_diffs = Vec::with_capacity(a.len() * b.len());
    for &va in &a {
        for &vb in &b {
            exact_diffs.push(va - vb);
        }
    }
    exact_diffs.sort_by(f64::total_cmp);
    let m = exact_diffs.len();
    let exact_hl = 0.5 * (exact_diffs[(m - 1) / 2] + exact_diffs[m / 2]);
    assert!(
        (mw.hodges_lehmann_diff - exact_hl).abs() < 1e-12,
        "exact two-sample HL {} differed from true pairwise median {} (median_a-median_b would be {})",
        mw.hodges_lehmann_diff,
        exact_hl,
        mw.median_a - mw.median_b
    );
    assert!(mw.hodges_lehmann_ci.is_some());
    let (ci_lo, ci_hi) = mw.hodges_lehmann_ci.unwrap();
    assert!(ci_lo < mw.hodges_lehmann_diff && mw.hodges_lehmann_diff < ci_hi);

    // 2. One-sample Hodges-Lehmann pseudomedian with n = 2_005 > 2_000 (legacy cutoff).
    let n = 2_005;
    let x: Vec<f64> = (0..n).map(|i| ((i as f64) / ((n - 1) as f64)).powi(3) * 100.0).collect();
    let zeros = vec![0.0; n];
    let wil = wilcoxon_matched_pairs(&x, &zeros, true, ConfidenceLevel::DEFAULT_95, true).unwrap();

    let mut walsh = Vec::with_capacity(n * (n + 1) / 2);
    for i in 0..n {
        for j in i..n {
            walsh.push(0.5 * x[i] + 0.5 * x[j]);
        }
    }
    walsh.sort_by(f64::total_cmp);
    let mw_tot = walsh.len();
    let exact_walsh_med = 0.5 * (walsh[(mw_tot - 1) / 2] + walsh[mw_tot / 2]);
    assert!(
        (wil.hodges_lehmann_pseudomedian - exact_walsh_med).abs() < 1e-12,
        "exact one-sample HL {} differed from Walsh median {} (raw sample median is 12.5)",
        wil.hodges_lehmann_pseudomedian,
        exact_walsh_med
    );
    assert!(wil.hodges_lehmann_ci.is_some());
}

#[test]
fn r1_s02_13_rm_anova_friedman_cox_mv_km_report_listwise_omissions() {
    // ColumnTable with 6 allocated rows: rows 0, 1, 3 complete; row 2 has excluded cell; row 4 is partial; row 5 is trailing blank.
    let mut tab = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 6).unwrap();
    let complete = [(0_usize, [10.0, 12.0, 15.0]), (1, [11.0, 14.0, 17.0]), (3, [12.0, 15.0, 19.0])];
    for (r, vals) in complete {
        for c in 0..3 {
            tab.data_sets[c].cells[r][0] = Cell::number(vals[c]);
        }
    }
    tab.data_sets[0].cells[2][0] = Cell::number(20.0);
    let mut excl = Cell::number(22.0);
    excl.excluded = true;
    tab.data_sets[1].cells[2][0] = excl;
    tab.data_sets[2].cells[2][0] = Cell::number(25.0);

    tab.data_sets[0].cells[4][0] = Cell::number(30.0);
    tab.data_sets[1].cells[4][0] = Cell::number(31.0);
    // Row 5 remains trailing empty -> must not count as skipped!

    let col_sheet = DataSheet::new(SheetId::new("rm-listwise"), "RM Listwise", TableArchetype::Column(tab)).unwrap();
    let rm_reml_spec = AnalysisSpec::new(
        SheetId::new("a-rm-reml"),
        col_sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
    )
    .unwrap();
    let rm_reml_res = run_analysis(&col_sheet, &rm_reml_spec).unwrap();
    assert!(!has_info_listwise(&rm_reml_res), "RepeatedMeasures with missing cells must fit mixed-effects REML instead of listwise deletion");
    assert!(rm_reml_res.checklist.iter().any(|c| c.assumption == "Mixed-effects REML (repeated measures)"
        && c.diagnostic_metric.as_deref().is_some_and(|m| m.starts_with("5 subjects, 13 observations, 2 incomplete block(s)"))));

    let fr_spec = AnalysisSpec::new(
        SheetId::new("a-fr"),
        col_sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..Default::default() }),
    )
    .unwrap();
    let fr_res = run_analysis(&col_sheet, &fr_spec).unwrap();
    assert!(has_info_listwise(&fr_res), "Friedman should report Complete-case row alignment");
    assert!(fr_res.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 3 retained")));

    // MultipleVariables Cox PH & MV Kaplan-Meier omitting incomplete rows.
    let mut mv = MultipleVariablesTable::new(4, 7).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Group");
    mv.variables[2].role = VariableRole::Categorical;
    mv.variables[3].title = String::from("Biomarker");
    let cox_rows = [
        (0, Some(5.0), Some(1.0), "A", Some(1.2)),
        (1, Some(8.0), Some(0.0), "A", Some(0.9)),
        (2, Some(10.0), Some(1.0), "B", None), // missing biomarker -> omitted
        (3, Some(12.0), Some(1.0), "B", Some(2.1)),
        (4, None, Some(1.0), "A", Some(1.8)), // missing time -> omitted
        (5, Some(15.0), Some(1.0), "B", Some(2.5)),
        (6, Some(20.0), Some(0.0), "A", Some(0.7)),
    ];
    for (r, t, s, g, x) in cox_rows {
        if let Some(tv) = t {
            mv.variables[0].cells[r] = Cell::number(tv);
        }
        if let Some(sv) = s {
            mv.variables[1].cells[r] = Cell::number(sv);
        }
        mv.variables[2].cells[r] = Cell::text(g);
        if let Some(xv) = x {
            mv.variables[3].cells[r] = Cell::number(xv);
        }
    }
    let mv_sheet = DataSheet::new(SheetId::new("mv-surv"), "MV Surv", TableArchetype::MultipleVariables(mv)).unwrap();

    let cox_spec = AnalysisSpec::new(
        SheetId::new("a-cox"),
        mv_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: vec![(2, String::from("A"))],
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    let cox_res = run_analysis(&mv_sheet, &cox_spec).unwrap();
    assert!(has_info_listwise(&cox_res));
    assert!(cox_res.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("2 row(s) omitted; 5 retained")));

    let km_spec = AnalysisSpec::new(
        SheetId::new("a-km"),
        mv_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: Some(2),
        }),
    )
    .unwrap();
    let km_res = run_analysis(&mv_sheet, &km_spec).unwrap();
    assert!(has_info_listwise(&km_res));
    assert!(km_res.checklist.iter().any(|c| c.diagnostic_metric.as_deref() == Some("1 row(s) omitted; 6 retained")));
}

#[test]
fn r1_x2_08_unexcluded_non_numeric_cells_warned_without_false_positive_for_excluded() {
    // 1. Excluded non-numeric text must NOT warn.
    let mut clean_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    for (r, (va, vb)) in [(1.0, 2.0), (1.5, 2.5), (2.0, 3.0), (2.5, 3.5)].into_iter().enumerate() {
        clean_col.data_sets[0].cells[r][0] = Cell::number(va);
        clean_col.data_sets[1].cells[r][0] = Cell::number(vb);
    }
    let mut excl_text = Cell::text("<0.5");
    excl_text.excluded = true;
    clean_col.data_sets[0].cells[4][0] = excl_text;
    clean_col.data_sets[1].cells[4][0] = Cell::number(4.0);
    let clean_sheet = DataSheet::new(SheetId::new("clean-excl"), "CleanExcl", TableArchetype::Column(clean_col)).unwrap();
    let desc_spec =
        AnalysisSpec::new(SheetId::new("a-d0"), clean_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let clean_res = run_analysis(&clean_sheet, &desc_spec).unwrap();
    assert!(!has_warn_ignored_text(&clean_res), "explicitly excluded text cell must not emit ignore warning");

    // 2. Unexcluded `<0.5` in ColumnTable triggers Warn + footnote in DescriptiveStats, Outliers, TwoGroup (Unpaired/Paired), Bland-Altman, OneWayAnova, RM Anova.
    let mut dirty_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).unwrap();
    for r in 0..5 {
        dirty_col.data_sets[0].cells[r][0] = Cell::number(1.0 + 0.5 * (r as f64));
        dirty_col.data_sets[1].cells[r][0] = Cell::number(3.0 + 0.9 * (r as f64) + ((r % 2) as f64) * 0.4);
        dirty_col.data_sets[2].cells[r][0] = Cell::number(6.0 + 1.3 * (r as f64) + (((r + 1) % 2) as f64) * 0.7);
    }
    dirty_col.data_sets[0].cells[2][0] = Cell::text("<0.5");
    dirty_col.data_sets[1].cells[5][0] = Cell::text("BDL");
    let dirty_sheet = DataSheet::new(SheetId::new("dirty-col"), "DirtyCol", TableArchetype::Column(dirty_col)).unwrap();

    for kind in [
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false }),
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..Default::default() }),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() }),
        AnalysisKind::BlandAltman(BlandAltmanSpec::default()),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, ..Default::default() }),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }, ..Default::default() }),
    ] {
        let s = AnalysisSpec::new(SheetId::new("a-x208"), dirty_sheet.id.clone(), kind.clone()).unwrap();
        let res = run_analysis(&dirty_sheet, &s).unwrap();
        assert!(has_warn_ignored_text(&res), "missing Numeric input validation warn for {kind:?}");
    }

    // 3. GroupedTable DescriptiveStats & MultipleVariables DescriptiveStats / Cox / KM also warn.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    for r in 0..3 {
        for c in 0..2 {
            grp.data_sets[c].cells[r][0] = Cell::number((r + c + 1) as f64);
            grp.data_sets[c].cells[r][1] = Cell::number((r + c + 2) as f64);
        }
    }
    grp.data_sets[0].cells[1][1] = Cell::text("nd");
    let grp_sheet = DataSheet::new(SheetId::new("dirty-grp"), "DirtyGrp", TableArchetype::Grouped(grp)).unwrap();
    let grp_spec =
        AnalysisSpec::new(SheetId::new("a-grp"), grp_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    assert!(has_warn_ignored_text(&run_analysis(&grp_sheet, &grp_spec).unwrap()));

    let mut mv = MultipleVariablesTable::new(4, 6).unwrap();
    mv.variables[2].role = VariableRole::Categorical;
    for r in 0..6 {
        mv.variables[0].cells[r] = Cell::number((r + 2) as f64);
        mv.variables[1].cells[r] = Cell::number((r % 2) as f64);
        mv.variables[2].cells[r] = Cell::text(if r < 3 { "Ctrl" } else { "Tx" });
        mv.variables[3].cells[r] = Cell::number((r as f64) * 0.5 + 1.0);
    }
    mv.variables[0].cells[0] = Cell::text("<1.0");
    mv.variables[3].cells[5] = Cell::text("missing");
    let mv_sheet = DataSheet::new(SheetId::new("dirty-mv"), "DirtyMV", TableArchetype::MultipleVariables(mv)).unwrap();
    let mv_desc = AnalysisSpec::new(SheetId::new("a-mvd"), mv_sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    assert!(has_warn_ignored_text(&run_analysis(&mv_sheet, &mv_desc).unwrap()));

    let mv_cox = AnalysisSpec::new(
        SheetId::new("a-mvcox"),
        mv_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: vec![(2, String::from("Ctrl"))],
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(has_warn_ignored_text(&run_analysis(&mv_sheet, &mv_cox).unwrap()));
}

#[test]
fn r1_x2_09_bland_altman_scale_resilient_and_no_nonfinite_sheets() {
    let a = [1.0e300, 1.1e300, 1.2e300];
    let b = [-1.0e300, 1.0e300, 1.5e300];
    let ba_large = bland_altman(&a, &b, false, ConfidenceLevel::DEFAULT_95).unwrap();
    let ba_unit = bland_altman(&[1.0, 1.1, 1.2], &[-1.0, 1.0, 1.5], false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(ba_large.bias.is_finite() && ba_large.sd_bias.is_finite() && ba_large.loa_lower.is_finite() && ba_large.loa_upper.is_finite());
    assert!((ba_large.bias / 1.0e300 - ba_unit.bias).abs() < 1e-12);
    assert!((ba_large.sd_bias / 1.0e300 - ba_unit.sd_bias).abs() < 1e-12);
    for (pt_big, pt_unit) in ba_large.points.iter().zip(ba_unit.points.iter()) {
        assert!(pt_big.0.is_finite() && pt_big.1.is_finite());
        assert!((pt_big.0 / 1.0e300 - pt_unit.0).abs() < 1e-12);
        assert!((pt_big.1 / 1.0e300 - pt_unit.1).abs() < 1e-12);
    }

    // True difference overflow (`1e308 - (-1e308)`) must return Err rather than emitting Inf/NaN.
    let overflow_err = bland_altman(&[1.0e308, 1.0e308], &[-1.0e308, 0.0], false, ConfidenceLevel::DEFAULT_95);
    assert!(overflow_err.is_err());

    // End-to-end Bland-Altman ResultSheet at 1e300 scale has strictly finite outputs everywhere.
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    for i in 0..3 {
        tab.data_sets[0].cells[i][0] = Cell::number(a[i]);
        tab.data_sets[1].cells[i][0] = Cell::number(b[i]);
    }
    let sheet = DataSheet::new(SheetId::new("ba-1e300"), "BA1e300", TableArchetype::Column(tab)).unwrap();
    let ba_spec = AnalysisSpec::new(SheetId::new("a-ba"), sheet.id.clone(), AnalysisKind::BlandAltman(BlandAltmanSpec::default())).unwrap();
    let ba_sheet = run_analysis(&sheet, &ba_spec).unwrap();
    assert_all_sheet_numbers_finite(&ba_sheet, "bland_altman 1e300");
    assert!(scalar_val(&ba_sheet, "Bias", None).is_some());

    // Sibling sheet generators must never emit non-finite values.
    let desc_spec = AnalysisSpec::new(SheetId::new("a-desc"), sheet.id.clone(), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    assert_all_sheet_numbers_finite(&run_analysis(&sheet, &desc_spec).unwrap(), "descriptive 1e300");

    let mut srv = SurvivalTable::new(2, 4).unwrap();
    for (r, t) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
        srv.times[r] = Cell::number(t);
        srv.cohorts[0].cells[r][0] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
        srv.cohorts[1].cells[r][0] = Cell::number(1.0);
    }
    let srv_sheet = DataSheet::new(SheetId::new("srv-fin"), "Srv", TableArchetype::Survival(srv)).unwrap();
    let srv_spec = AnalysisSpec::new(
        SheetId::new("a-srv"),
        srv_sheet.id.clone(),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )
    .unwrap();
    assert_all_sheet_numbers_finite(&run_analysis(&srv_sheet, &srv_spec).unwrap(), "km finite");
}

#[test]
fn s3_title_finding_1_dunnett_missing_control_column_returns_err() {
    let groups = vec![
        GroupComparisonInput { index: 0, label: String::from("G0"), n: 5.0, mean: 10.0, sd: 1.5 },
        GroupComparisonInput { index: 1, label: String::from("G1"), n: 5.0, mean: 14.0, sd: 1.5 },
    ];
    let err = compare_groups_with_procedure(&groups, 2.25, 12.0, MultiplicityProcedure::Dunnett { control_column: 2 }, 0.05, true).unwrap_err();
    assert!(matches!(err, StatsError::InvalidParameter { name: "control_column", .. }), "got {err:?}");

    // Also via `run_analysis` when `selected_data_sets` excludes `control_column`.
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).unwrap();
    for r in 0..4 {
        tab.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        tab.data_sets[1].cells[r][0] = Cell::number((r + 5) as f64);
        tab.data_sets[2].cells[r][0] = Cell::number((r + 9) as f64);
    }
    let sheet = DataSheet::new(SheetId::new("dun-err"), "DunErr", TableArchetype::Column(tab)).unwrap();
    let mut spec = AnalysisSpec::new(
        SheetId::new("a-dun"),
        sheet.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            compute_effect_sizes: false,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Dunnett { control_column: 2 },
                threshold: 0.05,
                report_confidence_intervals: true,
            },
            ..OneWayAnovaSpec::default()
        }),
    )
    .unwrap();
    spec.selected_data_sets = vec![0, 1];
    let res = run_analysis(&sheet, &spec);
    assert!(matches!(res, Err(StatsError::InvalidParameter { name: "control_column", .. })), "got {res:?}");
}

#[test]
fn s3_title_finding_2_baptista_pike_extreme_or_no_raw_bracket_fallback() {
    // Tiny interior OR (`~1e-6`) previously broke or hit raw bracket fallback `or_pt * 2^-40`.
    let (lo, hi) = baptista_pike_or_ci(1, 1000, 1000, 1, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(lo > 0.0 && lo < 1e-6 && hi > 1e-6 && hi < 1.0, "unexpected CI ({lo}, {hi})");

    // Symmetric extreme OR (`[[1, 100], [100, 1]]`) must invert cleanly: 1/hi(100,1,1,100) == lo(1,100,100,1).
    let (lo_inv, hi_inv) = baptista_pike_or_ci(1, 100, 100, 1, ConfidenceLevel::DEFAULT_95).unwrap();
    let (lo_flip, hi_flip) = baptista_pike_or_ci(100, 1, 1, 100, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!((lo_inv * hi_flip - 1.0).abs() < 1e-6 && (hi_inv * lo_flip - 1.0).abs() < 1e-6);

    // Zero cells at exact boundary margins keep `lo = 0` or `hi = inf` without NaN/fallback.
    let (lo_z1, hi_z1) = baptista_pike_or_ci(0, 50, 2, 100, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(lo_z1, 0.0);
    assert!(hi_z1.is_finite() && hi_z1 > 0.0 && hi_z1 < 100.0);
    let (lo_z2, hi_z2) = baptista_pike_or_ci(2, 0, 50, 50, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(lo_z2 > 0.0 && lo_z2.is_finite() && hi_z2.is_infinite());

    // Contingency ResultSheet wraps infinite upper bound cleanly via CiBound without emitting non-finite floats.
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].cells[0][0] = Cell::number(2.0);
    ct.data_sets[1].cells[0][0] = Cell::number(0.0);
    ct.data_sets[0].cells[1][0] = Cell::number(50.0);
    ct.data_sets[1].cells[1][0] = Cell::number(50.0);
    let ct_sheet = DataSheet::new(SheetId::new("bp-ct"), "BP CT", TableArchetype::Contingency(ct)).unwrap();
    let ct_spec = AnalysisSpec::new(
        SheetId::new("a-ct"),
        ct_sheet.id.clone(),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_all_sheet_numbers_finite(&run_analysis(&ct_sheet, &ct_spec).unwrap(), "baptista_pike zero cell sheet");
}

#[test]
fn s3_title_findings_3_and_5_kappa_pe_one_error_and_erfc_p_tail() {
    // Finding #3: when expected agreement p_e == 1.0 (every item placed in one category), kappa is undefined -> Err(ZeroVariance).
    let single_cat = [vec![12, 0, 0], vec![0, 0, 0], vec![0, 0, 0]];
    for w in [KappaWeighting::Unweighted, KappaWeighting::Linear, KappaWeighting::Quadratic] {
        let err = cohens_kappa(&single_cat, w, ConfidenceLevel::DEFAULT_95).unwrap_err();
        assert!(matches!(err, StatsError::ZeroVariance { .. }), "expected ZeroVariance for {w:?}, got {err:?}");
    }
    let fleiss_single = vec![vec![5, 0]; 6];
    let ferr = fleiss_kappa(&fleiss_single, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(ferr, StatsError::ZeroVariance { .. }), "expected ZeroVariance for Fleiss kappa, got {ferr:?}");

    // Dispatch via `run_analysis` also surfaces Err(ZeroVariance) rather than silently returning kappa = 1.0.
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].cells[0][0] = Cell::number(12.0);
    ct.data_sets[1].cells[0][0] = Cell::number(0.0);
    ct.data_sets[0].cells[1][0] = Cell::number(0.0);
    ct.data_sets[1].cells[1][0] = Cell::number(0.0);
    let ct_sheet = DataSheet::new(SheetId::new("k-pe1"), "Kappa pe=1", TableArchetype::Contingency(ct)).unwrap();
    let ck_spec = AnalysisSpec::new(
        SheetId::new("a-ck"),
        ct_sheet.id.clone(),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&ct_sheet, &ck_spec), Err(StatsError::ZeroVariance { .. })));

    // Finding #5: large |z| (e.g., [[100, 0], [0, 100]] -> z = sqrt(200) ~ 14.142) underflows 1 - norm_cdf(z) to 0.0,
    // whereas 2 * norm_sf(z) preserves non-zero exact tail probability (~2.09e-45).
    let strong = [vec![100, 0], vec![0, 100]];
    let kr = cohens_kappa(&strong, KappaWeighting::Unweighted, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(kr.z_statistic > 14.0, "expected z > 14, got {}", kr.z_statistic);
    assert!(kr.p_value > 0.0 && kr.p_value < 1e-40, "expected non-underflowed tiny tail p via erfc, got {}", kr.p_value);
    let expected_p = 2.0 * norm_sf(kr.z_statistic.abs());
    assert!((kr.p_value - expected_p).abs() <= 1e-55);
}

#[test]
fn s3_title_finding_4_wilcoxon_pratt_zeros_no_double_counting() {
    // 14 pairs: 8 zero differences + 6 nonzeros ([1, 2, 2, 3, -4, 5]), giving z_zeros = 8 and a nonzero tie group of t = 2.
    let a = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 21.0, 22.0, 22.0, 23.0, 10.0, 25.0];
    let b = [10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 20.0, 20.0, 20.0, 20.0, 14.0, 20.0];
    let res = wilcoxon_matched_pairs(&a, &b, true, ConfidenceLevel::DEFAULT_95, true).unwrap();
    assert!(!res.exact_p_used);

    // Independent exact Pratt (1959) variance without double-counting zero tie group:
    // N = 14, z = 8, nonzero tie sum (t^3 - t) = (2^3 - 2) = 6.
    let nf = 14.0_f64;
    let zf = 8.0_f64;
    let mean_w = 0.25 * (nf * (nf + 1.0) - zf * (zf + 1.0)); // 0.25 * (210 - 72) = 34.5
    let var_w = (nf * (nf + 1.0) * (2.0 * nf + 1.0) - zf * (zf + 1.0) * (2.0 * zf + 1.0) - 0.5 * 6.0) / 24.0;
    // Abs diffs: 8 zeros (midrank 4.5), 1 (9), 2,2 (10.5), 3 (12), |-4| (13), 5 (14). W+ = 9 + 10.5 + 10.5 + 12 + 14 = 56.0.
    let expected_w_plus = 56.0;
    assert!((res.w_plus - expected_w_plus).abs() < 1e-12);
    let cc = (expected_w_plus - mean_w).abs() - 0.5;
    let expected_p = 2.0 * norm_sf(cc / var_w.sqrt());
    assert!(
        (res.p_value - expected_p).abs() < 1e-12,
        "Pratt p {} differed from exact formula p {expected_p} (double-counting zeros would distort var_w)",
        res.p_value
    );

    // All-zero pairs must cleanly yield p = 1.0 and zero rank-biserial r instead of dividing by zero.
    let all_zero = wilcoxon_matched_pairs(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], true, ConfidenceLevel::DEFAULT_95, true).unwrap();
    assert_eq!(all_zero.p_value, 1.0);
    assert_eq!(all_zero.rank_biserial_r, 0.0);
}
