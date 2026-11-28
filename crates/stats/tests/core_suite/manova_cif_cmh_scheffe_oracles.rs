//! `D1` R 4.5.2 numerical oracles and degenerate edge-case tests for:
//! - One-way MANOVA (Pillai, Wilks, Hotelling–Lawley, Roy, SSCP H/E, univariate ANOVAs)
//! - Competing-risks Aalen–Johansen CIF + Greenwood/Marubini–Valsecchi SE + Gray's K-sample test
//! - Scheffé (1953) post-hoc comparisons (adjusted P values & simultaneous CIs)
//! - Stratified 2 × 2 × K Cochran–Mantel–Haenszel test, Robins–Breslow–Greenland CI, and
//!   Breslow–Day / Tarone homogeneity test.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CmhSpec, ColumnDesignHint, ColumnTable, CompetingRisksSpec, ContingencyTable, DataSheet, FactorialAnovaSpec,
    FactorialComparisonFamilyMode, GroupedTable, ManovaSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec,
    OneWayMethod, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::f_sf;
use curvebook_stats::categorical::{CmhStratum, cochran_mantel_haenszel};
use curvebook_stats::competing_risks::{CompetingRiskObservation, CompetingRiskStatus, aalen_johansen_cif, gray_k_sample_test};
use curvebook_stats::manova::one_way_manova;
use curvebook_stats::multiplicity::{GroupComparisonInput, compare_groups_pairwise_with_confidence};
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tol,
        "{label}: actual={actual:.15e}, expected={expected:.15e}, diff={:.3e} > tol={tol:.3e}",
        (actual - expected).abs()
    );
}

// ============================================================================
// 1. One-Way MANOVA — R 4.5.2 `summary(manova(Y ~ g), test = ...)` oracles
// ============================================================================

