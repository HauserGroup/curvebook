//! Consolidated regression, security/DoS overflow guard, and D1 reference-oracle tests for
//! categorical/contingency/parts-of-whole, kappa, ROC, and power analyses.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::f64::consts::PI;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use curvebook_doc::{
    AnalysisKind, ChecklistStatus, ContingencyMethod, ContingencySpec, KappaWeighting, PowerAnalysisSpec, ResultCell, RocCiMethod, RocSpec, TableKind,
};
use curvebook_math::dist::norm_sf;
use curvebook_math::rng::DeterministicRng;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::StatsError;
use curvebook_stats::categorical::{
    BarnardOrdering, analyze_2x2_ratios, analyze_contingency, baptista_pike_or_ci, barnard_exact_2x2_with_ordering, clopper_pearson_ci,
    cochran_armitage_trend, cohens_kappa, exact_binomial_test, fisher_exact_2x2, fisher_exact_rxc, fleiss_kappa, mcnemar_detailed, mcnemar_test,
};
use curvebook_stats::power::{
    CorrelationPowerMethod, OneSidedDirection, PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, TailMode,
    TwoProportionsMethod, compute_power_curve, compute_sensitivity_table, solve_power,
};
use curvebook_stats::roc::{compare_roc_delong_paired, roc_curve_analysis};
use curvebook_testkit::assert_rel_close;

use crate::common::{
    assert_all_sheet_numbers_finite, assert_close, cell_num, cell_p, col_sheet, contingency_sheet, parts_multi_sheet, parts_sheet, run_kind,
    run_kind_sel, scalar, scalar_ds, scalar_opt, tab_by_id,
};

fn run_with_timeout<F, R>(timeout: Duration, f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(timeout).expect("operation exceeded timeout (possible hang)")
}

fn ct_kind(method: ContingencyMethod) -> AnalysisKind {
    AnalysisKind::Contingency(ContingencySpec { method, ..Default::default() })
}

#[test]
fn audit_security_categorical_overflow_hang_and_dos_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Huge 2x2 counts must return Err cleanly without CPU hang + u64 overflow guards + exact accuracy.
    for (a, b, c, d, expected_name) in [
        (1_000_000_000_u64, 1_000_000_000, 1_000_000_000, 1_000_000_000, "fisher_exact_max_total"),
        (1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, "fisher_exact_max_total"),
        (1, 1_000_000_000_000, 1, 1_000_000_000_000, "fisher_exact_max_total"),
        (u64::MAX, 1, 0, 0, "fisher_exact_row1_total"),
        (0, 0, u64::MAX, 1, "fisher_exact_row2_total"),
        (1_u64 << 63, 0, 1_u64 << 63, 0, "fisher_exact_col1_total"),
        (1_u64 << 63, 0, 0, 1_u64 << 63, "fisher_exact_table_total"),
    ] {
        let res = run_with_timeout(Duration::from_secs(3), move || fisher_exact_2x2(a, b, c, d));
        assert!(matches!(res, Err(StatsError::InvalidParameter { name, .. }) if name == expected_name), "({a},{b},{c},{d}) -> {res:?}");
    }
    assert_close(fisher_exact_2x2(0, 0, 0, 0).unwrap(), 1.0, 1e-12, "fisher 0");
    assert_close(fisher_exact_2x2(14, 6, 5, 15).unwrap(), 0.010_386_702_198_860_637, 1e-12, "fisher 14,6,5,15");

    // Large-N symmetric / twin-mode / mirror Fisher 2x2 & modal 3x3 (RV-01).
    assert_eq!(fisher_exact_2x2(50_000, 50_000, 50_000, 50_000).unwrap(), 1.0);
    assert_eq!(fisher_exact_2x2(50_000, 50_001, 50_000, 49_999).unwrap(), 1.0);
    assert_eq!(fisher_exact_2x2(50_001, 50_000, 49_999, 50_000).unwrap(), 1.0);
    for k in [0_u64, 150, 300, 450] {
        let p1 = fisher_exact_2x2(40_000 - k, 40_000 + k, 60_000 + k, 60_000 - k).unwrap();
        let p2 = fisher_exact_2x2(40_000 + k, 40_000 - k, 60_000 - k, 60_000 + k).unwrap();
        assert!((p1 - p2).abs() <= 1e-13);
    }
    assert_eq!(fisher_exact_rxc(&vec![vec![1_000; 3]; 3]).unwrap(), 1.0);

    // 2. RxC panic/OOM/overflow/ragged guards & cap checks.
    let overflow_rxc = vec![vec![1_u64 << 63, 1_u64 << 63], vec![0, 5], vec![0, 0]];
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze_contingency(&overflow_rxc, false, conf)));
    assert!(caught.is_ok() && caught.unwrap().is_err());
    assert!(fisher_exact_rxc(&overflow_rxc).is_err());
    for over in [
        vec![vec![1_u64 << 62, 1_u64 << 62], vec![0, 5], vec![0, 0]],
        vec![vec![10_000, 10_000, 10_000], vec![10_000, 5_000, 5_001]],
        vec![vec![2_000_000, 2_000_000], vec![1_000_000, 1_000_000]],
    ] {
        assert!(matches!(fisher_exact_rxc(&over), Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));
    }
    assert!(matches!(fisher_exact_rxc(&[vec![1, 2], vec![3]]), Err(StatsError::Math(MathError::DimensionMismatch { expected: 2, actual: 1 }))));
    assert!((0.0..=0.01).contains(&fisher_exact_rxc(&[vec![10, 2, 1], vec![2, 10, 2], vec![1, 2, 10]]).unwrap()));
}

#[test]
fn audit_security_mcnemar_contingency_and_proportion_overflow_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;
    // 3. McNemar overflow/cap guards + O(1) symmetric + exact accuracy.
    assert!(matches!(mcnemar_test(u64::MAX, 1), Err(StatsError::InvalidParameter { name: "mcnemar_discordant_total", .. })));
    assert!(matches!(mcnemar_detailed(u64::MAX, 1, conf), Err(StatsError::InvalidParameter { name: "mcnemar_discordant_total", .. })));
    assert!(matches!(
        run_with_timeout(Duration::from_secs(2), || mcnemar_test(1_000_000_000, 1_000_000_000)),
        Err(StatsError::InvalidParameter { name: "mcnemar_max_total", .. })
    ));
    assert!(matches!(
        run_with_timeout(Duration::from_secs(2), move || mcnemar_detailed(3_000_000, 3_000_000, conf)),
        Err(StatsError::InvalidParameter { name: "mcnemar_max_total", .. })
    ));
    assert_eq!(mcnemar_test(2_000_000, 2_000_000).unwrap(), (0.0, 1.0));
    let (mc_chi2, mc_p) = mcnemar_test(18, 5).unwrap();
    assert_close(mc_chi2, 144.0 / 23.0, 1e-12, "McNemar chi2");
    assert_close(mc_p, 89_104.0 / 8_388_608.0, 1e-12, "McNemar exact p");
}

