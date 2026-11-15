#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use lumencraft_doc::{AnalysisKind, AnalysisSpec, Cell, ContingencyMethod, ContingencySpec, ContingencyTable, DataSheet, SheetId, TableArchetype};
use lumencraft_math::ConfidenceLevel;
use lumencraft_stats::categorical::{
    KappaWeighting, analyze_2x2_ratios, analyze_contingency, baptista_pike_or_ci, clopper_pearson_ci, cochran_armitage_trend, cohens_kappa,
    exact_binomial_test, fisher_exact_2x2, fisher_exact_rxc, fleiss_kappa, mcnemar_detailed, mcnemar_test,
};
use lumencraft_stats::{StatsError, run_analysis};

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

fn make_contingency_sheet(rows: &[&[u64]]) -> DataSheet {
    let n_rows = rows.len();
    let n_cols = rows.first().map_or(0, |r| r.len());
    let mut table = ContingencyTable::new(n_cols, n_rows).expect("valid dimensions");
    for (r, row) in rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(val as f64);
        }
    }
    DataSheet::new(SheetId::new("audit-sheet"), "Audit", TableArchetype::Contingency(table)).expect("sheet")
}

fn run_contingency_spec(rows: &[&[u64]], method: ContingencyMethod) -> Result<lumencraft_doc::ResultSheet, StatsError> {
    let sheet = make_contingency_sheet(rows);
    let spec =
        AnalysisSpec::new(SheetId::new("audit-res"), sheet.id.clone(), AnalysisKind::Contingency(ContingencySpec { method, ..Default::default() }))
            .expect("spec");
    run_analysis(&sheet, &spec)
}

#[test]
fn r1_s02_01_huge_counts_hang_2x2() {
    // Reproducer from R1-S02-01: huge 2x2 counts must return Err cleanly without CPU hang.
    let res = run_with_timeout(Duration::from_secs(3), || fisher_exact_2x2(1_000_000_000, 1_000_000_000, 1_000_000_000, 1_000_000_000));
    assert!(matches!(res, Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));

    let res_1e12 =
        run_with_timeout(Duration::from_secs(3), || fisher_exact_2x2(1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000));
    assert!(matches!(res_1e12, Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));
}

#[test]
fn r1_s02_01_huge_counts_panic_rxc() {
    // Reproducer from R1-S02-01: row sum wrapping u64 / capacity overflow must never panic or OOM.
    let counts = vec![vec![1_u64 << 63, 1_u64 << 63], vec![0, 5], vec![0, 0]];
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze_contingency(&counts, false, ConfidenceLevel::DEFAULT_95)));
    assert!(res.is_ok(), "analyze_contingency must not panic on overflowing table");
    assert!(res.unwrap().is_err());

    // Ensure non-wrapping column sums exceeding pool cap also fail cleanly in fisher_exact_rxc.
    let huge_finite = vec![vec![1_u64 << 62, 1_u64 << 62], vec![0, 5], vec![0, 0]];
    let rxc = fisher_exact_rxc(&huge_finite);
    assert!(matches!(rxc, Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));
}

#[test]
fn fisher_exact_2x2_guards_and_accuracy() {
    // u64 overflows on individual margins and table total.
    assert!(matches!(fisher_exact_2x2(u64::MAX, 1, 0, 0), Err(StatsError::InvalidParameter { name: "fisher_exact_row1_total", .. })));
    assert!(matches!(fisher_exact_2x2(0, 0, u64::MAX, 1), Err(StatsError::InvalidParameter { name: "fisher_exact_row2_total", .. })));
    assert!(matches!(fisher_exact_2x2(1_u64 << 63, 0, 1_u64 << 63, 0), Err(StatsError::InvalidParameter { name: "fisher_exact_col1_total", .. })));
    assert!(matches!(fisher_exact_2x2(1_u64 << 63, 0, 0, 1_u64 << 63), Err(StatsError::InvalidParameter { name: "fisher_exact_table_total", .. })));

    // Imbalanced huge table exceeding total cap returns Err fast without losing precision or hanging.
    let imbalanced = run_with_timeout(Duration::from_secs(2), || fisher_exact_2x2(1, 1_000_000_000_000, 1, 1_000_000_000_000));
    assert!(matches!(imbalanced, Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));

    // Valid exact tables retain full accuracy.
    assert!((fisher_exact_2x2(0, 0, 0, 0).unwrap() - 1.0).abs() < 1e-12);
    let p = fisher_exact_2x2(14, 6, 5, 15).unwrap();
    // Known two-sided hypergeometric probability for [[14, 6], [5, 15]] is 0.010386702198860637.
    assert!((p - 0.010_386_702_198_860_637).abs() < 1e-12, "unexpected Fisher 2x2 p = {p}");
}

