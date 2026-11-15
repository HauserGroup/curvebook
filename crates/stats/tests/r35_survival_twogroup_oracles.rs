#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::too_many_lines)]
//! R3.5 clean-room D1 reference oracles and defect reproducers for `survival.rs` and `two_group.rs`.
//!
//! Covers:
//! - Survival O/E hazard ratio SE (`sqrt(1/E_A + 1/E_B)`) & strict non-estimable guard (`E_A`, `E_B`, `O_B` = 0)
//! - Genuine Mantel–Haenszel hazard ratio (Berry, Kitchin & Mock 1991; Greenland & Robins 1985) vs Peto one-step
//! - Brookmeyer–Crowley (1982) median survival CI confidence-level sensitivity & `S = 0` boundary
//! - Welch *t* effect-size noncentral-*t* CI (`df_pool = n_a + n_b - 2`) vs `r^2` (`df_welch`)
//! - Pratt (1959) Wilcoxon signed-rank variance (no zero-group double counting)
//! - Hodges–Lehmann exact order-statistic selection & CI for `m * n > 40_000`
//! - Paired *t*-test Pearson pairing *r* two-sided *p*-value & `r = ±1` degenerate boundaries
//! - Pre-summarized unpaired/Welch *t*-tests (`f_test` exactness & `levene_median == None`)
//! - Consistent observed-direction one-tailed rank-test conventions across sign, Mann–Whitney, and Wilcoxon tests

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, ConfidenceLevel, MultiplicitySpec, ResultCell, SheetId,
    SubcolumnMode, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype, TwoGroupMethod, TwoGroupSpec,
};
use lumencraft_stats::run_analysis;
use lumencraft_stats::survival::{kaplan_meier_curve, two_cohort_hazard_ratio};
use lumencraft_stats::two_group::{
    f_test_variances, mann_whitney_u, paired_sign_test, paired_t_test, unpaired_t_test, unpaired_t_test_from_summary, welch_t_test,
    welch_t_test_from_summary, wilcoxon_matched_pairs,
};

fn assert_approx_eq(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, label: &str) {
    let diff = (actual - expected).abs();
    let limit = abs_tol.max(rel_tol * expected.abs());
    assert!(diff <= limit, "{label}: actual={actual:.15}, expected={expected:.15}, diff={diff:.3e}, limit={limit:.3e}");
}

fn obs(times: &[f64], events: &[bool]) -> Vec<SurvivalObservation> {
    times
        .iter()
        .zip(events.iter())
        .enumerate()
        .map(|(row, (&time, &ev))| SurvivalObservation { row, time, status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored } })
        .collect()
}

fn conf(percent: f64) -> ConfidenceLevel {
    ConfidenceLevel::new(percent).unwrap()
}

fn raw_col_sheet(cols: &[(&str, &[f64])]) -> lumencraft_doc::DataSheet {
    let max_len = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut tab = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), max_len).unwrap();
    for (c, (title, vals)) in cols.iter().enumerate() {
        tab.data_sets[c].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tab.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    lumencraft_doc::DataSheet::new(SheetId::new("r35-raw"), "Raw", TableArchetype::Column(tab)).unwrap()
}

fn summary_mean_sd_n_sheet(cols: &[(&str, (f64, f64, f64))]) -> lumencraft_doc::DataSheet {
    let mut tab = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, cols.len(), 1).unwrap();
    for (c, &(title, (mean, sd, n))) in cols.iter().enumerate() {
        tab.data_sets[c].title = title.to_owned();
        tab.data_sets[c].cells[0][0] = Cell::number(mean);
        tab.data_sets[c].cells[0][1] = Cell::number(sd);
        tab.data_sets[c].cells[0][2] = Cell::number(n);
    }
    lumencraft_doc::DataSheet::new(SheetId::new("r35-sum"), "Summary", TableArchetype::Column(tab)).unwrap()
}