#[test]
fn manova_3group_3response_matches_r_4_5_2_all_four_tests() {
    let groups: Vec<String> = ["A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C"].into_iter().map(String::from).collect();
    let y1 = [5.1, 4.9, 4.7, 5.4, 5.0, 6.4, 6.9, 6.5, 6.2, 6.8, 7.1, 7.6, 7.3, 6.9, 7.4];
    let y2 = [3.5, 3.0, 3.2, 3.9, 3.6, 3.2, 3.1, 2.8, 2.9, 3.0, 3.0, 3.4, 2.9, 3.1, 3.2];
    let y3 = [1.4, 1.4, 1.3, 1.7, 1.5, 4.5, 4.9, 4.6, 4.3, 4.8, 5.9, 6.6, 6.3, 5.1, 6.1];
    let obs: Vec<Vec<f64>> = (0..15).map(|i| vec![y1[i], y2[i], y3[i]]).collect();
    let names = vec![String::from("Y1"), String::from("Y2"), String::from("Y3")];

    let res = one_way_manova(&groups, &obs, &names, None).expect("MANOVA succeeds");
    assert_eq!(res.n_observations, 15);
    assert_eq!(res.n_groups, 3);
    assert_eq!(res.n_responses, 3);
    assert_eq!(res.df_hypothesis, 2.0);
    assert_eq!(res.df_error, 12.0);

    // R 4.5.2 `summary(manova(Y ~ g), test = ...)` exact oracles
    assert_close(res.pillai.statistic, 1.184_770_310_520, 1e-10, "Pillai stat");
    assert_close(res.pillai.approx_f, 5.328_753_186_125, 1e-10, "Pillai F");
    assert_close(res.pillai.num_df, 6.0, 1e-12, "Pillai df1");
    assert_close(res.pillai.den_df, 22.0, 1e-12, "Pillai df2");
    assert_close(res.pillai.p_value, 1.583_296_844_690_297e-3, 1e-10, "Pillai p");

    assert_close(res.wilks.statistic, 0.018_586_984_644, 1e-10, "Wilks stat");
    assert_close(res.wilks.approx_f, 21.116_408_357_587, 1e-9, "Wilks F");
    assert_close(res.wilks.num_df, 6.0, 1e-12, "Wilks df1");
    assert_close(res.wilks.den_df, 20.0, 1e-12, "Wilks df2");
    assert_close(res.wilks.p_value, 1.123_902_628_120_002e-7, 1e-12, "Wilks p");
    assert!(res.wilks.is_exact);

    assert_close(res.hotelling_lawley.statistic, 41.860_244_416_977, 1e-9, "HL stat");
    assert_close(res.hotelling_lawley.approx_f, 62.790_366_625_465, 1e-9, "HL F");
    assert_close(res.hotelling_lawley.num_df, 6.0, 1e-12, "HL df1");
    assert_close(res.hotelling_lawley.den_df, 18.0, 1e-12, "HL df2");
    assert_close(res.hotelling_lawley.p_value, 4.311_245_422_939_084e-11, 1e-14, "HL p");

    assert_close(res.roy.statistic, 41.597_225_824_900, 1e-9, "Roy stat");
    assert_close(res.roy.approx_f, 152.523_161_357_966, 1e-8, "Roy F");
    assert_close(res.roy.num_df, 3.0, 1e-12, "Roy df1");
    assert_close(res.roy.den_df, 11.0, 1e-12, "Roy df2");
    assert_close(res.roy.p_value, 3.052_203_442_653_696e-9, 1e-13, "Roy p");

    // Univariate ANOVA table vs R `summary.aov(fit)`
    assert_close(res.univariate[0].ss_between, 13.132, 1e-10, "Univar Y1 SSb");
    assert_close(res.univariate[0].ss_within, 0.892, 1e-10, "Univar Y1 SSw");
    assert_close(res.univariate[0].f_statistic, 88.331_838_565_022, 1e-9, "Univar Y1 F");
    assert_close(res.univariate[0].p_value, 6.621_520_191_905_109e-8, 1e-12, "Univar Y1 p");

    assert_close(res.univariate[1].f_statistic, 4.194_594_594_595, 1e-10, "Univar Y2 F");
    assert_close(res.univariate[1].p_value, 4.156_116_782_867_573e-2, 1e-10, "Univar Y2 p");

    assert_close(res.univariate[2].f_statistic, 203.135, 1e-9, "Univar Y3 F");
    assert_close(res.univariate[2].p_value, 5.576_313_453_196_163e-10, 1e-13, "Univar Y3 p");

    // End-to-end ResultSheet dispatch on MultipleVariablesTable.
    let mut mv = MultipleVariablesTable::new(4, 15).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Y1");
    mv.variables[2].title = String::from("Y2");
    mv.variables[3].title = String::from("Y3");
    for r in 0..15 {
        mv.variables[0].cells[r] = Cell::text(&groups[r]);
        mv.variables[1].cells[r] = Cell::number(y1[r]);
        mv.variables[2].cells[r] = Cell::number(y2[r]);
        mv.variables[3].cells[r] = Cell::number(y3[r]);
    }
    let ds = DataSheet::new(SheetId::new("mv-manova"), "MANOVA Data", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-manova"), ds.id.clone(), AnalysisKind::Manova(ManovaSpec::default())).unwrap();
    let rs = run_analysis(&ds, &spec).expect("run_analysis MANOVA");
    assert_eq!(rs.tabs.len(), 5);
    assert_close(rs.scalars.iter().find(|s| s.key == "Wilks' Lambda").unwrap().value, 0.018_586_984_644, 1e-10, "scalar Wilks");
}

#[test]
fn manova_two_group_all_four_f_match_hotelling_t2_and_edge_cases_error_cleanly() {
    let groups: Vec<String> = ["A", "A", "A", "A", "A", "B", "B", "B", "B", "B"].into_iter().map(String::from).collect();
    let obs = vec![
        vec![5.1, 3.5],
        vec![4.9, 3.0],
        vec![4.7, 3.2],
        vec![5.4, 3.9],
        vec![5.0, 3.6],
        vec![6.4, 3.2],
        vec![6.9, 3.1],
        vec![6.5, 2.8],
        vec![6.2, 2.9],
        vec![6.8, 3.0],
    ];
    let names = vec![String::from("Y1"), String::from("Y2")];
    let res = one_way_manova(&groups, &obs, &names, None).unwrap();
    // When s = 1, all four F statistics and p-values are mathematically identical (Hotelling's T^2).
    for tst in [&res.pillai, &res.wilks, &res.hotelling_lawley, &res.roy] {
        assert_close(tst.approx_f, 76.698_018_068_493, 1e-9, &tst.name);
        assert_close(tst.p_value, 1.736_462_585_404_9e-5, 1e-11, &tst.name);
        assert!(tst.is_exact);
    }

    // Degenerate edge cases: NaN, empty, collinear E, insufficient df, single group.
    assert!(matches!(one_way_manova(&[], &[], &names, None), Err(StatsError::EmptyInput)));
    let mut bad_nan = obs.clone();
    bad_nan[0][0] = f64::NAN;
    assert!(matches!(one_way_manova(&groups, &bad_nan, &names, None), Err(StatsError::InvalidParameter { .. })));

    let collinear: Vec<Vec<f64>> = obs.iter().map(|r| vec![r[0], 2.0 * r[0]]).collect();
    assert!(matches!(one_way_manova(&groups, &collinear, &names, None), Err(StatsError::ZeroVariance { .. })));

    let one_group = vec![String::from("Only"); 10];
    assert!(matches!(one_way_manova(&one_group, &obs, &names, None), Err(StatsError::InsufficientSampleSize { .. })));
}

// ============================================================================
// 2. Scheffé (1953) Post-Hoc Comparisons — R 4.5.2 oracle
// ============================================================================

#[test]
fn scheffe_post_hoc_matches_r_4_5_2_p_values_and_simultaneous_cis() {
    let g1 = [10.0, 11.0, 12.0, 10.5, 11.5];
    let g2 = [12.0, 13.0, 12.5, 13.5];
    let g3 = [15.0, 16.0, 14.5, 15.5, 16.5, 15.0];
    let g4 = [20.0, 19.0, 21.0, 20.5, 19.5];

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 6).unwrap();
    for (ci, (title, data)) in [("G1", &g1[..]), ("G2", &g2[..]), ("G3", &g3[..]), ("G4", &g4[..])].into_iter().enumerate() {
        col.data_sets[ci].title = String::from(title);
        for (ri, &v) in data.iter().enumerate() {
            col.data_sets[ci].cells[ri][0] = Cell::number(v);
        }
    }
    let ds = DataSheet::new(SheetId::new("col-scheffe"), "Scheffe", TableArchetype::Column(col)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("res-scheffe"),
        ds.id.clone(),
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::OrdinaryAnova,
            multiplicity: MultiplicitySpec {
                method: MultiplicityMethod::Scheffe,
                threshold: 0.05,
                report_confidence_intervals: true,
                ..Default::default()
            },
            ..OneWayAnovaSpec::default()
        }),
    )
    .unwrap();
    let rs = run_analysis(&ds, &spec).expect("OneWayAnova with Scheffe");
    assert_eq!(rs.pairwise_comparisons.len(), 6);

    // Direct kernel verification against R 4.5.2 `pf(t^2 / (k - 1), k - 1, df_e, lower.tail = FALSE)`.
    let groups = vec![
        GroupComparisonInput { index: 0, label: "G1".into(), n: 5.0, mean: 11.0, sd: 0.790_569_415_042_094_9 },
        GroupComparisonInput { index: 1, label: "G2".into(), n: 4.0, mean: 12.75, sd: 0.645_497_224_367_902_8 },
        GroupComparisonInput { index: 2, label: "G3".into(), n: 6.0, mean: 15.416_666_666_666_666, sd: 0.735_980_072_193_987_2 },
        GroupComparisonInput { index: 3, label: "G4".into(), n: 5.0, mean: 20.0, sd: 0.790_569_415_042_094_9 },
    ];
    let mse = 0.559_895_833_333_333_4;
    let df_e = 16.0;
    let mspec = MultiplicitySpec { method: MultiplicityMethod::Scheffe, threshold: 0.05, report_confidence_intervals: true, ..Default::default() };
    let dets = compare_groups_pairwise_with_confidence(&groups, mse, df_e, &mspec, ConfidenceLevel::DEFAULT_95).unwrap();

    // Pair G1-G2
    assert_close(dets[0].mean_diff, -1.75, 1e-12, "G1-G2 diff");
    assert_close(dets[0].adjusted_p_or_q, 2.550_570_644_870_986e-2, 1e-11, "G1-G2 Scheffe p");
    let (lo0, hi0) = dets[0].ci.unwrap();
    assert_close(lo0, -3.314_650_568_302, 1e-10, "G1-G2 Scheffe CI lo");
    assert_close(hi0, -0.185_349_431_698, 1e-10, "G1-G2 Scheffe CI hi");
    assert!(dets[0].significant);

    // Pair G1-G3
    assert_close(dets[1].adjusted_p_or_q, 5.801_567_743_900_388e-7, 1e-12, "G1-G3 Scheffe p");
    // Pair G2-G3
    assert_close(dets[3].adjusted_p_or_q, 5.480_240_395_186_857e-4, 1e-11, "G2-G3 Scheffe p");
    // Pair G3-G4
    assert_close(dets[5].adjusted_p_or_q, 3.499_925_002_070_126e-7, 1e-12, "G3-G4 Scheffe p");

    // ResultSheet pairwise record matches exact Scheffé p for G1 vs G2.
    assert_close(rs.pairwise_comparisons[0].raw_p_or_q, 2.550_570_644_870_986e-2, 1e-10, "ResultSheet G1-G2 Scheffe p");
}