#[test]
fn audit_security_contingency_ratios_kappa_and_binomial_overflow_guards() {
    let conf = ConfidenceLevel::DEFAULT_95;
    // 4. analyze_contingency & run_analysis huge/overflowing counts + proportion/kappa/ratio/CA guards.
    for bad in [
        vec![vec![u64::MAX, 1], vec![0, 5]],
        vec![vec![1_u64 << 63, 0], vec![1_u64 << 63, 0]],
        vec![vec![1_u64 << 63, 0], vec![0, 1_u64 << 63]],
        vec![vec![1, 2], vec![3]],
    ] {
        assert!(analyze_contingency(&bad, false, conf).is_err());
    }
    let huge = vec![vec![1_000_000_000_000_u64; 2]; 2];
    let r_huge = run_with_timeout(Duration::from_secs(2), move || analyze_contingency(&huge, false, conf).unwrap());
    assert_eq!(
        (r_huge.total_count, r_huge.chi_square, r_huge.chi_square_p, r_huge.fisher_exact_p, r_huge.mcnemar_exact_p),
        (4_000_000_000_000, 0.0, 1.0, None, None)
    );

    for (rows, method) in [
        (&[&[1_u64 << 63, 1_u64 << 63][..], &[0, 5], &[0, 0]][..], ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        (&[&[10_000_000_u64, 10_000_000][..], &[10_000_000, 10_000_000]][..], ContingencyMethod::FisherExact),
        (&[&[0_u64, 10_000_000][..], &[10_000_000, 0]][..], ContingencyMethod::McNemarPaired),
        (&[&[1_u64 << 63, 1_u64 << 63][..]][..], ContingencyMethod::ExactBinomial),
        (&[&[3_000_000_u64, 3_000_000][..]][..], ContingencyMethod::ExactBinomial),
        (&[&[1_u64 << 63, 1_u64 << 63][..], &[0, 5]][..], ContingencyMethod::BarnardExact),
    ] {
        assert!(run_kind(&contingency_sheet("sec-ct", rows), ct_kind(method)).is_err());
    }
    assert!(
        run_kind(
            &contingency_sheet("sec-ok", &[&[1_000_000_000_000, 1_000_000_000_000], &[1_000_000_000_000, 1_000_000_000_000]]),
            ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: false })
        )
        .is_ok()
    );

    assert_eq!(baptista_pike_or_ci(u64::MAX, 1, 1, 1, conf), None);
    let (bp_lo, bp_hi) = run_with_timeout(Duration::from_secs(2), move || {
        baptista_pike_or_ci(1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, conf)
    })
    .unwrap();
    assert!(bp_lo > 0.999_99 && bp_lo < 1.0 && bp_hi > 1.0 && bp_hi < 1.000_01 && (bp_lo * bp_hi - 1.0).abs() < 1e-11);
    assert!(analyze_2x2_ratios(u64::MAX, 1, 1, 1, conf).is_err());
    assert!(analyze_2x2_ratios(1_u64 << 63, 0, 0, 1_u64 << 63, conf).is_err());
    let r_big = run_with_timeout(Duration::from_secs(2), move || {
        analyze_2x2_ratios(1_000_000_000, 1_000_000_000, 1_000_000_000, 1_000_000_000, conf).unwrap()
    });
    let (rlo, rhi) = r_big.or_baptista_pike_ci.unwrap();
    assert!(rlo > 0.999_8 && rhi < 1.000_2 && (rlo * rhi - 1.0).abs() < 1e-10);
    assert!(clopper_pearson_ci(10, 5, conf).is_err());
    assert!(matches!(exact_binomial_test(1, 5_000_001, 0.5, conf), Err(StatsError::InvalidParameter { name: "binomial_max_total", .. })));
    assert!(cohens_kappa(&[vec![1_u64 << 63, 1_u64 << 63], vec![0, 0]], KappaWeighting::Unweighted, conf).is_err());
    assert!(matches!(cohens_kappa(&[vec![0, 0], vec![0, 0]], KappaWeighting::Unweighted, conf), Err(StatsError::EmptyInput)));
    assert!(fleiss_kappa(&[vec![1_u64 << 63, 1_u64 << 63], vec![1_u64 << 63, 1_u64 << 63]], conf).is_err());
    assert!(fleiss_kappa(&[vec![0, 0, 2], vec![1_u64 << 63, 1_u64 << 63, 2]], conf).is_err());
    assert!(matches!(
        cochran_armitage_trend(&[vec![1, 2], vec![3], vec![4, 5]]),
        Err(StatsError::Math(MathError::DimensionMismatch { expected: 2, actual: 1 }))
    ));
}

#[test]
fn audit_17_18_22_88_89_contingency_parts_boschloo_csm_and_baptista_pike() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 1. Empty margin reduction (#17) against R 4.5.2 oracles + <2x2 rejection.
    let r_pr = analyze_contingency(&[vec![10, 2], vec![3, 9], vec![0, 0]], false, conf).unwrap();
    assert_eq!((r_pr.n_rows, r_pr.n_cols, r_pr.dropped_empty_rows, r_pr.dropped_empty_cols), (2, 2, 1, 0));
    assert_close(r_pr.chi_square, 8.223_776_223_776_223, 1e-12, "padded row chi2");
    assert_close(r_pr.chi_square_p, 0.004_134_507_669_588_785, 1e-12, "padded row P");
    let r_pc = analyze_contingency(&[vec![10, 2, 0], vec![3, 9, 0], vec![4, 4, 0]], false, conf).unwrap();
    assert_eq!((r_pc.n_rows, r_pc.n_cols, r_pc.dropped_empty_rows, r_pc.dropped_empty_cols), (3, 2, 0, 1));
    assert_close(r_pc.chi_square, 8.240_522_875_816_994, 1e-12, "padded col chi2");
    assert_close(r_pc.chi_square_p, 0.016_240_268_065_175_74, 1e-12, "padded col P");
    assert_close(r_pc.cramers_v, 0.507_460_678_150_811_1, 1e-12, "padded col V");

    let sh_sub = contingency_sheet("ct-sub", &[&[10, 2, 1], &[3, 9, 1], &[0, 0, 8]]);
    let out_sub = run_kind_sel(&sh_sub, ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: true }), vec![0, 1]).unwrap();
    let mt = tab_by_id(&out_sub, "contingency-main");
    assert_eq!(mt.row_labels[0], "Chi-square (Yates)");
    assert!(mt.footnotes.iter().any(|f| f.contains("empty (all-zero) row(s)") && f.contains("2 x 2")));
    assert!(out_sub.checklist.iter().any(|c| c.assumption == "Empty margin reduction" && c.status == ChecklistStatus::Info));

    for bad in [vec![vec![5_u64, 0], vec![3, 0]], vec![vec![5, 3], vec![0, 0]], vec![vec![0, 0, 0], vec![0, 5, 0], vec![0, 0, 0]]] {
        assert!(matches!(analyze_contingency(&bad, false, conf), Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })));
        let rows: Vec<&[u64]> = bad.iter().map(Vec::as_slice).collect();
        assert!(matches!(
            run_kind(&contingency_sheet("b", &rows), ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: false })),
            Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
        ));
    }
    assert!(matches!(cochran_armitage_trend(&[vec![5, 0], vec![3, 0], vec![2, 0]]), Err(StatsError::InvalidParameter { .. })));
    assert!(matches!(cochran_armitage_trend(&[vec![10, 2], vec![3, 9], vec![0, 0]]), Err(StatsError::InvalidParameter { .. })));

    // 2. Undefined 0/0 2x2 ratios (#18) + extreme Baptista-Pike OR CI (S3 #2).
    let r_c2 = analyze_2x2_ratios(5, 0, 3, 0, conf).unwrap();
    assert_eq!((r_c2.odds_ratio, r_c2.or_baptista_pike_ci, r_c2.specificity, r_c2.specificity_wilson_ci), (None, None, None, None));
    assert_close(r_c2.relative_risk.unwrap(), 1.0, 1e-12, "5,0,3,0 RR");
    let r_c1 = analyze_2x2_ratios(0, 5, 0, 3, conf).unwrap();
    assert_eq!((r_c1.odds_ratio, r_c1.relative_risk, r_c1.sensitivity, r_c1.sensitivity_wilson_ci), (None, None, None, None));
    assert_eq!(analyze_2x2_ratios(5, 0, 1, 3, conf).unwrap().odds_ratio, Some(f64::INFINITY));
    assert_eq!(analyze_2x2_ratios(5, 5, 0, 3, conf).unwrap().relative_risk, Some(f64::INFINITY));

    let (bp_t_lo, bp_t_hi) = baptista_pike_or_ci(1, 1000, 1000, 1, conf).unwrap();
    assert!(bp_t_lo > 0.0 && bp_t_lo < 1e-6 && bp_t_hi > 1e-6 && bp_t_hi < 1.0);
    let ((l1, h1), (l2, h2)) = (baptista_pike_or_ci(1, 100, 100, 1, conf).unwrap(), baptista_pike_or_ci(100, 1, 1, 100, conf).unwrap());
    assert!((l1 * h2 - 1.0).abs() < 1e-6 && (h1 * l2 - 1.0).abs() < 1e-6);
    assert_eq!(baptista_pike_or_ci(0, 50, 2, 100, conf).unwrap().0, 0.0);
    assert!(baptista_pike_or_ci(2, 0, 50, 50, conf).unwrap().1.is_infinite());
    let bp_sh = run_kind(
        &contingency_sheet("bp-z", &[&[2, 0], &[50, 50]]),
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            ..Default::default()
        }),
    )
    .unwrap();
    assert_all_sheet_numbers_finite(&bp_sh, "BP zero cell");
}