#[test]
fn r35_surv_oe_hr_genuine_mh_and_peto_oracles() {
    // D1 Provenance: R 4.5.2 (survival 3.8.3 survdiff) + closed-form Berry, Kitchin & Mock (1991)
    // and Greenland & Robins (1985) variance evaluation, 2026-11-16.
    // Dataset A (within-arm ties only, 95% CI):
    let a = obs(&[2.0, 4.0, 6.0, 6.0, 8.0, 12.0, 15.0], &[true, true, true, false, true, true, false]);
    let b = obs(&[3.0, 7.0, 9.0, 11.0, 14.0, 18.0, 22.0], &[true, false, true, true, false, true, false]);

    let hr_a = two_cohort_hazard_ratio(&a, &b, conf(95.0)).unwrap();
    assert_approx_eq(hr_a.log_rank_hr, 2.148_656_482_559_67, 1e-11, 1e-11, "A OE HR");
    let lr_ci = hr_a.log_rank_ci.expect("A OE CI must be Some");
    assert_approx_eq(lr_ci.0, 0.554_315_675_588_536, 1e-11, 1e-11, "A OE CI lo (sqrt(1/Ea+1/Eb))");
    assert_approx_eq(lr_ci.1, 8.328_692_265_727_42, 1e-11, 1e-11, "A OE CI hi (sqrt(1/Ea+1/Eb))");
    assert_approx_eq(hr_a.mantel_haenszel_hr, 2.563_844_375_963_02, 1e-11, 1e-11, "A genuine MH HR");
    let mh_ci = hr_a.mantel_haenszel_ci.expect("A MH GR CI must be Some");
    assert_approx_eq(mh_ci.0, 0.538_626_076_139_008, 1e-11, 1e-11, "A MH GR CI lo");
    assert_approx_eq(mh_ci.1, 12.203_824_276_901_1, 1e-11, 1e-11, "A MH GR CI hi");
    assert_approx_eq(hr_a.peto_one_step_hr, 2.436_952_630_793_2, 1e-11, 1e-11, "A Peto HR");
    let peto_ci = hr_a.peto_one_step_ci.expect("A Peto CI must be Some");
    assert_approx_eq(peto_ci.0, 0.587_292_184_703_270, 1e-11, 1e-11, "A Peto CI lo");
    assert_approx_eq(peto_ci.1, 10.112_067_348_095_993, 1e-11, 1e-11, "A Peto CI hi");

    // Reciprocal symmetry when cohorts are swapped: HR(B, A) == 1 / HR(A, B) and bounds invert.
    let hr_a_rev = two_cohort_hazard_ratio(&b, &a, conf(95.0)).unwrap();
    assert_approx_eq(hr_a_rev.log_rank_hr, 1.0 / hr_a.log_rank_hr, 1e-12, 1e-12, "OE HR reciprocal");
    assert_approx_eq(hr_a_rev.mantel_haenszel_hr, 1.0 / hr_a.mantel_haenszel_hr, 1e-12, 1e-12, "MH HR reciprocal");
    assert_approx_eq(hr_a_rev.peto_one_step_hr, 1.0 / hr_a.peto_one_step_hr, 1e-12, 1e-12, "Peto HR reciprocal");
    let mh_ci_rev = hr_a_rev.mantel_haenszel_ci.unwrap();
    assert_approx_eq(mh_ci_rev.0, 1.0 / mh_ci.1, 1e-12, 1e-12, "MH CI lower reciprocal");
    assert_approx_eq(mh_ci_rev.1, 1.0 / mh_ci.0, 1e-12, 1e-12, "MH CI upper reciprocal");

    // Dataset B (cross-group event ties + censor ties, 90% CI):
    let b_a = obs(&[2.0, 4.0, 4.0, 7.0, 10.0, 14.0], &[true, true, false, true, true, false]);
    let b_b = obs(&[4.0, 6.0, 8.0, 10.0, 12.0, 16.0], &[true, true, false, true, false, true]);
    let hr_b = two_cohort_hazard_ratio(&b_a, &b_b, conf(90.0)).unwrap();
    assert_approx_eq(hr_b.log_rank_hr, 1.655_458_562_345_08, 1e-11, 1e-11, "B OE HR (90%)");
    let b_lr_ci = hr_b.log_rank_ci.unwrap();
    assert_approx_eq(b_lr_ci.0, 0.498_502_407_925_136, 1e-11, 1e-11, "B OE CI lo (90%)");
    assert_approx_eq(b_lr_ci.1, 5.497_552_284_748_864, 1e-11, 1e-11, "B OE CI hi (90%)");
    assert_approx_eq(hr_b.mantel_haenszel_hr, 1.803_010_298_389_226, 1e-11, 1e-11, "B genuine MH HR (90%)");
    let b_mh_ci = hr_b.mantel_haenszel_ci.unwrap();
    assert_approx_eq(b_mh_ci.0, 0.490_906_523_477_688, 1e-11, 1e-11, "B MH GR CI lo (90%)");
    assert_approx_eq(b_mh_ci.1, 6.622_128_614_360_038, 1e-11, 1e-11, "B MH GR CI hi (90%)");
    assert_approx_eq(hr_b.peto_one_step_hr, 1.902_149_719_257_808, 1e-11, 1e-11, "B Peto HR (90%)");
    let b_peto_ci = hr_b.peto_one_step_ci.unwrap();
    assert_approx_eq(b_peto_ci.0, 0.504_395_986_141_404, 1e-11, 1e-11, "B Peto CI lo (90%)");
    assert_approx_eq(b_peto_ci.1, 7.173_279_831_489_833, 1e-11, 1e-11, "B Peto CI hi (90%)");

    // Non-estimable O/E HR when O_B == 0 must NOT silently switch to Peto's estimator:
    let early_events = obs(&[1.0, 2.0, 3.0, 4.0], &[true, true, true, true]);
    let all_censored_late = obs(&[5.0, 6.0, 7.0, 8.0], &[false, false, false, false]);
    let hr_zero_b = two_cohort_hazard_ratio(&early_events, &all_censored_late, conf(95.0)).unwrap();
    assert!(hr_zero_b.log_rank_hr.is_nan(), "expected NaN O/E HR when O_B == 0, got {}", hr_zero_b.log_rank_hr);
    assert!(hr_zero_b.log_rank_hr_opt().is_none());
    assert!(hr_zero_b.log_rank_ci.is_none());
    assert!(hr_zero_b.mantel_haenszel_hr.is_nan(), "MH denom S == 0 must be NaN");
    assert!(hr_zero_b.mantel_haenszel_ci.is_none());
    assert!(hr_zero_b.peto_one_step_hr.is_finite(), "Peto remains estimable since V > 0");
    assert!(hr_zero_b.peto_one_step_ci.is_some());

    // Swapped zero-event arm (O_A == 0, O_B > 0, E_A > 0, E_B > 0): OE HR is 0.0, CI is None.
    let hr_zero_a = two_cohort_hazard_ratio(&all_censored_late, &early_events, conf(95.0)).unwrap();
    assert_approx_eq(hr_zero_a.log_rank_hr, 0.0, 1e-15, 0.0, "O_A == 0 gives OE HR = 0");
    assert!(hr_zero_a.log_rank_ci.is_none(), "O_A == 0 cannot form log-scale Wald CI");
    assert_approx_eq(hr_zero_a.mantel_haenszel_hr, 0.0, 1e-15, 0.0, "O_A == 0 gives MH HR = 0");
    assert!(hr_zero_a.mantel_haenszel_ci.is_none());

    // All-censored cohorts: all three HRs & CIs must be non-estimable (NaN, None) cleanly.
    let c_none_a = obs(&[1.0, 3.0], &[false, false]);
    let c_none_b = obs(&[2.0, 4.0], &[false, false]);
    let hr_none = two_cohort_hazard_ratio(&c_none_a, &c_none_b, conf(95.0)).unwrap();
    assert!(hr_none.log_rank_hr.is_nan() && hr_none.log_rank_ci.is_none());
    assert!(hr_none.mantel_haenszel_hr.is_nan() && hr_none.mantel_haenszel_ci.is_none());
    assert!(hr_none.peto_one_step_hr.is_nan() && hr_none.peto_one_step_ci.is_none());

    // End-to-end Survival ResultSheet: 2-cohort emits all 3 finite HR scalars; 3-cohort with an
    // all-censored arm omits non-estimable O/E HR & CI cleanly without failing float sanitization.
    let mut srv2 = SurvivalTable::new(2, 14).unwrap();
    for (r, o) in a.iter().enumerate() {
        srv2.times[r] = Cell::number(o.time);
        srv2.cohorts[0].cells[r][0] = Cell::number(if o.status == SurvivalStatus::Event { 1.0 } else { 0.0 });
    }
    for (r, o) in b.iter().enumerate() {
        srv2.times[7 + r] = Cell::number(o.time);
        srv2.cohorts[1].cells[7 + r][0] = Cell::number(if o.status == SurvivalStatus::Event { 1.0 } else { 0.0 });
    }
    let sheet2 = lumencraft_doc::DataSheet::new(SheetId::new("srv-2"), "Surv2", TableArchetype::Survival(srv2)).unwrap();
    let km_spec = |sid: &str| {
        AnalysisSpec::new(
            SheetId::new(format!("r-{sid}")),
            SheetId::new(sid),
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: false,
                multiplicity: MultiplicitySpec::default(),
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        )
        .unwrap()
    };
    let rs2 = run_analysis(&sheet2, &km_spec("srv-2")).unwrap();
    let sc = |k: &str| rs2.scalars.iter().find(|s| s.key == k).unwrap().value;
    assert_approx_eq(sc("Hazard ratio (log-rank)"), hr_a.log_rank_hr, 1e-12, 1e-12, "sheet OE HR");
    assert_approx_eq(sc("Hazard ratio (Mantel-Haenszel)"), hr_a.mantel_haenszel_hr, 1e-12, 1e-12, "sheet MH HR");
    assert_approx_eq(sc("Hazard ratio (Peto one-step)"), hr_a.peto_one_step_hr, 1e-12, 1e-12, "sheet Peto HR");

    let mut srv3 = SurvivalTable::new(3, 12).unwrap();
    for (r, o) in early_events.iter().enumerate() {
        srv3.times[r] = Cell::number(o.time);
        srv3.cohorts[0].cells[r][0] = Cell::number(1.0);
    }
    for (r, o) in all_censored_late.iter().enumerate() {
        srv3.times[4 + r] = Cell::number(o.time);
        srv3.cohorts[1].cells[4 + r][0] = Cell::number(0.0);
    }
    for (r, t) in [2.5, 4.5, 6.5, 8.5].into_iter().enumerate() {
        srv3.times[8 + r] = Cell::number(t);
        srv3.cohorts[2].cells[8 + r][0] = Cell::number(1.0);
    }
    let sheet3 = lumencraft_doc::DataSheet::new(SheetId::new("srv-3"), "Surv3", TableArchetype::Survival(srv3)).unwrap();
    let rs3 = run_analysis(&sheet3, &km_spec("srv-3")).unwrap();
    let pw = rs3.tabs.iter().find(|t| t.id == "survival-pairwise").unwrap();
    assert!(matches!(pw.rows[0][3], ResultCell::Empty), "Cohort 0 vs 1 (O_B == 0) HR cell must be Empty");
    assert!(matches!(pw.rows[0][4], ResultCell::Empty), "Cohort 0 vs 1 (O_B == 0) CI cell must be Empty");
}