#[test]
fn fisher_exact_rxc_guards_and_accuracy() {
    // Overflowing table must fail rather than panic or OOM.
    let overflow = vec![vec![1_u64 << 63, 1_u64 << 63], vec![0, 5], vec![0, 0]];
    assert!(fisher_exact_rxc(&overflow).is_err());

    // Above deterministic Monte Carlo cap (50_000) returns Err without slow allocation or fallback.
    let over_cap = vec![vec![10_000, 10_000, 10_000], vec![10_000, 5_000, 5_001]];
    assert!(matches!(fisher_exact_rxc(&over_cap), Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));

    // Dispatching 2x2 through fisher_exact_rxc still enforces the 2x2 cap.
    let over_2x2 = vec![vec![2_000_000, 2_000_000], vec![1_000_000, 1_000_000]];
    assert!(matches!(fisher_exact_rxc(&over_2x2), Err(StatsError::InvalidParameter { name: "fisher_exact_max_total", .. })));

    // Ragged 2x2 and ragged RxC tables return DimensionMismatch rather than indexing panics.
    assert!(matches!(
        fisher_exact_rxc(&[vec![1, 2], vec![3]]),
        Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: 2, actual: 1 }))
    ));

    // Valid 3x3 table keeps deterministic result.
    let valid_3x3 = vec![vec![10, 2, 1], vec![2, 10, 2], vec![1, 2, 10]];
    let p_3x3 = fisher_exact_rxc(&valid_3x3).unwrap();
    assert!((0.0..=0.01).contains(&p_3x3), "unexpected 3x3 Fisher p = {p_3x3}");
}

#[test]
fn mcnemar_guards_and_accuracy() {
    assert!(matches!(mcnemar_test(u64::MAX, 1), Err(StatsError::InvalidParameter { name: "mcnemar_discordant_total", .. })));
    assert!(matches!(
        mcnemar_detailed(u64::MAX, 1, ConfidenceLevel::DEFAULT_95),
        Err(StatsError::InvalidParameter { name: "mcnemar_discordant_total", .. })
    ));

    let over_cap = run_with_timeout(Duration::from_secs(2), || mcnemar_test(1_000_000_000, 1_000_000_000));
    assert!(matches!(over_cap, Err(StatsError::InvalidParameter { name: "mcnemar_max_total", .. })));

    let over_cap_det = run_with_timeout(Duration::from_secs(2), || mcnemar_detailed(3_000_000, 3_000_000, ConfidenceLevel::DEFAULT_95));
    assert!(matches!(over_cap_det, Err(StatsError::InvalidParameter { name: "mcnemar_max_total", .. })));

    // Symmetric case within cap finishes in O(1) without summing 2M binomial terms.
    let (chi2_sym, p_sym) = mcnemar_test(2_000_000, 2_000_000).unwrap();
    assert_eq!(chi2_sym, 0.0);
    assert_eq!(p_sym, 1.0);

    // Known paired discordant test (18, 5): chi2 = 144/23, exact p = 89104 / 2^23.
    let (chi2, p) = mcnemar_test(18, 5).unwrap();
    assert!((chi2 - 144.0 / 23.0).abs() < 1e-12);
    assert!((p - 89_104.0 / 8_388_608.0).abs() < 1e-12, "unexpected McNemar p = {p}");
}