#[test]
fn audit_22_contingency_method_df_yates_label_and_no_silent_fallback() {
    // 3. Method-specific df, Yates label (#22), & no silent chi-square fallback (R1-S02-02).
    let ca_res =
        run_kind(&contingency_sheet("ca", &[&[15, 5], &[10, 10], &[8, 12], &[3, 17]]), ct_kind(ContingencyMethod::CochranArmitageTrend)).unwrap();
    let ca_t = tab_by_id(&ca_res, "contingency-main");
    assert_eq!(ca_t.row_labels[0], "Cochran-Armitage trend");
    assert_close(cell_num(&ca_t.rows[0][0]).abs(), 3.819_143_697_985_006, 1e-10, "CA |z|");
    // AUD-05-60: Cochran-Armitage emits signed standard normal z (not z^2), so df is Empty.
    assert_eq!(ca_t.rows[0][1], ResultCell::Empty);
    assert_close(cell_p(&ca_t.rows[0][2]), 0.000_133_915_738_267_329_4, 1e-12, "CA P");

    let s2 = contingency_sheet("s2", &[&[14, 6], &[5, 15]]);
    let s3 = contingency_sheet("s3", &[&[10, 4, 2], &[3, 12, 5], &[1, 3, 14]]);
    for (sh, m) in [(&s2, ContingencyMethod::FisherExact), (&s2, ContingencyMethod::BarnardExact), (&s3, ContingencyMethod::FisherExact)] {
        assert_eq!(tab_by_id(&run_kind(sh, ct_kind(m)).unwrap(), "contingency-main").rows[0][1], ResultCell::Empty);
    }
    let y3 = run_kind(&s3, ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: true })).unwrap();
    assert_eq!(tab_by_id(&y3, "contingency-main").row_labels[0], "Pearson chi-square");
    assert_close(cell_num(&tab_by_id(&y3, "contingency-main").rows[0][1]), 4.0, 1e-12, "3x3 df");

    for (rows, method) in [
        (&[&[10_u64, 5, 2][..], &[4, 8, 6], &[1, 3, 9]][..], ContingencyMethod::CochranArmitageTrend),
        (&[&[10_u64, 5][..], &[4, 8]][..], ContingencyMethod::CochranArmitageTrend),
        (&[&[5_u64, 10, 15][..], &[12, 8, 5]][..], ContingencyMethod::BarnardExact),
        (&[&[1_500_u64, 1_500][..], &[1_500, 1_500]][..], ContingencyMethod::BarnardExact),
        (&[&[1_600_u64, 1_400][..], &[1_400, 1_600]][..], ContingencyMethod::BarnardExact),
        (&[&[10_000_u64, 10_000, 10_000][..], &[10_000, 10_000, 10_000], &[1_000, 1_000, 1_000]][..], ContingencyMethod::FisherExact),
    ] {
        assert!(matches!(run_kind(&contingency_sheet("bad-ct", rows), ct_kind(method)), Err(StatsError::InvalidParameter { .. })));
    }
    let r7 = [2_u64; 7];
    let f7 = run_kind(&contingency_sheet("s7", &[&r7[..]; 7]), ct_kind(ContingencyMethod::FisherExact)).unwrap();
    assert_eq!(f7.tabs[0].row_labels[0], "Fisher's exact test (Monte Carlo)");
    assert_eq!(scalar_opt(&f7, "Fisher's exact Monte Carlo draws", None), Some(10_000.0));
}

#[test]
fn audit_88_89_parts_of_whole_and_extreme_tail_boschloo_csm() {
    // 4. PartsOfWhole method compatibility, G2 dispatch, G^2 clamp, structural zero df, integer & multi-series (#88/#89/FA-SEC05-01/R2-S02-12).
    let pow3 = parts_sheet("pow3", &[("A", 50.0), ("B", 30.0), ("C", 20.0)]);
    for bad_m in [
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear },
        ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic },
        ContingencyMethod::FleissKappa,
        ContingencyMethod::FisherExact,
        ContingencyMethod::BarnardExact,
        ContingencyMethod::CochranArmitageTrend,
        ContingencyMethod::McNemarPaired,
        ContingencyMethod::PearsonChiSquare { yates_continuity: true },
    ] {
        assert!(matches!(run_kind(&pow3, ct_kind(bad_m)), Err(StatsError::IncompatibleTable { actual: TableKind::PartsOfWhole, .. })));
    }
    let exp_g2 = 2.0 * (50.0 * (50.0 / (100.0 / 3.0_f64)).ln() + 30.0 * (30.0 / (100.0 / 3.0_f64)).ln() + 20.0 * (20.0 / (100.0 / 3.0_f64)).ln());
    let rg2 = run_kind(&pow3, ct_kind(ContingencyMethod::LikelihoodRatioG2)).unwrap();
    assert_eq!(tab_by_id(&rg2, "parts-gof").row_labels, vec!["Likelihood-ratio G2", "Pearson chi-square"]);
    assert_close(scalar_ds(&rg2, "P value", None), (-exp_g2 / 2.0).exp(), 1e-12, "PoW G2 P");
    assert_close(scalar_ds(&rg2, "Chi-square P value", None), (-7.0_f64).exp(), 1e-12, "PoW Chi2 P");

    let pow_g2_exact = parts_sheet("pow-ex", &[("S1", 1.0), ("S2", 5.0), ("S3", 29.0)]);
    let r_ex = run_kind(
        &pow_g2_exact,
        AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![1.0, 5.0, 29.0]), ..Default::default() }),
    )
    .unwrap();
    assert_eq!(scalar_ds(&r_ex, "Likelihood-ratio G-squared", None), 0.0);
    let pow_sz = parts_sheet("pow-sz", &[("C1", 10.0), ("C2", 20.0), ("C3", 30.0), ("C4", 0.0)]);
    let r_sz =
        run_kind(&pow_sz, AnalysisKind::Contingency(ContingencySpec { expected_proportions: Some(vec![1.0, 1.0, 1.0, 0.0]), ..Default::default() }))
            .unwrap();
    assert_close(cell_num(&tab_by_id(&r_sz, "parts-gof").rows[0][1]), 2.0, 1e-12, "structural zero df");

    for (sh, m) in [
        (parts_sheet("f1", &[("A", 0.5), ("B", 0.3), ("C", 0.2)]), ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
        (parts_sheet("f2", &[("A", 10.0), ("B", 20.5), ("C", 30.0)]), ContingencyMethod::LikelihoodRatioG2),
    ] {
        assert!(matches!(run_kind(&sh, ct_kind(m)), Err(StatsError::InvalidParameter { name: "goodness_of_fit_count", .. })));
    }
    let multi_gof = parts_multi_sheet("mg", &["A", "B", "C"], &[("S0", &[50.0, 30.0, 20.0]), ("S1", &[34.0, 33.0, 33.0])]);
    let r0 = run_kind_sel(&multi_gof, ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: false }), vec![0]).unwrap();
    assert_close(scalar_ds(&r0, "Goodness-of-fit chi-square", None), 14.0, 1e-12, "GOF S0 chi2");
    assert_close(scalar_ds(&r0, "P value", None), 0.000_911_881_965_554_515_4, 1e-12, "GOF S0 P");
    let r1 = run_kind_sel(&multi_gof, ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: false }), vec![1]).unwrap();
    assert_close(scalar_ds(&r1, "Goodness-of-fit chi-square", None), 0.02, 1e-12, "GOF S1 chi2");
    assert_close(scalar_ds(&r1, "P value", None), 0.990_049_833_749_168, 1e-12, "GOF S1 P");
    for (sel, is_oob) in [(vec![0, 1], false), (vec![], false), (vec![5], true)] {
        let err = run_kind_sel(&multi_gof, ct_kind(ContingencyMethod::PearsonChiSquare { yates_continuity: false }), sel).unwrap_err();
        assert!(if is_oob {
            matches!(err, StatsError::MissingColumn { index: 5 })
        } else {
            matches!(err, StatsError::InvalidParameter { name: "selected_data_sets", .. })
        });
    }
    let multi_bin = parts_multi_sheet("mb", &["S", "F"], &[("Skew", &[9.0, 1.0]), ("Bal", &[5.0, 5.0])]);
    let b0 = run_kind_sel(&multi_bin, ct_kind(ContingencyMethod::ExactBinomial), vec![0]).unwrap();
    assert_close(scalar_ds(&b0, "successes", None), 9.0, 1e-12, "Bin S0 successes");
    assert_close(scalar_ds(&b0, "p_value", None), 0.021_484_375, 1e-12, "Bin S0 P");
    let b1 = run_kind_sel(&multi_bin, ct_kind(ContingencyMethod::ExactBinomial), vec![1]).unwrap();
    assert_close(scalar_ds(&b1, "successes", None), 5.0, 1e-12, "Bin S1 successes");
    assert_close(scalar_ds(&b1, "p_value", None), 1.0, 1e-12, "Bin S1 P");
    for (sel, is_oob) in [(vec![0, 1], false), (vec![], false), (vec![3], true)] {
        let err = run_kind_sel(&multi_bin, ct_kind(ContingencyMethod::ExactBinomial), sel).unwrap_err();
        assert!(if is_oob {
            matches!(err, StatsError::MissingColumn { index: 3 })
        } else {
            matches!(err, StatsError::InvalidParameter { name: "selected_data_sets", .. })
        });
    }

    // 5. Extreme-tail Boschloo & CSM D1 Python oracles (FA-SEC05-04) + signed Z symmetry (FA-SEC05-13).
    assert_rel_close(fisher_exact_2x2(40, 0, 0, 40).unwrap(), 1.860_340_365_603_626_4e-23, 1e-10, "Fisher 40");
    for (a, b, c, d, ord, exp_p, rel, exp_pi) in [
        (40, 0, 0, 40, BarnardOrdering::Boschloo, 1.654_361_225_106_045e-24, 1e-10, Some((0.5, 1e-6))),
        (30, 0, 1, 30, BarnardOrdering::Boschloo, 5.377_642_775_528_156e-17, 1e-9, None),
        (25, 0, 0, 40, BarnardOrdering::Boschloo, 1.555_645_477_983_303_6e-19, 1e-8, Some((0.384_817_95, 1e-4))),
        (30, 0, 1, 15, BarnardOrdering::Boschloo, 5.846_276_880_745_333e-12, 1e-8, Some((0.659_205_58, 1e-4))),
        (30, 5, 0, 35, BarnardOrdering::Csm, 2.150_379_483_853_381e-15, 1e-8, None),
        (28, 3, 0, 31, BarnardOrdering::Csm, 4.746_203_430_272_416e-15, 1e-8, None),
    ] {
        let r = barnard_exact_2x2_with_ordering(a, b, c, d, ord).unwrap();
        assert!(!r.used_asymptotic_fallback);
        assert_rel_close(r.p_value, exp_p, rel, &format!("{ord:?} ({a},{b},{c},{d})"));
        if let Some((pi, tol)) = exp_pi {
            assert_close(r.nuisance_pi, pi, tol, "nuisance pi");
        }
    }
    let bp = barnard_exact_2x2_with_ordering(10, 2, 3, 9, BarnardOrdering::Boschloo).unwrap();
    let bn = barnard_exact_2x2_with_ordering(2, 10, 9, 3, BarnardOrdering::Boschloo).unwrap();
    assert!(bp.statistic > 0.0 && bn.statistic < 0.0);
    assert_close(bp.statistic, -bn.statistic, 1e-12, "Barnard signed Z");
}