#[test]
fn r35_surv_brookmeyer_crowley_confidence_and_s_zero_oracles() {
    // D1 Provenance: R 4.5.2 survival 3.8.3 (survfit conf.type="plain"/"log-log", quantile.survfit).
    // Dataset 1:
    let c1 = obs(&[2.0, 4.0, 6.0, 6.0, 8.0, 12.0, 15.0], &[true, true, true, false, true, true, false]);
    let km_p90 = kaplan_meier_curve("C1", &c1, false, conf(90.0)).unwrap();
    let km_p95 = kaplan_meier_curve("C1", &c1, false, conf(95.0)).unwrap();
    assert_eq!(km_p90.median_ci, (Some(4.0), Some(12.0)));
    assert_eq!(km_p95.median_ci, (Some(4.0), None));

    // Log-log CIs must also respond to confidence level (80% -> (4, 12) vs 90% -> (2, None)):
    let km_ll80 = kaplan_meier_curve("C1", &c1, true, conf(80.0)).unwrap();
    let km_ll90 = kaplan_meier_curve("C1", &c1, true, conf(90.0)).unwrap();
    assert_eq!(km_ll80.median_ci, (Some(4.0), Some(12.0)));
    assert_eq!(km_ll90.median_ci, (Some(2.0), None));

    // All-uncensored n=4 & n=6 edge cases where final step reaches S = 0.0 (indeterminate Greenwood SE):
    let all4 = obs(&[2.0, 4.0, 6.0, 8.0], &[true, true, true, true]);
    let km_all4 = kaplan_meier_curve("All4", &all4, false, conf(95.0)).unwrap();
    assert_eq!(km_all4.median_ci, (Some(2.0), None));

    let all6 = obs(&[2.0, 4.0, 6.0, 8.0, 10.0, 12.0], &[true, true, true, true, true, true]);
    let km_all6_plain = kaplan_meier_curve("All6", &all6, false, conf(95.0)).unwrap();
    let km_all6_ll = kaplan_meier_curve("All6", &all6, true, conf(95.0)).unwrap();
    assert_eq!(km_all6_plain.median_ci, (Some(4.0), Some(10.0)));
    assert_eq!(km_all6_ll.median_ci, (Some(2.0), None));

    // Single late event at end (S drops straight from 1.0 to 0.0): no finite-variance steps -> (None, None).
    let last_only = obs(&[2.0, 4.0, 6.0, 8.0], &[false, false, false, true]);
    let km_last = kaplan_meier_curve("LastOnly", &last_only, false, conf(95.0)).unwrap();
    assert_eq!(km_last.median_ci, (None, None));
}