#[test]
fn analyze_contingency_and_run_analysis_huge_counts() {
    // Overflow checks on row, column, total, and ragged tables.
    assert!(analyze_contingency(&[vec![u64::MAX, 1], vec![0, 5]], false, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(analyze_contingency(&[vec![1_u64 << 63, 0], vec![1_u64 << 63, 0]], false, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(analyze_contingency(&[vec![1_u64 << 63, 0], vec![0, 1_u64 << 63]], false, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(analyze_contingency(&[vec![1, 2], vec![3]], false, ConfidenceLevel::DEFAULT_95).is_err());

    // Large finite 2x2 table works quickly for chi-square/G^2 while safely skipping exact Fisher & McNemar.
    let huge_finite = vec![vec![1_000_000_000_000, 1_000_000_000_000], vec![1_000_000_000_000, 1_000_000_000_000]];
    let res = run_with_timeout(Duration::from_secs(2), move || analyze_contingency(&huge_finite, false, ConfidenceLevel::DEFAULT_95).unwrap());
    assert_eq!(res.total_count, 4_000_000_000_000);
    assert_eq!(res.chi_square, 0.0);
    assert_eq!(res.chi_square_p, 1.0);
    assert_eq!(res.fisher_exact_p, None);
    assert_eq!(res.mcnemar_exact_p, None);

    // run_analysis dispatches cleanly across methods.
    assert!(
        run_contingency_spec(&[&[1_u64 << 63, 1_u64 << 63], &[0, 5], &[0, 0]], ContingencyMethod::PearsonChiSquare { yates_continuity: false })
            .is_err()
    );
    assert!(run_contingency_spec(&[&[10_000_000, 10_000_000], &[10_000_000, 10_000_000]], ContingencyMethod::FisherExact).is_err());
    assert!(run_contingency_spec(&[&[0, 10_000_000], &[10_000_000, 0]], ContingencyMethod::McNemarPaired).is_err());
    assert!(run_contingency_spec(&[&[1_u64 << 63, 1_u64 << 63]], ContingencyMethod::ExactBinomial).is_err());
    assert!(run_contingency_spec(&[&[3_000_000, 3_000_000]], ContingencyMethod::ExactBinomial).is_err());
    assert!(run_contingency_spec(&[&[1_u64 << 63, 1_u64 << 63], &[0, 5]], ContingencyMethod::BarnardExact).is_err());

    let chi_ok = run_contingency_spec(
        &[&[1_000_000_000_000, 1_000_000_000_000], &[1_000_000_000_000, 1_000_000_000_000]],
        ContingencyMethod::PearsonChiSquare { yates_continuity: false },
    );
    assert!(chi_ok.is_ok());
}

#[test]
fn proportion_kappa_and_ratio_overflow_guards() {
    // Baptista-Pike OR CI skips cleanly (returns None) on overflow or huge tables without OOM.
    assert_eq!(baptista_pike_or_ci(u64::MAX, 1, 1, 1, ConfidenceLevel::DEFAULT_95), None);
    let bp_huge = run_with_timeout(Duration::from_secs(2), || {
        baptista_pike_or_ci(1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, 1_000_000_000_000, ConfidenceLevel::DEFAULT_95)
    });
    assert_eq!(bp_huge, None);
    let (bp_lo, bp_hi) = baptista_pike_or_ci(14, 6, 5, 15, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(bp_lo > 1.0 && bp_hi > bp_lo && bp_hi.is_finite());

    // analyze_2x2_ratios catches u64 overflow while computing finite large tables quickly.
    assert!(analyze_2x2_ratios(u64::MAX, 1, 1, 1, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(analyze_2x2_ratios(1_u64 << 63, 0, 0, 1_u64 << 63, ConfidenceLevel::DEFAULT_95).is_err());
    let ratios_huge = run_with_timeout(Duration::from_secs(2), || {
        analyze_2x2_ratios(1_000_000_000, 1_000_000_000, 1_000_000_000, 1_000_000_000, ConfidenceLevel::DEFAULT_95).unwrap()
    });
    assert_eq!(ratios_huge.or_baptista_pike_ci, None);
    assert!((ratios_huge.odds_ratio - 1.0).abs() < 1e-12);

    // Clopper-Pearson k > n and Exact Binomial total cap.
    assert!(clopper_pearson_ci(10, 5, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(matches!(
        exact_binomial_test(1, 5_000_001, 0.5, ConfidenceLevel::DEFAULT_95),
        Err(StatsError::InvalidParameter { name: "binomial_max_total", .. })
    ));

    // Cohen's kappa and Fleiss' kappa overflow / zero / ragged guards.
    assert!(cohens_kappa(&[vec![1_u64 << 63, 1_u64 << 63], vec![0, 0]], KappaWeighting::Unweighted, ConfidenceLevel::DEFAULT_95).is_err());
    assert!(matches!(cohens_kappa(&[vec![0, 0], vec![0, 0]], KappaWeighting::Unweighted, ConfidenceLevel::DEFAULT_95), Err(StatsError::EmptyInput)));
    assert!(fleiss_kappa(&[vec![1_u64 << 63, 1_u64 << 63], vec![1_u64 << 63, 1_u64 << 63]], ConfidenceLevel::DEFAULT_95).is_err());
    assert!(fleiss_kappa(&[vec![0, 0, 2], vec![1_u64 << 63, 1_u64 << 63, 2]], ConfidenceLevel::DEFAULT_95).is_err());

    // Cochran-Armitage rejects ragged matrices without indexing panics.
    assert!(matches!(
        cochran_armitage_trend(&[vec![1, 2], vec![3], vec![4, 5]]),
        Err(StatsError::Math(lumencraft_math::MathError::DimensionMismatch { expected: 2, actual: 1 }))
    ));
}