#[test]
fn audit_90_kappa_se_coverage_pe_one_and_erfc_tail() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let z95 = 1.959_963_984_540_054;

    // 1. Cohen's kappa Fleiss-Cohen-Everitt (1969) SE oracles (R2-S02-02) + p_e == 1 ZeroVariance + erfc tail (S3 #3/#5).
    let k2 = cohens_kappa(&[vec![45, 5], vec![5, 45]], KappaWeighting::Unweighted, conf).unwrap();
    assert_close(k2.kappa, 0.8, 1e-12, "2x2 kappa");
    assert_close(k2.se, 0.06, 1e-12, "2x2 kappa SE");
    assert_close(k2.ci.0, 0.8 - z95 * 0.06, 1e-10, "2x2 kappa CI lo");
    let m3 = vec![vec![40, 8, 2], vec![6, 30, 4], vec![1, 3, 26]];
    for (wt, exp_k, exp_se) in [
        (KappaWeighting::Unweighted, 0.695_238_095_238_095, 0.055_822_163_014_395_6),
        (KappaWeighting::Linear, 0.739_967_897_271_268, 0.051_171_359_296_692_1),
        (KappaWeighting::Quadratic, 0.785_714_285_714_286, 0.051_734_596_988_002_6),
    ] {
        let r = cohens_kappa(&m3, wt, conf).unwrap();
        assert_close(r.kappa, exp_k, 1e-12, "3x3 kappa");
        assert_close(r.se, exp_se, 1e-12, "3x3 SE");
    }
    let sh_k = run_kind(
        &contingency_sheet("k3", &[&[40, 8, 2], &[6, 30, 4], &[1, 3, 26]]),
        ct_kind(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Quadratic }),
    )
    .unwrap();
    assert_close(scalar_ds(&sh_k, "se", None), 0.051_734_596_988_002_6, 1e-12, "sheet kappa SE");

    let single = [vec![12_u64, 0, 0], vec![0, 0, 0], vec![0, 0, 0]];
    for wt in [KappaWeighting::Unweighted, KappaWeighting::Linear, KappaWeighting::Quadratic] {
        assert!(matches!(cohens_kappa(&single, wt, conf), Err(StatsError::ZeroVariance { .. })));
    }
    assert!(matches!(fleiss_kappa(&vec![vec![5, 0]; 6], conf), Err(StatsError::ZeroVariance { .. })));
    assert!(matches!(
        run_kind(&contingency_sheet("k1", &[&[12, 0], &[0, 0]]), ct_kind(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted })),
        Err(StatsError::ZeroVariance { .. })
    ));
    let strong = cohens_kappa(&[vec![100, 0], vec![0, 100]], KappaWeighting::Unweighted, conf).unwrap();
    assert!(strong.z_statistic > 14.0 && strong.p_value > 0.0 && strong.p_value < 1e-40);
    assert!((strong.p_value - 2.0 * norm_sf(strong.z_statistic.abs())).abs() <= 1e-55);

    // 2. Fleiss's kappa non-null SE (Fleiss, Levin & Paik 2003) & 3,000-rep Monte Carlo coverage (#90).
    let null_se = |counts: &[Vec<u64>]| -> f64 {
        let (n, k, mf) = (counts.len() as f64, counts[0].len(), counts[0].iter().sum::<u64>() as f64);
        let mut cs = vec![0.0; k];
        for row in counts {
            for (j, &c) in row.iter().enumerate() {
                cs[j] += c as f64;
            }
        }
        let pj: Vec<f64> = cs.iter().map(|&s| s / (n * mf)).collect();
        let pe: f64 = pj.iter().map(|&p| p * p).sum();
        let pj3: f64 = pj.iter().map(|&p| p * p * p).sum();
        // AUD-05-04: Fleiss, Nee & Landis (1979) asymptotic null variance under H0: kappa = 0.
        (2.0 * (pe + pe * pe - 2.0 * pj3) / (n * mf * (mf - 1.0) * (1.0 - pe).powi(2))).max(0.0).sqrt()
    };
    let fixed: Vec<Vec<u64>> = [
        [4, 0, 0],
        [3, 1, 0],
        [4, 0, 0],
        [2, 2, 0],
        [4, 0, 0],
        [3, 0, 1],
        [4, 0, 0],
        [3, 1, 0],
        [2, 1, 1],
        [4, 0, 0],
        [0, 4, 0],
        [0, 3, 1],
        [1, 3, 0],
        [0, 4, 0],
        [0, 2, 2],
        [1, 2, 1],
        [0, 4, 0],
        [0, 3, 1],
        [1, 3, 0],
        [0, 4, 0],
        [0, 0, 4],
        [0, 1, 3],
        [1, 0, 3],
        [0, 0, 4],
        [0, 2, 2],
        [1, 1, 2],
        [0, 0, 4],
        [0, 1, 3],
        [1, 0, 3],
        [0, 0, 4],
    ]
    .into_iter()
    .map(Vec::from)
    .collect();
    let rf = fleiss_kappa(&fixed, conf).unwrap();
    let se0 = null_se(&fixed);
    assert_close(rf.z_statistic, rf.kappa / se0, 1e-12, "Fleiss z uses SE_0");
    assert!(rf.se > se0);
    assert_close(rf.ci.0, (rf.kappa - z95 * rf.se).clamp(-1.0, 1.0), 1e-12, "Fleiss CI lo");
    assert_close(rf.ci.1, (rf.kappa + z95 * rf.se).clamp(-1.0, 1.0), 1e-12, "Fleiss CI hi");

    let pop_k = (0.66 - 0.3562) / (1.0 - 0.3562);
    let mut rng = DeterministicRng::from_seed(0xF1E1_55CA_99A0_2026);
    let (mut ks, mut se_sum, mut cov, mut cov0) = (Vec::with_capacity(3_000), 0.0, 0_usize, 0_usize);
    for _ in 0..3_000 {
        let mut tbl = Vec::with_capacity(60);
        for _ in 0..60 {
            let u = rng.next_f64();
            let tc = if u < 0.5 {
                0
            } else if u < 0.8 {
                1
            } else {
                2
            };
            let mut row = vec![0_u64; 3];
            for _ in 0..4 {
                let ur = rng.next_f64();
                row[if ur < 0.80 {
                    tc
                } else if ur < 0.90 {
                    (tc + 1) % 3
                } else {
                    (tc + 2) % 3
                }] += 1;
            }
            tbl.push(row);
        }
        let fk = fleiss_kappa(&tbl, conf).unwrap();
        let s0 = null_se(&tbl);
        ks.push(fk.kappa);
        se_sum += fk.se;
        cov += usize::from(fk.ci.0 <= pop_k && pop_k <= fk.ci.1);
        cov0 += usize::from(fk.kappa - z95 * s0 <= pop_k && pop_k <= fk.kappa + z95 * s0);
    }
    let mk = ks.iter().sum::<f64>() / 3_000.0;
    let emp_sd = (ks.iter().map(|&k| (k - mk).powi(2)).sum::<f64>() / 2_999.0).sqrt();
    assert!((se_sum / 3_000.0 - emp_sd).abs() < 0.1 * emp_sd);
    assert!((0.925..=0.975).contains(&(cov as f64 / 3_000.0)) && (cov0 as f64 / 3_000.0) < 0.88);
}