#[test]
fn r35_tg_welch_noncentral_t_ci_pooled_df_oracle() {
    // D1 Provenance: R 4.5.2 uniroot on stats::pt(..., df = n_a + n_b - 2, ncp = delta), 2026-11-16.
    let a = [10.0, 12.0, 14.0, 16.0, 18.0];
    let b = [25.0, 35.0, 45.0, 55.0, 65.0, 75.0];

    let w_raw = welch_t_test(&a, &b, true, conf(95.0)).unwrap();
    assert_approx_eq(w_raw.df, 5.340_886_860_124, 1e-9, 1e-9, "Welch df");
    assert_approx_eq(w_raw.effect_sizes.cohens_d, -2.552_685_079_934, 1e-10, 1e-10, "Welch Cohen d");
    assert_approx_eq(w_raw.effect_sizes.hedges_g, -2.332_834_801_205, 1e-10, 1e-10, "Welch Hedges g");
    let d_ci = w_raw.effect_sizes.cohens_d_ci.unwrap();
    let g_ci = w_raw.effect_sizes.hedges_g_ci.unwrap();
    assert_approx_eq(d_ci.0, -4.177_853_838_570, 1e-6, 1e-6, "Welch d CI lo (df_pool=9)");
    assert_approx_eq(d_ci.1, -0.857_701_561_587, 1e-6, 1e-6, "Welch d CI hi (df_pool=9)");
    assert_approx_eq(g_ci.0, -3.818_035_724_648, 1e-6, 1e-6, "Welch g CI lo (df_pool=9)");
    assert_approx_eq(g_ci.1, -0.783_831_921_786, 1e-6, 1e-6, "Welch g CI hi (df_pool=9)");
    assert_approx_eq(w_raw.effect_sizes.r_squared, 0.800_873_294_374, 1e-10, 1e-10, "Welch R^2 (t_w^2/(t_w^2+df_w))");

    // Pre-summarized Welch must produce the exact same pooled-df CIs:
    let sdb = (1750.0_f64 / 5.0).sqrt();
    let sda = 10.0_f64.sqrt();
    let w_sum = welch_t_test_from_summary((14.0, sda, 5.0), (50.0, sdb, 6.0), true, conf(95.0)).unwrap();
    let d_ci_sum = w_sum.effect_sizes.cohens_d_ci.unwrap();
    assert_approx_eq(d_ci_sum.0, -4.177_853_838_570, 1e-6, 1e-6, "Summary Welch d CI lo");
    assert_approx_eq(d_ci_sum.1, -0.857_701_561_587, 1e-6, 1e-6, "Summary Welch d CI hi");
}