// ============================================================================
// 3. Competing-Risks Aalen–Johansen CIF & Gray's K-Sample Test — R 4.5.2 oracle
// ============================================================================

#[test]
fn competing_risks_cif_and_gray_test_match_r_4_5_2() {
    let mk = |t: f64, s: u8| CompetingRiskObservation {
        time: t,
        status: match s {
            1 => CompetingRiskStatus::PrimaryEvent,
            2 => CompetingRiskStatus::CompetingEvent,
            _ => CompetingRiskStatus::Censored,
        },
    };
    let arm_a: Vec<_> = [(1., 1), (2., 1), (2., 2), (3., 0), (4., 1), (5., 2), (6., 1), (8., 0)].into_iter().map(|(t, s)| mk(t, s)).collect();
    let arm_b: Vec<_> = [(2., 2), (3., 2), (4., 1), (5., 0), (5., 2), (7., 1), (9., 2), (10., 0)].into_iter().map(|(t, s)| mk(t, s)).collect();

    let cif_a = aalen_johansen_cif("ArmA", &arm_a, ConfidenceLevel::DEFAULT_95, true).unwrap();
    let cif_b = aalen_johansen_cif("ArmB", &arm_b, ConfidenceLevel::DEFAULT_95, true).unwrap();

    // Verified against R 4.5.2 `survival::survfit(Surv(t, factor(s)) ~ 1)`
    assert_close(cif_a.final_cif_primary, 0.562_5, 1e-12, "ArmA I1");
    assert_close(cif_a.steps.last().unwrap().se_primary, 0.188_797_593_337, 1e-10, "ArmA SE1");
    assert_close(cif_a.final_cif_competing, 0.281_25, 1e-12, "ArmA I2");
    assert_close(cif_a.steps.last().unwrap().se_competing, 0.170_090_164_508, 1e-10, "ArmA SE2");

    assert_close(cif_b.final_cif_primary, 0.291_666_666_667, 1e-10, "ArmB I1");
    assert_close(cif_b.steps.last().unwrap().se_primary, 0.174_511_553_793, 1e-10, "ArmB SE1");
    assert_close(cif_b.final_cif_competing, 0.541_666_666_667, 1e-10, "ArmB I2");
    assert_close(cif_b.steps.last().unwrap().se_competing, 0.188_845_481_123, 1e-10, "ArmB SE2");

    // Identity: S(t) + I1(t) + I2(t) == 1 at every time step.
    for step in cif_a.steps.iter().chain(cif_b.steps.iter()) {
        assert_close(step.overall_survival + step.cif_primary + step.cif_competing, 1.0, 1e-13, "S + I1 + I2 == 1");
        assert!(step.ci_primary_lower <= step.cif_primary + 1e-12 && step.cif_primary <= step.ci_primary_upper + 1e-12);
    }

    // Gray's K-sample test vs R 4.5.2 exact implementation
    let groups = vec![(String::from("ArmA"), arm_a.clone()), (String::from("ArmB"), arm_b.clone())];
    let gp = gray_k_sample_test(&groups, CompetingRiskStatus::PrimaryEvent, 0.0).unwrap();
    let gc = gray_k_sample_test(&groups, CompetingRiskStatus::CompetingEvent, 0.0).unwrap();
    assert_close(gp.chi_square, 1.621_930_359_557, 1e-10, "Gray primary chi2");
    assert_close(gp.df, 1.0, 1e-12, "Gray primary df");
    assert_close(gp.p_value, 2.028_228_364_471_531e-1, 1e-10, "Gray primary p");
    assert_close(gc.chi_square, 0.179_602_498_333, 1e-10, "Gray competing chi2");
    assert_close(gc.p_value, 6.717_150_699_293_601e-1, 1e-10, "Gray competing p");

    // No-competing-risks identity: Gray(rho=0) == Mantel-Cox log-rank (R `survdiff` = 1.896511986595).
    let g_km1: Vec<_> = [(1., 1), (2., 1), (3., 0), (4., 1), (5., 1), (6., 0)].into_iter().map(|(t, s)| mk(t, s)).collect();
    let g_km2: Vec<_> = [(2., 1), (4., 0), (5., 1), (7., 1), (8., 0), (9., 1)].into_iter().map(|(t, s)| mk(t, s)).collect();
    let g_km3: Vec<_> = [(1., 1), (3., 1), (6., 1), (8., 0), (10., 1), (12., 0)].into_iter().map(|(t, s)| mk(t, s)).collect();
    let gray_no_cr = gray_k_sample_test(
        &[(String::from("1"), g_km1), (String::from("2"), g_km2), (String::from("3"), g_km3)],
        CompetingRiskStatus::PrimaryEvent,
        0.0,
    )
    .unwrap();
    assert_close(gray_no_cr.chi_square, 1.896_511_986_595, 1e-10, "Gray == survdiff when no competing events");
    assert_close(gray_no_cr.df, 2.0, 1e-12, "Gray 3-arm df");

    // End-to-end SurvivalTable dispatch with status codes 0, 1, 2.
    let mut surv = SurvivalTable::new(2, 8).unwrap();
    surv.cohorts[0].title = String::from("ArmA");
    surv.cohorts[1].title = String::from("ArmB");
    let times_a = [1., 2., 2., 3., 4., 5., 6., 8.];
    let codes_a = [1., 1., 2., 0., 1., 2., 1., 0.];
    let codes_b = [2., 2., 1., 0., 2., 1., 2., 0.];
    for r in 0..8 {
        surv.times[r] = Cell::number(times_a[r]);
        surv.cohorts[0].cells[r][0] = Cell::number(codes_a[r]);
        surv.cohorts[1].cells[r][0] = Cell::number(codes_b[r]);
    }
    let ds = DataSheet::new(SheetId::new("surv-cr"), "CR Survival", TableArchetype::Survival(surv)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-cr"), ds.id.clone(), AnalysisKind::CompetingRisks(CompetingRisksSpec::default())).unwrap();
    let rs = run_analysis(&ds, &spec).expect("run_analysis CompetingRisks");
    assert!(rs.tabs.iter().any(|t| t.id == "cr-gray-test"));
    assert_eq!(rs.fitted_curves.len(), 2);

    // Degenerate edge cases: empty, negative time, NaN time, negative rho.
    assert!(matches!(aalen_johansen_cif("empty", &[], ConfidenceLevel::DEFAULT_95, true), Err(StatsError::EmptyInput)));
    assert!(matches!(aalen_johansen_cif("neg", &[mk(-1.0, 1)], ConfidenceLevel::DEFAULT_95, true), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(aalen_johansen_cif("nan", &[mk(f64::NAN, 1)], ConfidenceLevel::DEFAULT_95, true), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(gray_k_sample_test(&groups, CompetingRiskStatus::PrimaryEvent, -0.5), Err(StatsError::InvalidParameter { .. })));
}

// ============================================================================
// 4. Stratified 2 x 2 x K Cochran–Mantel–Haenszel — R 4.5.2 `mantelhaen.test`
// ============================================================================

#[test]
fn cmh_and_breslow_day_tarone_match_r_4_5_2() {
    // Same 2x2x3 array checked against R 4.5.2 `mantelhaen.test(arr, correct = TRUE/FALSE)`
    // Note R fills column-major: arr[1,1,k]=a, arr[2,1,k]=c, arr[1,2,k]=b, arr[2,2,k]=d
    let strata = vec![
        CmhStratum { label: "Center 1".into(), a: 10, b: 15, c: 20, d: 40 },
        CmhStratum { label: "Center 2".into(), a: 18, b: 10, c: 12, d: 25 },
        CmhStratum { label: "Center 3".into(), a: 8, b: 12, c: 14, d: 30 },
    ];
    let res = cochran_mantel_haenszel(&strata, ConfidenceLevel::DEFAULT_95).unwrap();

    assert_eq!(res.n_strata, 3);
    assert_eq!(res.informative_strata, 3);
    assert_eq!(res.total_count, 214);

    // R 4.5.2 `mantelhaen.test(..., correct = TRUE)`
    assert_close(res.cmh_chi2_corrected, 4.204_763_118_628, 1e-10, "CMH chi2 cc");
    assert_close(res.cmh_p_corrected, 0.040_310_604_067_860_4, 1e-11, "CMH p cc");
    // R 4.5.2 `mantelhaen.test(..., correct = FALSE)`
    assert_close(res.cmh_chi2_uncorrected, 4.838_281_403_520, 1e-10, "CMH chi2 no-cc");
    assert_close(res.cmh_p_uncorrected, 0.027_834_621_448_911_3, 1e-11, "CMH p no-cc");

    // Common OR and Robins–Breslow–Greenland 95% CI
    assert_close(res.common_or.unwrap(), 1.922_234_004_949, 1e-10, "Common OR MH");
    let (ci_lo, ci_hi) = res.common_or_ci.unwrap();
    assert_close(ci_lo, 1.074_285_184_428, 1e-10, "RBG CI lower");
    assert_close(ci_hi, 3.439_481_083_180, 1e-10, "RBG CI upper");

    // Breslow–Day & Tarone (1985) homogeneity
    assert_close(res.breslow_day_chi2.unwrap(), 2.455_039_348_729, 1e-10, "Breslow-Day chi2");
    assert_close(res.breslow_day_tarone_chi2.unwrap(), 2.454_974_903_233, 1e-10, "Breslow-Day-Tarone chi2");
    assert_close(res.breslow_day_df.unwrap(), 2.0, 1e-12, "Breslow-Day df");
    assert_close(res.breslow_day_tarone_p.unwrap(), 0.293_027_900_304, 1e-10, "Breslow-Day-Tarone p");

    // End-to-end ContingencyTable (K rows x 4 columns [a, b, c, d]) dispatch.
    let mut ct = ContingencyTable::new(4, 3).unwrap();
    ct.row_titles = vec!["Center 1".into(), "Center 2".into(), "Center 3".into()];
    let rows = [[10., 15., 20., 40.], [18., 10., 12., 25.], [8., 12., 14., 30.]];
    for (r, vals) in rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ds = DataSheet::new(SheetId::new("ct-cmh"), "CMH Strata", TableArchetype::Contingency(ct)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-cmh"), ds.id.clone(), AnalysisKind::CochranMantelHaenszel(CmhSpec::default())).unwrap();
    let rs = run_analysis(&ds, &spec).expect("run_analysis CMH on Contingency");
    assert_close(rs.scalars.iter().find(|s| s.key == "Common OR (Mantel-Haenszel)").unwrap().value, 1.922_234_004_949, 1e-10, "CMH scalar OR");

    // Degenerate edge cases: empty, zero-variance strata.
    assert!(matches!(cochran_mantel_haenszel(&[], ConfidenceLevel::DEFAULT_95), Err(StatsError::EmptyInput)));
    let zero_var = vec![CmhStratum { label: "Zero".into(), a: 5, b: 0, c: 0, d: 0 }];
    assert!(matches!(cochran_mantel_haenszel(&zero_var, ConfidenceLevel::DEFAULT_95), Err(StatsError::ZeroVariance { .. })));
}

#[test]
fn cmh_mv_excluded_third_level_does_not_corrupt_active_levels() {
    // Same 3 strata as the R 4.5.2 oracle above, plus row 0 containing an excluded 3rd-level
    // typo in exposure and row 1 containing an incomplete 3rd-level outcome (missing stratum).
    let raw_rows: &[(&str, &str, &str, Option<f64>, bool)] = &[
        ("TypoExposed", "Case", "Center 1", Some(99.0), true),
        ("Exposed", "TypoOutcome", "", Some(50.0), false),
        ("Exposed", "Case", "Center 1", Some(10.0), false),
        ("Exposed", "Control", "Center 1", Some(15.0), false),
        ("Unexposed", "Case", "Center 1", Some(20.0), false),
        ("Unexposed", "Control", "Center 1", Some(40.0), false),
        ("Exposed", "Case", "Center 2", Some(18.0), false),
        ("Exposed", "Control", "Center 2", Some(10.0), false),
        ("Unexposed", "Case", "Center 2", Some(12.0), false),
        ("Unexposed", "Control", "Center 2", Some(25.0), false),
        ("Exposed", "Case", "Center 3", Some(8.0), false),
        ("Exposed", "Control", "Center 3", Some(12.0), false),
        ("Unexposed", "Case", "Center 3", Some(14.0), false),
        ("Unexposed", "Control", "Center 3", Some(30.0), false),
    ];

    let mut mv = MultipleVariablesTable::new(4, raw_rows.len()).unwrap();
    mv.variables[0].title = String::from("Exposure");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Outcome");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Stratum");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Count");
    for (r, &(exp, out, strat, cnt, excl)) in raw_rows.iter().enumerate() {
        let mut c0 = Cell::text(exp);
        c0.excluded = excl;
        mv.variables[0].cells[r] = c0;
        mv.variables[1].cells[r] = Cell::text(out);
        if !strat.is_empty() {
            mv.variables[2].cells[r] = Cell::text(strat);
        }
        if let Some(v) = cnt {
            mv.variables[3].cells[r] = Cell::number(v);
        }
    }

    let ds = DataSheet::new(SheetId::new("mv-cmh-excl"), "CMH Excluded", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-cmh-excl"), ds.id.clone(), AnalysisKind::CochranMantelHaenszel(CmhSpec::default())).unwrap();
    let rs = run_analysis(&ds, &spec).expect("excluded/incomplete 3rd-level rows must not fail or corrupt CMH levels");
    assert_close(rs.scalars.iter().find(|s| s.key == "Common OR (Mantel-Haenszel)").unwrap().value, 1.922_234_004_949, 1e-10, "MV CMH OR");
    assert_close(
        rs.scalars.iter().find(|s| s.key == "CMH chi-square (continuity-corrected)").unwrap().value,
        4.204_763_118_628,
        1e-10,
        "MV CMH chi2",
    );
    assert_close(rs.scalars.iter().find(|s| s.key == "Total N").unwrap().value, 214.0, 1e-12, "MV CMH Total N");
}

#[test]
fn manova_boundary_df_e_equals_p_omits_nonpositive_hl_den_df_scalar() {
    // g = 3, p = 2, N = 5 => df_h = 2, df_e = 2 == p, s = 2: Hotelling-Lawley den_df <= 0.
    let mut mv = MultipleVariablesTable::new(3, 5).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Y1");
    mv.variables[2].title = String::from("Y2");
    let rows = [("A", 1.0, 2.0), ("A", 2.0, 4.5), ("B", 5.0, 1.0), ("B", 6.5, 2.0), ("C", 9.0, 7.0)];
    for (r, &(g, y1, y2)) in rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(g);
        mv.variables[1].cells[r] = Cell::number(y1);
        mv.variables[2].cells[r] = Cell::number(y2);
    }
    let ds = DataSheet::new(SheetId::new("mv-manova-bd"), "MANOVA Boundary", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(SheetId::new("res-manova-bd"), ds.id.clone(), AnalysisKind::Manova(ManovaSpec::default())).unwrap();
    let rs = run_analysis(&ds, &spec).expect("boundary df_e == p MANOVA succeeds");
    assert!(rs.scalars.iter().any(|s| s.key == "Hotelling-Lawley trace"));
    assert!(
        rs.scalars.iter().all(|s| s.key != "Hotelling-Lawley den df" && s.key != "Hotelling-Lawley F" && s.key != "Hotelling-Lawley P"),
        "non-positive Hotelling-Lawley den_df and NaN F/P must not be published as scalars: {:?}",
        rs.scalars
    );
    assert!(rs.scalars.iter().any(|s| s.key == "Pillai F" && s.value.is_finite()));
    assert!(rs.scalars.iter().any(|s| s.key == "Wilks F" && s.value.is_finite()));
}

#[test]
fn scheffe_honors_selected_pairs_and_control_column_on_rm_welch_and_factorial() {
    // 1. RM non-sphericity (4 conditions, 6 subjects) with selected_pairs & control_column.
    let mut rm_col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 4, 6).unwrap();
    let rm_data = [
        [10.0, 10.8, 11.2, 9.6, 10.4, 11.0],
        [13.0, 13.4, 14.2, 12.1, 13.8, 14.0],
        [16.2, 15.8, 17.1, 15.0, 16.5, 16.8],
        [20.5, 21.0, 22.1, 19.8, 21.4, 20.9],
    ];
    for (ci, vals) in rm_data.iter().enumerate() {
        rm_col.data_sets[ci].title = format!("T{ci}");
        for (ri, &v) in vals.iter().enumerate() {
            rm_col.data_sets[ci].cells[ri][0] = Cell::number(v);
        }
    }
    let rm_ds = DataSheet::new(SheetId::new("col-rm-sch"), "RM Scheffe", TableArchetype::Column(rm_col)).unwrap();

    // All pairs vs single selected_pair (0, 1): adjusted p must match identically (df1 = k - 1 = 3, NOT 1).
    let run_rm = |mult: MultiplicitySpec| {
        let spec = AnalysisSpec::new(
            SheetId::new("res-rm-sch"),
            rm_ds.id.clone(),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
                multiplicity: mult,
                ..OneWayAnovaSpec::default()
            }),
        )
        .unwrap();
        run_analysis(&rm_ds, &spec).expect("RM non-sphericity Scheffe")
    };
    let rs_rm_all = run_rm(MultiplicitySpec { method: MultiplicityMethod::Scheffe, ..Default::default() });
    let rs_rm_sel = run_rm(MultiplicitySpec { method: MultiplicityMethod::Scheffe, selected_pairs: Some(vec![(0, 1)]), ..Default::default() });
    let rs_rm_ctrl = run_rm(MultiplicitySpec { method: MultiplicityMethod::Scheffe, control_column: Some(0), ..Default::default() });
    assert_eq!(rs_rm_all.pairwise_comparisons.len(), 6);
    assert_eq!(rs_rm_sel.pairwise_comparisons.len(), 1, "RM non-sphericity Scheffe must honor selected_pairs");
    assert_eq!(rs_rm_ctrl.pairwise_comparisons.len(), 3, "RM non-sphericity Scheffe must honor control_column");
    let p_all_01 = rs_rm_all.pairwise_comparisons.iter().find(|p| p.group_a == 0 && p.group_b == 1).unwrap().raw_p_or_q;
    let p_sel_01 = rs_rm_sel.pairwise_comparisons[0].raw_p_or_q;
    assert_close(p_sel_01, p_all_01, 1e-12, "RM selected_pairs Scheffe keeps df1 = k - 1");

    // 2. Welch/Brown-Forsythe ANOVA (4 groups) with selected_pairs & control_column.
    let run_welch = |mult: MultiplicitySpec| {
        let spec = AnalysisSpec::new(
            SheetId::new("res-welch-sch"),
            rm_ds.id.clone(),
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::BrownForsytheAndWelch,
                multiplicity: mult,
                ..OneWayAnovaSpec::default()
            }),
        )
        .unwrap();
        run_analysis(&rm_ds, &spec).expect("Welch Scheffe")
    };
    let rs_w_all = run_welch(MultiplicitySpec { method: MultiplicityMethod::Scheffe, ..Default::default() });
    let rs_w_sel = run_welch(MultiplicitySpec { method: MultiplicityMethod::Scheffe, selected_pairs: Some(vec![(0, 1)]), ..Default::default() });
    let rs_w_ctrl = run_welch(MultiplicitySpec { method: MultiplicityMethod::Scheffe, control_column: Some(0), ..Default::default() });
    assert_eq!(rs_w_all.pairwise_comparisons.len(), 6);
    assert_eq!(rs_w_sel.pairwise_comparisons.len(), 1, "Welch Scheffe must honor selected_pairs");
    assert_eq!(rs_w_ctrl.pairwise_comparisons.len(), 3, "Welch Scheffe must honor control_column");
    let pw_all_01 = rs_w_all.pairwise_comparisons.iter().find(|p| p.group_a == 0 && p.group_b == 1).unwrap().raw_p_or_q;
    assert_close(rs_w_sel.pairwise_comparisons[0].raw_p_or_q, pw_all_01, 1e-12, "Welch selected_pairs Scheffe keeps df1 = k - 1");

    // 3. Factorial ANOVA (2 rows x 3 columns = 6 cells, 4 replicates per cell).
    let mut gt = GroupedTable::new(SubcolumnMode::RawReplicates(4), 3, 2).unwrap();
    gt.row_titles = vec!["R1".into(), "R2".into()];
    for c in 0..3 {
        gt.data_sets[c].title = format!("C{}", c + 1);
    }
    let vals_2x3: [[[f64; 4]; 3]; 2] = [
        [[10.0, 10.6, 9.8, 10.4], [12.0, 12.8, 11.7, 12.3], [15.1, 15.9, 14.8, 15.4]],
        [[11.0, 11.5, 10.7, 11.2], [13.4, 14.0, 13.1, 13.7], [17.2, 18.0, 16.9, 17.5]],
    ];
    for r in 0..2 {
        for c in 0..3 {
            for s in 0..4 {
                gt.data_sets[c].cells[r][s] = Cell::number(vals_2x3[r][c][s]);
            }
        }
    }
    let fac_ds = DataSheet::new(SheetId::new("grp-fac-sch"), "Factorial Scheffe", TableArchetype::Grouped(gt)).unwrap();
    let run_fac = |mode: FactorialComparisonFamilyMode, pre: Vec<(usize, usize)>, mult: MultiplicitySpec| {
        let spec = AnalysisSpec::new(
            SheetId::new("res-fac-sch"),
            fac_ds.id.clone(),
            AnalysisKind::FactorialAnova(FactorialAnovaSpec {
                comparison_family_mode: mode,
                preselected_pairs: pre,
                multiplicity: mult,
                ..FactorialAnovaSpec::default()
            }),
        )
        .unwrap();
        run_analysis(&fac_ds, &spec).expect("Factorial Scheffe")
    };

    // (a) PreselectedPairs with single pair (0, 1) must use df1 = total_cells - 1 = 5 (NOT 1).
    let rs_pre = run_fac(
        FactorialComparisonFamilyMode::PreselectedPairs,
        vec![(0, 1)],
        MultiplicitySpec { method: MultiplicityMethod::Scheffe, ..Default::default() },
    );
    assert_eq!(rs_pre.pairwise_comparisons.len(), 1);
    let rs_pre_lsd = run_fac(
        FactorialComparisonFamilyMode::PreselectedPairs,
        vec![(0, 1)],
        MultiplicitySpec { method: MultiplicityMethod::FisherLsd, ..Default::default() },
    );
    let p_pre_sch = rs_pre.pairwise_comparisons[0].raw_p_or_q;
    let p_pre_lsd = rs_pre_lsd.pairwise_comparisons[0].raw_p_or_q;
    assert!(
        p_pre_sch > p_pre_lsd,
        "PreselectedPairs single-pair Scheffe ({p_pre_sch}) must strictly exceed Fisher LSD ({p_pre_lsd}) because df1 = total_cells - 1 = 5"
    );

    // (b) TwoFactorFamilies vs PerSliceFamilies: TwoFactorFamilies honors (n_slices = 2, k_var = 3)
    //     with Sidak across n_slices = 2 slices (1 - (1 - p_slice)^2), strictly more conservative than PerSliceFamilies.
    let rs_two = run_fac(
        FactorialComparisonFamilyMode::TwoFactorFamilies,
        Vec::new(),
        MultiplicitySpec { method: MultiplicityMethod::Scheffe, ..Default::default() },
    );
    let rs_slice = run_fac(
        FactorialComparisonFamilyMode::PerSliceFamilies,
        Vec::new(),
        MultiplicitySpec { method: MultiplicityMethod::Scheffe, ..Default::default() },
    );
    let p_two_0 = rs_two.pairwise_comparisons[0].raw_p_or_q;
    let p_slice_0 = rs_slice.pairwise_comparisons[0].raw_p_or_q;
    let expected_sidak_two = 1.0 - (1.0 - p_slice_0).powi(2);
    assert_close(p_two_0, expected_sidak_two, 1e-12, "TwoFactorFamilies Scheffe Sidak across n_slices=2");

    // (c) Verify exact df1 = 5 on PreselectedPairs directly against f_sf(t^2 / 5, 5, 18).
    let df_e_fac = 18.0_f64;
    let mut ss_res = 0.0_f64;
    for row in &vals_2x3 {
        for cell in row {
            let m = cell.iter().sum::<f64>() / 4.0;
            ss_res += cell.iter().map(|&v| (v - m).powi(2)).sum::<f64>();
        }
    }
    let ms_res = ss_res / df_e_fac;
    let t_cell = (10.2 - 12.2) / (2.0 * ms_res / 4.0).sqrt();
    let expected_pre_p = f_sf((t_cell * t_cell) / 5.0, 5.0, df_e_fac).unwrap();
    assert_close(p_pre_sch, expected_pre_p, 1e-10, "PreselectedPairs Scheffe exact df1 = 5");
}