#[test]
fn audit_38_roc_single_subject_separation_and_paired_delong() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let c10: Vec<f64> = (1..=10).map(f64::from).collect();
    assert!(matches!(
        roc_curve_analysis(&c10, &[9.5], RocCiMethod::DeLong, conf),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));
    assert!(matches!(
        roc_curve_analysis(&[9.5], &c10, RocCiMethod::DeLong, conf),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));
    let roc_sh = col_sheet("roc1", &[("Ctrl", &c10), ("Case", &[9.5])]);
    assert!(matches!(
        run_kind(&roc_sh, AnalysisKind::RocCurve(RocSpec::default())),
        Err(StatsError::InsufficientSampleSize { required: 2, actual: 1, .. })
    ));

    let hm = roc_curve_analysis(&c10, &[9.5], RocCiMethod::HanleyMcNeil, conf).unwrap();
    assert_close(hm.auc, 0.9, 1e-12, "HM single-case AUC");
    assert_close(hm.p_value_vs_05, 4.0 / 11.0, 1e-12, "HM exact MW P");
    assert!(hm.se > 0.2 && hm.ci.0 < 0.5 && hm.ci.1 > 0.5);
    assert!(roc_curve_analysis(&[10.0], &c10, RocCiMethod::HanleyMcNeil, conf).unwrap().p_value_vs_05 >= 0.05);
    assert!(
        roc_curve_analysis(&(1..=100).map(f64::from).collect::<Vec<_>>(), &[99.5], RocCiMethod::HanleyMcNeil, conf).unwrap().p_value_vs_05 >= 0.05
    );
    let hm_out = run_kind(&roc_sh, AnalysisKind::RocCurve(RocSpec { ci_method: RocCiMethod::HanleyMcNeil })).unwrap();
    assert!(scalar(&hm_out, "P value (vs. 0.5)") >= 0.05);
    assert!(hm_out.checklist.iter().any(|c| c.assumption == "ROC small-sample inference" && c.status == ChecklistStatus::Warn));

    // Perfect, reverse, and all-tied separation (R1-S02-03).
    let (ctrl, case) = ([1.0, 2.0, 3.0, 4.0, 5.0], [10.0, 20.0, 30.0, 40.0, 50.0]);
    for method in [RocCiMethod::DeLong, RocCiMethod::HanleyMcNeil] {
        let sep = roc_curve_analysis(&ctrl, &case, method, conf).unwrap();
        assert_close(sep.auc, 1.0, 1e-12, "sep AUC");
        assert_close(sep.p_value_vs_05, 2.0 / 252.0, 1e-12, "sep MW P");
        assert!(sep.ci.0 > 0.3 && sep.ci.0 < 1.0 && sep.ci.1 <= 1.0);
        let rev = roc_curve_analysis(&case, &ctrl, method, conf).unwrap();
        assert_close(rev.auc, 0.0, 1e-12, "rev AUC");
        assert_close(rev.p_value_vs_05, 2.0 / 252.0, 1e-12, "rev MW P");
        assert!(rev.ci.0 >= 0.0 && rev.ci.1 > 0.0 && rev.ci.1 < 0.7);
    }
    let tied = roc_curve_analysis(&[5.0; 5], &[5.0; 5], RocCiMethod::DeLong, conf).unwrap();
    assert_close(tied.auc, 0.5, 1e-12, "tied AUC");
    assert_close(tied.p_value_vs_05, 1.0, 1e-12, "tied P");
    assert!(tied.ci.0 < 0.5 && tied.ci.1 > 0.5);

    // Paired DeLong v10/v01 validation (RV-35).
    let ra = roc_curve_analysis(&[1.0, 1.5, 2.0, 2.4, 3.0], &[2.2, 3.1, 3.8, 4.5, 5.2], RocCiMethod::DeLong, conf).unwrap();
    let rb = roc_curve_analysis(&[1.0, 1.5, 2.0, 2.4, 3.0], &[2.0, 2.9, 3.9, 4.1, 5.5], RocCiMethod::DeLong, conf).unwrap();
    assert!(compare_roc_delong_paired(&ra, &rb).is_ok());
    let mut bad = rb.clone();
    bad.delong_v10.pop();
    assert!(matches!(compare_roc_delong_paired(&ra, &bad), Err(StatsError::DimensionMismatch { expected: 5, actual: 4 })));
    let mut bad2 = ra.clone();
    bad2.delong_v01.pop();
    assert!(matches!(compare_roc_delong_paired(&bad2, &rb), Err(StatsError::DimensionMismatch { expected: 5, actual: 4 })));
    let mut bad3 = rb.clone();
    bad3.delong_v01.push(0.5);
    assert!(matches!(compare_roc_delong_paired(&ra, &bad3), Err(StatsError::DimensionMismatch { expected: 5, actual: 6 })));
    let mut bad4 = rb;
    bad4.delong_v10[2] = f64::NAN;
    assert!(matches!(compare_roc_delong_paired(&ra, &bad4), Err(StatsError::Math(MathError::DomainError { .. }))));
}