#[test]
fn r35_tg_pratt_wilcoxon_oracle_no_zero_double_counting() {
    // D1 Provenance: R 4.5.2 exact Pratt (1959) variance without zero-group tie double-counting.
    // d = [0, 0, 0, 1, 1, 2, 2, 2, -1, 3, 4, 5, 6, 7, 8]
    let a = [10.0, 20.0, 30.0, 11.0, 21.0, 12.0, 22.0, 32.0, 9.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0];
    let b = [10.0, 20.0, 30.0, 10.0, 20.0, 10.0, 20.0, 30.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];

    let w_two = wilcoxon_matched_pairs(&a, &b, true, conf(95.0), true).unwrap();
    assert_approx_eq(w_two.w_plus, 109.0, 1e-12, 0.0, "Pratt W+");
    assert_approx_eq(w_two.w_minus, 5.0, 1e-12, 0.0, "Pratt W-");
    assert_approx_eq(w_two.p_value, 0.003_214_264_298_464_38, 1e-11, 1e-11, "Pratt two-sided p");

    let w_one = wilcoxon_matched_pairs(&a, &b, false, conf(95.0), true).unwrap();
    assert_approx_eq(w_one.p_value, 0.001_607_132_149_232_19, 1e-11, 1e-11, "Pratt one-sided p");
}