#[test]
fn audit_41_power_cohens_h_welch_allocation_bounds_and_mde() {
    // 1. Cohen's |h| <= pi & unreachable DetectableEffect (#41).
    for bad_h in [5.0, -5.0, PI + 1e-6, -PI - 1e-6] {
        let req = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
            .with_two_proportions_mode(TwoProportionsMethod::CohensH)
            .with_effect(PowerEffectSpec::Standardized(bad_h));
        assert!(matches!(solve_power(&req), Err(StatsError::InvalidParameter { name: "cohens_h", .. })));
        assert!(matches!(
            solve_power(&req.with_target(PowerSolveTarget::Power).with_sample_size(20.0)),
            Err(StatsError::InvalidParameter { name: "cohens_h", .. })
        ));
    }
    assert_close(
        solve_power(
            &PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
                .with_two_proportions_mode(TwoProportionsMethod::CohensH)
                .with_effect(PowerEffectSpec::Standardized(PI)),
        )
        .unwrap()
        .effect_size,
        PI,
        1e-12,
        "h=pi",
    );
    let unr = PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_two_proportions_mode(TwoProportionsMethod::CohensH)
        .with_sample_size(2.0)
        .with_alpha(0.05)
        .with_target_power(0.90);
    assert!(matches!(solve_power(&unr), Err(StatsError::InvalidParameter { name: "target_power", .. })));
    for dir in [OneSidedDirection::Greater, OneSidedDirection::Less] {
        assert!(matches!(
            solve_power(&unr.clone().with_tail(TailMode::OneSided(dir)).with_target_power(0.99)),
            Err(StatsError::InvalidParameter { name: "target_power", .. })
        ));
    }
    let ok_h = solve_power(&unr.with_sample_size(10.0)).unwrap();
    assert!(ok_h.effect_size.abs() <= PI && ok_h.achieved_power >= 0.90 - 1e-9);

    // 2. Welch unpaired t power R oracle & MDE round-trip (R2-S02-03).
    let welch = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(5.0)
            .with_sample_size_group2(25.0)
            .with_alpha(0.05)
            .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 10.0, sd: 10.0, sd_group2: Some(1.0), within_pair_correlation: None }),
    )
    .unwrap();
    assert_close(welch.actual_df.unwrap(), 4.016_013_322_658_279, 1e-10, "Welch df");
    assert_close(welch.noncentrality_param, 2.233_835_258_044_689, 1e-10, "Welch ncp");
    assert_close(welch.critical_value, 2.772_084_483_361_45, 1e-10, "Welch crit");
    assert_close(welch.achieved_power, 0.401_502_1, 1e-6, "Welch power");
    let wmde = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(8.0)
        .with_sample_size_group2(20.0)
        .with_alpha(0.05)
        .with_target_power(0.80)
        .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 0.0, sd: 8.0, sd_group2: Some(2.0), within_pair_correlation: None });
    let wr = solve_power(&wmde).unwrap();
    assert_close(
        solve_power(&wmde.with_target(PowerSolveTarget::Power).with_effect(PowerEffectSpec::MeanDiffAndSd {
            mean_diff: wr.raw_effect_size.unwrap(),
            sd: 8.0,
            sd_group2: Some(2.0),
            within_pair_correlation: None,
        }))
        .unwrap()
        .achieved_power,
        0.80,
        1e-9,
        "Welch MDE",
    );
}

#[test]
fn audit_fa_sec05_08_power_one_sided_mde_and_bounds() {
    // 3. One-sided MDE with target_power <= alpha (FA-SEC05-08) & SampleSize power <= alpha rejection (R2-S02-11).
    for tp in [0.02, 0.05] {
        for tail in [TailMode::OneSided(OneSidedDirection::Greater), TailMode::OneSided(OneSidedDirection::Less)] {
            for fam in [
                PowerTestFamily::OneSampleT,
                PowerTestFamily::TwoSamplePairedT,
                PowerTestFamily::TwoSampleUnpairedT,
                PowerTestFamily::SimpleLinearRegression,
            ] {
                let r = solve_power(
                    &PowerAnalysisRequest::new(fam)
                        .with_target(PowerSolveTarget::DetectableEffect)
                        .with_tail(tail)
                        .with_alpha(0.05)
                        .with_target_power(tp)
                        .with_sample_size(20.0),
                )
                .unwrap();
                assert_close(r.achieved_power, tp, 1e-7, "1-sided MDE tp<=alpha");
            }
            for cm in [
                CorrelationPowerMethod::FisherZ,
                CorrelationPowerMethod::FisherZWithTCrit,
                CorrelationPowerMethod::ExactTTransform,
                CorrelationPowerMethod::ConditionalSlopeT,
            ] {
                let r = solve_power(
                    &PowerAnalysisRequest::new(PowerTestFamily::Correlation)
                        .with_correlation_method(cm)
                        .with_target(PowerSolveTarget::DetectableEffect)
                        .with_tail(tail)
                        .with_alpha(0.05)
                        .with_target_power(tp)
                        .with_sample_size(25.0),
                )
                .unwrap();
                assert_close(r.achieved_power, tp, 1e-6, "Corr 1-sided MDE");
            }
            let rp1 = solve_power(
                &PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
                    .with_two_proportions_mode(TwoProportionsMethod::CohensH)
                    .with_target(PowerSolveTarget::DetectableEffect)
                    .with_tail(tail)
                    .with_alpha(0.05)
                    .with_target_power(tp)
                    .with_sample_size(50.0),
            )
            .unwrap();
            let rp2 = solve_power(
                &PowerAnalysisRequest::new(PowerTestFamily::TwoProportions)
                    .with_two_proportions_mode(TwoProportionsMethod::PooledNullUnpooledAlt)
                    .with_effect(PowerEffectSpec::BaselineProportion { p1: 0.5 })
                    .with_target(PowerSolveTarget::DetectableEffect)
                    .with_tail(tail)
                    .with_alpha(0.05)
                    .with_target_power(tp)
                    .with_sample_size(50.0),
            )
            .unwrap();
            assert_close(rp1.achieved_power, tp, 1e-7, "2P h MDE");
            assert_close(rp2.achieved_power, tp, 1e-7, "2P base MDE");
        }
    }
    for fam in [PowerTestFamily::OneSampleT, PowerTestFamily::Correlation] {
        assert!(matches!(
            solve_power(
                &PowerAnalysisRequest::new(fam)
                    .with_target(PowerSolveTarget::DetectableEffect)
                    .with_alpha(0.05)
                    .with_target_power(0.02)
                    .with_sample_size(20.0)
            ),
            Err(StatsError::InvalidParameter { name: "target_power", .. })
        ));
    }
    for (tail, sign) in [(TailMode::OneSided(OneSidedDirection::Greater), 1.0), (TailMode::OneSided(OneSidedDirection::Less), -1.0)] {
        for tp in [0.05, 0.10] {
            assert!(matches!(
                solve_power(&PowerAnalysisRequest::unpaired_t(2.0 * sign, 0.10, tp).with_tail(tail)),
                Err(StatsError::InvalidParameter { name: "target_power", .. })
            ));
        }
    }

    // 4. Allocation ratio, small k, non-monotone (51,3) R oracle, checked bounds, Regression Less MDE, & RV-74 fixed/ULP n2.
    assert!(solve_power(&PowerAnalysisRequest::two_proportions(0.6, 0.3, 0.05, 0.80).with_allocation_ratio(0.5)).unwrap().achieved_power >= 0.80);
    assert!(solve_power(&PowerAnalysisRequest::unpaired_t(0.8, 0.05, 0.80).with_allocation_ratio(0.25)).unwrap().achieved_power >= 0.80);
    let trap = solve_power(
        &PowerAnalysisRequest::two_proportions(0.001, 0.20, 0.05, 0.70)
            .with_two_proportions_mode(TwoProportionsMethod::PooledNullUnpooledAlt)
            .with_allocation_ratio(0.05),
    )
    .unwrap();
    assert_eq!((trap.n_per_group, trap.n_group2), (51, Some(3)));
    assert_close(trap.achieved_power, 0.701_269, 1e-5, "(51,3) R power");

    assert!(matches!(
        solve_power(
            &PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(1e8).with_allocation_ratio(1e12)
        ),
        Err(StatsError::InvalidParameter { name: "allocation_ratio", .. })
    ));
    assert!(matches!(
        solve_power(
            &PowerAnalysisRequest::one_way_anova(usize::MAX / 2, 0.3, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(10.0)
        ),
        Err(StatsError::InvalidParameter { name: "num_groups", .. })
    ));
    assert!(matches!(
        compute_power_curve(&PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_allocation_ratio(1e9), 10, 100, 10),
        Err(StatsError::InvalidParameter { .. })
    ));

    let reg_less = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
            .with_target(PowerSolveTarget::DetectableEffect)
            .with_tail(TailMode::OneSided(OneSidedDirection::Less))
            .with_sample_size(40.0)
            .with_alpha(0.05)
            .with_target_power(0.80),
    )
    .unwrap();
    assert!(reg_less.effect_size > 0.0 && !reg_less.power_curve.is_empty());
    let reg_sl = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::SimpleLinearRegression)
            .with_target(PowerSolveTarget::DetectableEffect)
            .with_tail(TailMode::OneSided(OneSidedDirection::Less))
            .with_sample_size(40.0)
            .with_alpha(0.05)
            .with_target_power(0.80)
            .with_effect(PowerEffectSpec::RegressionSlope { beta1: 0.0, sigma_x: 2.0, sigma_eps: 3.0 }),
    )
    .unwrap();
    assert!(reg_sl.raw_effect_size.unwrap() < 0.0);
    assert_eq!(
        solve_power(
            &PowerAnalysisRequest::new(PowerTestFamily::OneWayAnova)
                .with_target(PowerSolveTarget::DetectableEffect)
                .with_sample_size(20.0)
                .with_alpha(0.05)
                .with_target_power(0.80)
                .with_effect(PowerEffectSpec::GroupMeansAndSd { means: vec![0.0, 1.0, 2.0], within_sd: 1.0 })
        )
        .unwrap()
        .total_n,
        60
    );

    // Fixed n2=60/80 + explicit (11, 25) IEEE-754 1-ULP overshoot guard + k=2.5 curve propagation (RV-74 + R2-S02-04).
    let fixed60 = compute_power_curve(&PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(60.0), 10, 50, 5).unwrap();
    assert!(fixed60.iter().all(|pt| (pt.total_n - (pt.n + 60.0)).abs() < 1e-12));
    let solved80 = solve_power(&PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(80.0)).unwrap();
    assert_eq!((solved80.n_group2, solved80.total_n), (Some(80), solved80.n_per_group + 80));

    let pair_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(11.0)
        .with_sample_size_group2(25.0)
        .with_allocation_ratio(1.0)
        .with_alpha(0.05)
        .with_effect(PowerEffectSpec::Standardized(0.7));
    let pr = solve_power(&pair_req).unwrap();
    assert_eq!((pr.n_per_group, pr.n_group2, pr.total_n), (11, Some(25), 36));
    let p11 = pr.power_curve.iter().find(|pt| (pt.n - 11.0).abs() < 0.5).unwrap();
    assert_close(p11.total_n, 36.0, 1e-12, "ULP curve total_n=36");
    assert_close(compute_power_curve(&pair_req, 11, 22, 12).unwrap()[0].total_n, 36.0, 1e-12, "direct curve total_n=36");
    let mde11 = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(11.0)
        .with_sample_size_group2(25.0)
        .with_allocation_ratio(1.0)
        .with_alpha(0.05)
        .with_target_power(0.80);
    assert!(solve_power(&mde11).unwrap().sensitivity_table.iter().all(|c| (c.sample_size, c.total_n) == (11, 36)));
    assert!(compute_sensitivity_table(&mde11, &[0.05, 0.01], &[0.80, 0.90]).unwrap().iter().all(|c| (c.sample_size, c.total_n) == (11, 36)));
    let p25 = PowerAnalysisRequest::unpaired_t(0.6, 0.05, 0.80)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(20.0)
        .with_sample_size_group2(50.0)
        .with_allocation_ratio(1.0);
    assert_close(compute_power_curve(&p25, 10, 30, 5).unwrap()[0].total_n, 35.0, 1e-12, "k=2.5 curve");
}