#[test]
fn r35_tg_hodges_lehmann_large_n_oracle() {
    // D1 Provenance: Exact O(mn) sorted pairwise difference distribution + R 4.5.2 stats::wilcox.test
    // on m = 210, n = 210 (m * n = 44_100 > 40_000 cutoff).
    let la: Vec<f64> = (0..210).map(|i| 10.0 + ((i * 37) % 101) as f64 * 0.13 + (i as f64) * 0.01).collect();
    let lb: Vec<f64> = (0..210).map(|j| 7.0 + ((j * 53) % 97) as f64 * 0.11 + (j as f64) * 0.012).collect();

    let mw = mann_whitney_u(&la, &lb, true, conf(95.0)).unwrap();
    // Exact median of 44_100 pairwise diffs is 4.038 (naive median(la)-median(lb) is 4.042);
    // R wilcox.test(conf.int=TRUE) gives 4.03802026581186, CI = (3.30199503540664, 4.77395041182155).
    assert_approx_eq(mw.hodges_lehmann_diff, 4.038, 1e-11, 0.0, "HL 210x210 exact pairwise median");
    assert_approx_eq(mw.hodges_lehmann_diff, 4.038_020_265_811_86, 1e-4, 0.0, "HL 210x210 vs R wilcox.test");
    let (ci_lo, ci_hi) = mw.hodges_lehmann_ci.expect("large-sample HL CI must be Some");
    assert_approx_eq(ci_lo, 3.302, 1e-11, 0.0, "HL 210x210 exact Moses CI lo");
    assert_approx_eq(ci_hi, 4.774, 1e-11, 0.0, "HL 210x210 exact Moses CI hi");
}