#[test]
fn audit_aud05_categorical_kappa_mcnemar_ca_barnard_fisher_and_nnt() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // AUD-05-04 (Fleiss, Nee & Landis 1979): Exact null variance SE_0 for K=2 (0.85, 0.15) & K=3 (0.7, 0.2, 0.1).
    let tbl2: Vec<Vec<u64>> = (0..100)
        .map(|i| {
            if i < 80 {
                vec![10, 0]
            } else if i < 90 {
                vec![5, 5]
            } else {
                vec![0, 10]
            }
        })
        .collect();
    let r2 = fleiss_kappa(&tbl2, conf).unwrap();
    assert_close(r2.kappa / r2.z_statistic, (2.0_f64 / (100.0 * 10.0 * 9.0)).sqrt(), 1e-12, "Fleiss 1979 K=2 SE_0");
    let tbl3: Vec<Vec<u64>> = (0..100)
        .map(|i| {
            if i < 65 {
                vec![10, 0, 0]
            } else if i < 75 {
                vec![5, 5, 0]
            } else if i < 90 {
                vec![0, 10, 0]
            } else {
                vec![0, 0, 10]
            }
        })
        .collect();
    let r3 = fleiss_kappa(&tbl3, conf).unwrap();
    let (pe, pj3) = (0.7_f64.powi(2) + 0.2_f64.powi(2) + 0.1_f64.powi(2), 0.7_f64.powi(3) + 0.2_f64.powi(3) + 0.1_f64.powi(3));
    assert_close(
        r3.kappa / r3.z_statistic,
        (2.0 * (pe + pe * pe - 2.0 * pj3) / (100.0 * 10.0 * 9.0 * (1.0 - pe).powi(2))).sqrt(),
        1e-12,
        "Fleiss 1979 K=3 SE_0",
    );

    // AUD-05-05 (McNemar 1947; Cochran-Armitage): Zero-margin 2x2 McNemar, asymmetric 3x3 guard, & interior empty dose gap.
    let res_zm = run_kind(&contingency_sheet("mc-zm", &[&[0, 10], &[0, 5]]), ct_kind(ContingencyMethod::McNemarPaired)).unwrap();
    assert_close(scalar(&res_zm, "McNemar exact P"), 2.0 * 0.5_f64.powi(10), 1e-12, "McNemar zero-margin P");
    assert_close(scalar(&res_zm, "McNemar chi-square"), 8.1, 1e-12, "McNemar zero-margin Yates chi2");

    let asym_3x3 = vec![vec![10_u64, 2, 0], vec![0, 0, 0], vec![3, 15, 0]];
    assert!(matches!(
        run_kind(&contingency_sheet("mc-3x3", &[&[10, 2, 0], &[0, 0, 0], &[3, 15, 0]]), ct_kind(ContingencyMethod::McNemarPaired)),
        Err(StatsError::InvalidParameter { name: "mcnemar_dimensions", .. })
    ));
    let ac_3x3 = analyze_contingency(&asym_3x3, false, conf).unwrap();
    assert_eq!((ac_3x3.mcnemar_chi2, ac_3x3.mcnemar_exact_p), (None, None));

    let with_gap = vec![vec![20_u64, 0], vec![0, 0], vec![5, 15], vec![5, 15]];
    let exp_gap = -5.175_491_695_067_655;
    assert_close(cochran_armitage_trend(&with_gap).unwrap().0, exp_gap, 1e-12, "CA gap [1,3,4]");
    assert_close(cochran_armitage_trend(&[vec![20, 0], vec![5, 15], vec![5, 15]]).unwrap().0, -4.743_416_490_252_569, 1e-12, "CA contig");
    assert_close(analyze_contingency(&with_gap, false, conf).unwrap().cochran_armitage_z.unwrap(), exp_gap, 1e-12, "analyze_contingency CA gap");
    for (id, rows) in
        [("ca42", &[&[20_u64, 0][..], &[0, 0], &[5, 15], &[5, 15]][..]), ("ca43", &[&[20_u64, 0, 0][..], &[0, 0, 0], &[5, 15, 0], &[5, 15, 0]][..])]
    {
        let rs = run_kind(&contingency_sheet(id, rows), ct_kind(ContingencyMethod::CochranArmitageTrend)).unwrap();
        let tab = tab_by_id(&rs, "contingency-main");
        assert_eq!(tab.rows[0][1], ResultCell::Empty);
        assert_close(cell_num(&tab.rows[0][0]), exp_gap, 1e-12, id);
    }

    // AUD-05-06 (Barnard 1947 CSM), AUD-05-16 (Fisher RxC cell cap), AUD-05-51/52/60 (symmetry, "∞" ratios, signed NNT).
    let csm = barnard_exact_2x2_with_ordering(2, 0, 19, 481, BarnardOrdering::Csm).unwrap();
    assert!(!csm.used_asymptotic_fallback);
    assert_close(csm.p_value, 0.000_854_99, 1e-5, "Barnard CSM boundary oracle");

    let mut diag100 = vec![vec![0_u64; 100]; 100];
    for (i, row) in diag100.iter_mut().enumerate() {
        row[i] = 1;
    }
    assert!(matches!(fisher_exact_rxc(&diag100), Err(StatsError::InvalidParameter { name: "fisher_exact_max_cells", .. })));

    for (id, rows) in [
        ("ct25", &[&[5_u64, 25, 25, 25, 25][..], &[4, 25, 25, 25, 25]][..]),
        ("ct52", &[&[5_u64, 4][..], &[25, 25], &[25, 25], &[25, 25], &[25, 25]][..]),
    ] {
        let r = run_kind(&contingency_sheet(id, rows), AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
        assert_eq!(r.checklist.iter().find(|c| c.assumption == "Minimum expected cell count").unwrap().status, ChecklistStatus::Pass);
    }
    let r_inf = run_kind(
        &contingency_sheet("ct-inf", &[&[10, 0], &[0, 5]]),
        AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }),
    )
    .unwrap();
    let rt = tab_by_id(&r_inf, "contingency-ratios");
    assert_eq!((&rt.rows[0][0], &rt.rows[1][0]), (&ResultCell::Text("∞".into()), &ResultCell::Text("∞".into())));
    let r_harm = analyze_2x2_ratios(2, 8, 7, 3, conf).unwrap();
    assert!(r_harm.nnt.unwrap() < 0.0);
    assert_close(r_harm.nnt.unwrap().abs(), r_harm.nnt_value.unwrap(), 1e-12, "signed NNT magnitude");
}

#[test]
fn audit_aud05_power_min_n_asinh_partial_sensitivity_and_chisquare_cap() {
    // AUD-05-11: Contextual min integer n for Correlation (n=3) & homoscedastic UnpairedT (n1=1, n2=10).
    let cp = solve_power(
        &PowerAnalysisRequest::correlation(0.99, 0.05, 0.80)
            .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(3.0),
    )
    .unwrap();
    assert_eq!((cp.n_per_group, cp.total_n, cp.actual_df, cp.power_curve[0].n), (3, 3, Some(1.0), 3.0));
    assert_close(cp.achieved_power, cp.power_at_continuous_n, 1e-12, "Corr n=3 power");
    assert!(cp.achieved_power < 0.70);
    let cn =
        solve_power(&PowerAnalysisRequest::correlation(0.999, 0.05, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform)).unwrap();
    assert!((cn.n_per_group, cn.total_n, cn.actual_df) == (3, 3, Some(1.0)) && cn.achieved_power >= 0.80);
    let cmde = solve_power(
        &PowerAnalysisRequest::new(PowerTestFamily::Correlation)
            .with_correlation_method(CorrelationPowerMethod::ConditionalSlopeT)
            .with_target(PowerSolveTarget::DetectableEffect)
            .with_sample_size(3.0)
            .with_alpha(0.05)
            .with_target_power(0.80),
    )
    .unwrap();
    assert_eq!((cmde.n_per_group, cmde.actual_df), (3, Some(1.0)));
    assert_close(cmde.achieved_power, 0.80, 1e-9, "Corr n=3 MDE");
    assert!(
        solve_power(
            &PowerAnalysisRequest::correlation(0.99, 0.05, 0.80)
                .with_correlation_method(CorrelationPowerMethod::FisherZ)
                .with_target(PowerSolveTarget::Power)
                .with_sample_size(3.0)
        )
        .is_err()
    );

    let tp = solve_power(
        &PowerAnalysisRequest::unpaired_t(1.0, 0.05, 0.80).with_target(PowerSolveTarget::Power).with_sample_size(1.0).with_sample_size_group2(10.0),
    )
    .unwrap();
    assert_eq!((tp.n_per_group, tp.n_group2, tp.total_n, tp.actual_df, tp.power_curve[0].n), (1, Some(10), 11, Some(9.0), 1.0));
    assert_close(tp.achieved_power, tp.power_at_continuous_n, 1e-12, "UnpairedT (1,10)");
    let tn = solve_power(&PowerAnalysisRequest::unpaired_t(6.0, 0.05, 0.80).with_sample_size_group2(10.0)).unwrap();
    assert!((tn.n_per_group, tn.n_group2, tn.total_n, tn.actual_df) == (1, Some(10), 11, Some(9.0)) && tn.achieved_power >= 0.80);
    assert!(
        solve_power(
            &PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
                .with_target(PowerSolveTarget::Power)
                .with_sample_size(1.0)
                .with_sample_size_group2(10.0)
                .with_effect(PowerEffectSpec::MeanDiffAndSd { mean_diff: 1.0, sd: 1.0, sd_group2: Some(2.0), within_pair_correlation: None })
        )
        .is_err()
    );
    assert_eq!((PowerTestFamily::Correlation.min_integer_n(), PowerTestFamily::TwoSampleUnpairedT.min_integer_n()), (4, 2));

    // AUD-05-35: Correlation power asinh(t / sqrt(df)) avoids atanh(1.0) saturation at tiny alpha/df.
    let r_sa = solve_power(
        &PowerAnalysisRequest::correlation(0.999_999_999, 1e-9, 0.80)
            .with_correlation_method(CorrelationPowerMethod::ExactTTransform)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(3.0),
    )
    .unwrap();
    assert!(r_sa.achieved_power.is_finite() && r_sa.achieved_power > 0.0);
    let r_df =
        solve_power(&PowerAnalysisRequest::correlation(0.99, 0.001, 0.80).with_correlation_method(CorrelationPowerMethod::ExactTTransform)).unwrap();
    assert!(r_df.n_per_group >= 3 && (0.80..=1.0).contains(&r_df.achieved_power));
    let r_fz = solve_power(
        &PowerAnalysisRequest::correlation(0.99999, 1e-12, 0.80)
            .with_correlation_method(CorrelationPowerMethod::FisherZWithTCrit)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(3.005),
    )
    .unwrap();
    assert!(
        r_fz.power_at_continuous_n.is_finite() && r_fz.power_at_continuous_n > 0.0 && r_fz.achieved_power.is_finite() && r_fz.achieved_power > 0.0
    );

    // AUD-05-49: Sensitivity table keeps achievable cells & notes omitted ones; AUD-05-50: ChiSquare MAX_CONTINUOUS_N cap.
    let req49 = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(50.0);
    let res49 = solve_power(&req49).unwrap();
    assert!(res49.n_per_group == 88 && res49.achieved_power >= 0.80);
    assert!(
        solve_power(
            &PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80)
                .with_target(PowerSolveTarget::Power)
                .with_sample_size(87.0)
                .with_sample_size_group2(50.0)
        )
        .unwrap()
        .achieved_power
            < 0.80
    );
    assert!(!res49.sensitivity_table.is_empty() && res49.sensitivity_table.len() < 16);
    assert!(res49.sensitivity_table.iter().any(|c| (c.alpha - 0.05).abs() < 1e-12 && (c.target_power - 0.80).abs() < 1e-12));
    assert!(res49.notes.iter().any(|n| n.contains("Sensitivity table omitted")));
    let sh_pow = col_sheet("pow-src", &[("A", &[1.0, 2.0])]);
    let rs_pow = run_kind(
        &sh_pow,
        AnalysisKind::PowerAnalysis(PowerAnalysisSpec {
            family: curvebook_doc::PowerTestFamily::TwoSampleUnpairedT,
            solve_target: curvebook_doc::PowerSolveTarget::SampleSize,
            alpha: 0.05,
            target_power: Some(0.80),
            sample_size_group2: Some(50.0),
            effect: curvebook_doc::PowerEffectSpec::Standardized(0.5),
            ..Default::default()
        }),
    )
    .unwrap();
    assert!(rs_pow.tabs.iter().any(|t| t.id == "power-sensitivity"));
    assert!(compute_sensitivity_table(&req49, &[1.5], &[0.80]).is_err());
    assert!(compute_sensitivity_table(&req49, &[0.001], &[0.95]).is_err());

    for tiny_w in [1e-8_f64, 1e-200_f64] {
        assert!(matches!(
            solve_power(&PowerAnalysisRequest::chi_square(tiny_w, 1.0, 0.05, 0.80)),
            Err(StatsError::NotConverged { algorithm: "solve_continuous_n" })
        ));
    }
    assert_eq!(solve_power(&PowerAnalysisRequest::chi_square(0.30, 1.0, 0.05, 0.80)).unwrap().total_n, 88);
}