#[test]
fn r35_tg_pairing_correlation_two_sided_and_boundaries() {
    // D1 Provenance: R 4.5.2 stats::cor.test(x, y, alternative = "two.sided").
    // Negative correlation: r = -0.961937978939, two-sided p = 0.008862912526 (n = 5, df = 3).
    let x = [1.0, 2.0, 4.0, 7.0, 11.0];
    let y = [10.0, 8.0, 5.0, 3.0, 1.0];
    let res_neg = paired_t_test(&x, &y, true, conf(95.0)).unwrap();
    assert_approx_eq(res_neg.pairing_r.unwrap(), -0.961_937_978_939, 1e-10, 1e-10, "neg pairing r");
    assert_approx_eq(res_neg.pairing_p.unwrap(), 0.008_862_912_526, 1e-10, 1e-10, "neg pairing two-sided p");

    // Positive correlation of identical magnitude yields the identical two-sided p:
    let y_pos = [0.0, 2.0, 5.0, 7.0, 9.0];
    let res_pos = paired_t_test(&x, &y_pos, true, conf(95.0)).unwrap();
    assert_approx_eq(res_pos.pairing_r.unwrap(), 0.961_937_978_939, 1e-10, 1e-10, "pos pairing r");
    assert_approx_eq(res_pos.pairing_p.unwrap(), 0.008_862_912_526, 1e-10, 1e-10, "pos pairing two-sided p");

    // Degenerate boundaries r = -1.0, r = +1.0, r = 0.0 (with non-zero difference variance, n >= 3):
    let a = [1.0, 2.0, 3.0, 4.0, 5.0];
    let b_neg1 = [10.0, 8.0, 6.0, 4.0, 2.0];
    let res_m1 = paired_t_test(&a, &b_neg1, true, conf(95.0)).unwrap();
    assert_approx_eq(res_m1.pairing_r.unwrap(), -1.0, 1e-12, 0.0, "exact r = -1");
    assert_approx_eq(res_m1.pairing_p.unwrap(), 0.0, 1e-12, 0.0, "r = -1 two-sided p = 0");

    let b_pos1 = [2.0, 4.0, 6.0, 8.0, 10.0];
    let res_p1 = paired_t_test(&a, &b_pos1, true, conf(95.0)).unwrap();
    assert_approx_eq(res_p1.pairing_r.unwrap(), 1.0, 1e-12, 0.0, "exact r = +1");
    assert_approx_eq(res_p1.pairing_p.unwrap(), 0.0, 1e-12, 0.0, "r = +1 two-sided p = 0");

    let a_zero = [-2.0, -1.0, 0.0, 1.0, 2.0];
    let b_zero = [2.0, -1.0, -2.0, -1.0, 2.0];
    let res_zero = paired_t_test(&a_zero, &b_zero, true, conf(95.0)).unwrap();
    assert_approx_eq(res_zero.pairing_r.unwrap(), 0.0, 1e-12, 0.0, "exact r = 0");
    assert_approx_eq(res_zero.pairing_p.unwrap(), 1.0, 1e-12, 0.0, "r = 0 two-sided p = 1");

    // Checklist: negative correlation must warn rather than pass "Pairing effectiveness"
    let sheet_neg = raw_col_sheet(&[("X", &x), ("Y", &y)]);
    let spec = AnalysisSpec::new(
        SheetId::new("r-neg-pair"),
        sheet_neg.id.clone(),
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::PairedT, ..Default::default() }),
    )
    .unwrap();
    let rs = run_analysis(&sheet_neg, &spec).unwrap();
    let pairing_item = rs.checklist.iter().find(|c| c.assumption == "Pairing effectiveness").unwrap();
    assert_eq!(pairing_item.status, ChecklistStatus::Warn);
}

#[test]
fn r35_tg_summary_t_tests_no_placeholder_levene_and_exact_f() {
    let raw_a = [10.0, 12.0, 14.0, 16.0, 18.0];
    let raw_b = [20.0, 24.0, 28.0, 32.0];
    let f_raw = f_test_variances(&raw_a, &raw_b).unwrap();
    let raw_st = unpaired_t_test(&raw_a, &raw_b, true, conf(95.0)).unwrap();
    assert!(raw_st.levene_median.is_some(), "raw Student t must include Brown-Forsythe Levene result");

    let sda = 10.0_f64.sqrt();
    let sdb = (80.0_f64 / 3.0).sqrt();
    let sum_st = unpaired_t_test_from_summary((14.0, sda, 5.0), (26.0, sdb, 4.0), true, conf(95.0)).unwrap();
    let sum_w = welch_t_test_from_summary((14.0, sda, 5.0), (26.0, sdb, 4.0), true, conf(95.0)).unwrap();

    assert_approx_eq(sum_st.f_test.f_statistic, f_raw.f_statistic, 1e-12, 1e-12, "summary F stat");
    assert_approx_eq(sum_st.f_test.p_value, f_raw.p_value, 1e-12, 1e-12, "summary F p");
    assert_approx_eq(sum_w.f_test.f_statistic, f_raw.f_statistic, 1e-12, 1e-12, "summary Welch F stat");
    assert_approx_eq(sum_st.t_statistic, raw_st.t_statistic, 1e-12, 1e-12, "summary Student t");
    assert!(sum_st.levene_median.is_none(), "summary Student t must not fabricate a Levene result");
    assert!(sum_w.levene_median.is_none(), "summary Welch t must not fabricate a Levene result");

    // End-to-end ResultSheet checklist on pre-summarized MeanSdN input must emit exact F-test +
    // "Pre-summarized input mode", never fake "Levene(median) F = 0.000, p = 1.0000":
    let sum_sheet = summary_mean_sd_n_sheet(&[("A", (14.0, sda, 5.0)), ("B", (26.0, 20.0, 4.0))]);
    for method in [TwoGroupMethod::UnpairedT, TwoGroupMethod::WelchT] {
        let spec = AnalysisSpec::new(
            SheetId::new("r-sum"),
            sum_sheet.id.clone(),
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method, ..Default::default() }),
        )
        .unwrap();
        let rs = run_analysis(&sum_sheet, &spec).unwrap();
        assert!(
            !rs.checklist.iter().any(|c| c.diagnostic_metric.as_deref().is_some_and(|m| m.contains("Levene"))),
            "pre-summarized checklist must not report placeholder Levene metric"
        );
        assert!(
            rs.checklist.iter().any(|c| c.assumption == "Pre-summarized input mode" && c.status == ChecklistStatus::Info),
            "pre-summarized checklist must include Pre-summarized input mode Info item"
        );
    }
}

#[test]
fn r35_tg_one_tailed_conventions_consistent() {
    // D1 Provenance: R 4.5.2 stats::binom.test & stats::wilcox.test exact tails.
    let ma = [1.0, 2.0, 3.0, 4.0, 5.0];
    let mb = [6.0, 7.0, 8.0, 9.0, 10.5];
    let mw_ab = mann_whitney_u(&ma, &mb, false, conf(95.0)).unwrap();
    let mw_ba = mann_whitney_u(&mb, &ma, false, conf(95.0)).unwrap();
    assert_approx_eq(mw_ab.p_value, 1.0 / 252.0, 1e-12, 0.0, "MW exact one-tailed a < b");
    assert_approx_eq(mw_ba.p_value, 1.0 / 252.0, 1e-12, 0.0, "MW exact one-tailed b > a");

    let s_ab = paired_sign_test(&ma, &mb, false, conf(95.0)).unwrap();
    let s_ba = paired_sign_test(&mb, &ma, false, conf(95.0)).unwrap();
    assert_approx_eq(s_ab.p_value, 1.0 / 32.0, 1e-12, 0.0, "Sign test exact one-tailed a < b");
    assert_approx_eq(s_ba.p_value, 1.0 / 32.0, 1e-12, 0.0, "Sign test exact one-tailed b > a");

    let mb_untied_diff = [6.0, 8.0, 11.0, 15.0, 20.5];
    let w_ab = wilcoxon_matched_pairs(&ma, &mb_untied_diff, false, conf(95.0), false).unwrap();
    let w_ba = wilcoxon_matched_pairs(&mb_untied_diff, &ma, false, conf(95.0), false).unwrap();
    assert!(w_ab.exact_p_used && w_ba.exact_p_used);
    assert_approx_eq(w_ab.p_value, 1.0 / 32.0, 1e-12, 0.0, "Wilcoxon exact one-tailed a < b");
    assert_approx_eq(w_ba.p_value, 1.0 / 32.0, 1e-12, 0.0, "Wilcoxon exact one-tailed b > a");

    // Tied paired sample + asymptotic MW: observed-direction one-tailed p == 0.5 * two_tailed p
    let w_tied_1 = wilcoxon_matched_pairs(&ma, &mb, false, conf(95.0), false).unwrap();
    let w_tied_2 = wilcoxon_matched_pairs(&ma, &mb, true, conf(95.0), false).unwrap();
    assert_approx_eq(w_tied_1.p_value, 0.5 * w_tied_2.p_value, 1e-12, 0.0, "Tied Wilcoxon one-tailed = half two-tailed");

    let lg_a: Vec<f64> = (0..30).map(|i| i as f64).collect();
    let lg_b: Vec<f64> = (0..30).map(|i| i as f64 + 10.0).collect();
    let mw_asymp_1 = mann_whitney_u(&lg_a, &lg_b, false, conf(95.0)).unwrap();
    let mw_asymp_1_rev = mann_whitney_u(&lg_b, &lg_a, false, conf(95.0)).unwrap();
    let mw_asymp_2 = mann_whitney_u(&lg_a, &lg_b, true, conf(95.0)).unwrap();
    assert!(!mw_asymp_1.exact_p_used);
    assert_approx_eq(mw_asymp_1.p_value, mw_asymp_1_rev.p_value, 1e-12, 0.0, "Asymp MW one-tailed direction symmetry");
    assert_approx_eq(mw_asymp_1.p_value, 0.5 * mw_asymp_2.p_value, 1e-12, 0.0, "Asymp MW one-tailed = half two-tailed");
}
