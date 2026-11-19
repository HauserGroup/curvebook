//! Wave 2 (`L2` Stats) unit, analytical, and oracle reference parity tests for:
//! - Barnard's exact unconditional 2x2 test (`ContingencyMethod::BarnardExact`)
//! - Passing–Bablok nonparametric method-comparison regression (`PassingBablokSpec`)
//! - Multiple Imputation (`MultivariateSpec::MultipleImputation`: MICE Type-1 PMM + Rubin / Barnard–Rubin pooling)
//!
//! Reference provenance (clean-room Decision D1):
//! - Suissa, S., & Shuster, J. J. (1985), *Journal of the Royal Statistical Society: Series A*, 148(4), 317–327
//!   (exact 2x2 combinatorial sums over S = s verified in Python 3.13 stdlib `math` 2026-11-13).
//! - Boschloo (1970); Barnard (1945/1947) CSM convexity-symmetry-maximum ordering.
//! - Passing, H., & Bablok, W. (1983), *Journal of Clinical Chemistry and Clinical Biochemistry*, 21(11), 709–720
//!   (rank CIs and CUSUM Kolmogorov supremum test; exact slope/CI/CUSUM benchmark verified in Python 3.13 stdlib 2026-11-13).
//! - Rubin, D. B. (1987), *Multiple Imputation for Nonresponse in Surveys*; Barnard, J., & Rubin, D. B. (1999), *Biometrika*, 86(4), 948–955;
//!   van Buuren, S. & Groothuis-Oudshoorn, K. (2011), *Journal of Statistical Software*, 45(3), 1–67
//!   (Rubin/Barnard-Rubin *t*/df/CI oracle verified in R 4.5.2 `pt`/`qt` 2026-11-13).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, ContingencyTable, DataSheet,
    MultipleVariablesTable, MultivariateSpec, PassingBablokSpec, ResultCell, ResultTabKind, SheetId, SubcolumnMode, TableArchetype, VariableRole,
    XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::categorical::{BarnardOrdering, barnard_exact_2x2, barnard_exact_2x2_with_ordering, fisher_exact_2x2};
use curvebook_stats::multivariate::{MultipleImputationConfig, mice_predictive_mean_matching, rubin_pool_estimates};
use curvebook_stats::regression::{execute_passing_bablok, passing_bablok_regression};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_approx_eq;

fn spec_for(id: &str, src: &str, kind: AnalysisKind) -> AnalysisSpec {
    AnalysisSpec::new(SheetId::new(id), SheetId::new(src), kind).unwrap()
}

fn scalar_by_key(sheet: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key).map(|s| s.value).unwrap_or_else(|| panic!("missing scalar {key}"))
}

fn tab_by_kind(sheet: &curvebook_doc::ResultSheet, kind: ResultTabKind) -> &curvebook_doc::ResultTab {
    sheet.tabs.iter().find(|t| t.kind == kind).unwrap_or_else(|| panic!("missing tab {kind:?}"))
}

fn contingency_sheet(id: &str, vals: [[f64; 2]; 2]) -> DataSheet {
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    ct.data_sets[0].title = String::from("Outcome+");
    ct.data_sets[1].title = String::from("Outcome-");
    ct.row_titles = vec![String::from("Treatment"), String::from("Control")];
    for (r, row) in vals.iter().enumerate() {
        for (c, &v) in row.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), "2x2 Table", TableArchetype::Contingency(ct)).unwrap()
}

// ============================================================================
// 1. Barnard's Exact Unconditional 2x2 Binomial Test
// ============================================================================

#[test]
fn barnard_exact_score_matches_exact_combinatorics() {
    // Table [[3, 1], [1, 3]]: exact two-sided Suissa-Shuster score P = 74/256 = 0.2890625 at pi = 0.5.
    let r1 = barnard_exact_2x2(3, 1, 1, 3).unwrap();
    assert!(!r1.used_asymptotic_fallback);
    assert_eq!(r1.ordering, BarnardOrdering::ScorePooled);
    assert_approx_eq(r1.p_value, 74.0 / 256.0, 1e-11, 1e-11, "Barnard 3/1/1/3 P");
    assert_approx_eq(r1.nuisance_pi, 0.5, 1e-3, 1e-3, "Barnard 3/1/1/3 nuisance pi");

    // Table [[1, 7], [6, 2]]: balanced 8+8 margins, exact P = 0.021270751953125 at pi = 0.5.
    let r2 = barnard_exact_2x2(1, 7, 6, 2).unwrap();
    assert!(!r2.used_asymptotic_fallback);
    assert_approx_eq(r2.p_value, 0.021270751953125, 1e-11, 1e-11, "Barnard 1/7/6/2 P");

    // Moderate balanced clinical trial [[14, 6], [5, 15]] -> P = 0.006439537086407654.
    let r3 = barnard_exact_2x2(14, 6, 5, 15).unwrap();
    assert!(!r3.used_asymptotic_fallback);
    assert_approx_eq(r3.p_value, 0.006439537086407654, 1e-9, 1e-9, "Barnard 14/6/5/15 P");
}

#[test]
fn barnard_exact_maximizes_over_skewed_nuisance_parameter_pi() {
    // Asymmetric sparse table [[1, 19], [6, 14]] maximizes away from pi = 0.5 (exact continuum peak at pi = 0.159325643541092).
    let r1 = barnard_exact_2x2(1, 19, 6, 14).unwrap();
    assert!(!r1.used_asymptotic_fallback);
    assert_approx_eq(r1.p_value, 0.04875822085272651, 1e-12, 1e-12, "Barnard 1/19/6/14 skewed pi maximization");
    assert_approx_eq(r1.nuisance_pi, 0.159325643541092, 1e-5, 1e-5, "Barnard 1/19/6/14 skewed pi location");

    // Asymmetric [[3, 0], [0, 12]] maximizes near marginal success proportion pi = 0.2.
    let r2 = barnard_exact_2x2(3, 0, 0, 12).unwrap();
    assert!(!r2.used_asymptotic_fallback);
    assert_approx_eq(r2.p_value, 0.0005497579110400005, 1e-8, 1e-8, "Barnard 3/0/0/12 exact P");
    assert!(r2.nuisance_pi < 0.35, "expected skewed nuisance pi < 0.35, got {}", r2.nuisance_pi);
}

#[test]
fn barnard_exact_orderings_symmetry_and_degenerate_tables() {
    // Compare all 4 orderings on [[1, 7], [6, 2]] against Fisher's conditional test (0.0405594...).
    let fisher_p = fisher_exact_2x2(1, 7, 6, 2).unwrap();
    let score = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::ScorePooled).unwrap();
    let wald = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::WaldUnpooled).unwrap();
    let boschloo = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::Boschloo).unwrap();
    let csm = barnard_exact_2x2_with_ordering(1, 7, 6, 2, BarnardOrdering::Csm).unwrap();

    assert!(score.p_value > 0.0 && score.p_value < 0.05);
    assert!(wald.p_value > 0.0 && wald.p_value < 0.05);
    // Boschloo's unconditional Fisher p-ordering dominates Fisher conditional exact p!
    assert!(boschloo.p_value <= fisher_p + 1e-12, "Boschloo {} should not exceed Fisher {}", boschloo.p_value, fisher_p);
    assert!(csm.p_value > 0.0 && csm.p_value < 0.05);

    // Row-swap and outcome-complement invariance.
    let swapped = barnard_exact_2x2(6, 2, 1, 7).unwrap();
    let flipped = barnard_exact_2x2(7, 1, 2, 6).unwrap();
    assert_approx_eq(swapped.p_value, score.p_value, 1e-11, 1e-11, "row-swap invariance");
    assert_approx_eq(flipped.p_value, score.p_value, 1e-11, 1e-11, "column-complement invariance");

    // Degenerate zero-column [[5, 0], [5, 0]] and empty-group [[0, 0], [5, 5]] yield P = 1.0 cleanly.
    let zero_col = barnard_exact_2x2(5, 0, 5, 0).unwrap();
    assert_approx_eq(zero_col.p_value, 1.0, 1e-12, 1e-12, "identical marginal proportions P = 1");
    let empty_row = barnard_exact_2x2(0, 0, 5, 5).unwrap();
    assert_approx_eq(empty_row.p_value, 1.0, 1e-12, 1e-12, "empty group margin P = 1");
}

#[test]
fn barnard_exact_result_sheet_and_table_caps() {
    let sheet = contingency_sheet("ct-barnard", [[14.0, 6.0], [5.0, 15.0]]);
    let spec = spec_for(
        "r-barnard",
        "ct-barnard",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::BarnardExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            expected_proportions: None,
            null_proportion: None,
        }),
    );
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_approx_eq(scalar_by_key(&res, "Barnard's exact P"), 0.006439537086407654, 1e-9, 1e-9, "scalar Barnard P");
    let pi_val = scalar_by_key(&res, "Barnard nuisance pi");
    assert!((0.0..=1.0).contains(&pi_val));
    assert!(scalar_by_key(&res, "Barnard Z-pooled").abs() > 2.5);
    assert_eq!(tab_by_kind(&res, ResultTabKind::MainSummary).row_labels[0], "Barnard's exact test");
    assert!(res.checklist.iter().any(|c| c.assumption.contains("Barnard")));

    // Non-2x2 table (2 rows x 3 columns) must reject BarnardExact without panicking.
    let mut ct_2x3 = ContingencyTable::new(3, 2).unwrap();
    for c in 0..3 {
        ct_2x3.data_sets[c].cells[0][0] = Cell::number(5.0 + c as f64);
        ct_2x3.data_sets[c].cells[1][0] = Cell::number(8.0 - c as f64);
    }
    let sheet_2x3 = DataSheet::new(SheetId::new("ct-2x3"), "2x3", TableArchetype::Contingency(ct_2x3)).unwrap();
    let spec_2x3 = spec_for(
        "r-b2x3",
        "ct-2x3",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::BarnardExact,
            compute_odds_and_risk_ratios: false,
            compute_sensitivity_specificity: false,
            compute_phi_and_cramers_v: false,
            expected_proportions: None,
            null_proportion: None,
        }),
    );
    assert!(matches!(run_analysis(&sheet_2x3, &spec_2x3), Err(StatsError::InvalidParameter { .. })));

    // Huge table hits cap / asymptotic fallback instantaneously instead of hanging.
    let huge = barnard_exact_2x2(5_000, 5_000, 4_800, 5_200).unwrap();
    assert!(huge.used_asymptotic_fallback);
    assert!(huge.p_value.is_finite() && huge.p_value > 0.0 && huge.p_value < 0.05);

    // Overflowing table counts return clear StatsError::InvalidParameter.
    assert!(matches!(barnard_exact_2x2(u64::MAX, 1, 1, 1), Err(StatsError::InvalidParameter { .. })));

    // Oversized asymmetric CSM grid is explicitly guarded.
    assert!(matches!(barnard_exact_2x2_with_ordering(44, 46, 46, 44, BarnardOrdering::Csm), Err(StatsError::InvalidParameter { .. })));
}

// ============================================================================
// 2. Passing–Bablok Nonparametric Regression
// ============================================================================

#[test]
fn passing_bablok_oracle_benchmark_and_collinear_baseline() {
    // 16-point method comparison benchmark verified against exact Passing-Bablok (1983) estimator.
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0];
    let ys = [3.4, 5.1, 6.8, 9.3, 10.6, 13.1, 14.9, 17.4, 18.8, 21.3, 24.7, 29.2, 32.8, 37.5, 41.0, 45.2];
    let pb = passing_bablok_regression(&xs, &ys, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(pb.n, 16);
    assert_eq!(pb.valid_slopes, 120);
    assert_eq!(pb.k_offset, 0);
    assert_approx_eq(pb.slope, 2.0, 1e-12, 1e-12, "PB16 slope");
    assert_approx_eq(pb.slope_ci.0, 1.966666666666667, 1e-12, 1e-12, "PB16 slope CI lower");
    assert_approx_eq(pb.slope_ci.1, 2.025, 1e-12, 1e-12, "PB16 slope CI upper");
    assert_approx_eq(pb.intercept, 1.1, 1e-12, 1e-12, "PB16 intercept");
    assert_approx_eq(pb.intercept_ci.0, 0.787499999999997, 1e-11, 1e-11, "PB16 intercept CI lower");
    assert_approx_eq(pb.intercept_ci.1, 1.383333333333331, 1e-11, 1e-11, "PB16 intercept CI upper");
    assert_approx_eq(pb.cusum_h, 1.0, 1e-12, 1e-12, "PB16 CUSUM H");
    assert_approx_eq(pb.cusum_statistic, 1.0 / 8.0_f64.sqrt(), 1e-12, 1e-12, "PB16 CUSUM statistic H / sqrt(L + 1)");
    assert_approx_eq(pb.cusum_p_value, 0.999_633_292_157_727_7, 1e-9, 1e-9, "PB16 CUSUM P");
    assert!(pb.linearity_passed);

    // Exact noiseless line y = 2.5*x - 3.0 -> zero CUSUM H and P = 1.0.
    let x_exact = [1.0, 2.0, 3.0, 5.0, 8.0, 11.0];
    let y_exact: Vec<f64> = x_exact.iter().map(|&x| 2.5 * x - 3.0).collect();
    let pb_exact = passing_bablok_regression(&x_exact, &y_exact, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pb_exact.slope, 2.5, 1e-12, 1e-12, "exact line slope");
    assert_approx_eq(pb_exact.intercept, -3.0, 1e-12, 1e-12, "exact line intercept");
    assert_approx_eq(pb_exact.cusum_h, 0.0, 1e-12, 1e-12, "exact line CUSUM H");
    assert_approx_eq(pb_exact.cusum_p_value, 1.0, 1e-12, 1e-12, "exact line CUSUM P");
}

#[test]
fn passing_bablok_k_offset_symmetry_permutation_and_nonlinear_cusum() {
    // Dataset with:
    // - identical points (2, 8) and (2, 8) -> undefined S_ij dropped
    // - same-X distinct-Y (4, 5) and (4, 11) -> undefined dx = 0 dropped
    // - slope == -1 pair (5, 7) and (8, 4) -> excluded cleanly
    // - negative slopes < -1 triggering exact K offset!
    let xs = [1.0, 2.0, 2.0, 4.0, 4.0, 5.0, 6.0, 8.0];
    let ys = [14.0, 8.0, 8.0, 5.0, 11.0, 7.0, 1.0, 4.0];
    let pb = passing_bablok_regression(&xs, &ys, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(pb.k_offset > 0, "expected K > 0 for steep negative slopes, got {}", pb.k_offset);
    assert!(pb.valid_slopes < 8 * 7 / 2, "expected dropped undefined and -1 slopes, got {}", pb.valid_slopes);
    assert!(pb.slope.is_finite() && pb.intercept.is_finite());

    // Inverse equivariance X <-> Y on strictly increasing pairs without -1 slopes.
    let ax = [1.2, 2.5, 3.8, 5.1, 6.9, 8.4, 10.2, 12.7, 15.0, 18.3];
    let ay = [2.9, 4.8, 7.1, 8.9, 11.7, 13.8, 16.5, 20.1, 23.4, 28.6];
    let pb_xy = passing_bablok_regression(&ax, &ay, ConfidenceLevel::DEFAULT_95).unwrap();
    let pb_yx = passing_bablok_regression(&ay, &ax, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pb_yx.slope, 1.0 / pb_xy.slope, 1e-11, 1e-11, "Passing-Bablok X<->Y inverse slope");
    assert_approx_eq(pb_yx.intercept, -pb_xy.intercept / pb_xy.slope, 1e-10, 1e-10, "Passing-Bablok X<->Y inverse intercept");

    // Row permutation invariance.
    let perm = [7, 2, 9, 0, 5, 1, 8, 3, 6, 4];
    let px: Vec<f64> = perm.iter().map(|&i| ax[i]).collect();
    let py: Vec<f64> = perm.iter().map(|&i| ay[i]).collect();
    let pb_perm = passing_bablok_regression(&px, &py, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pb_perm.slope, pb_xy.slope, 1e-12, 1e-12, "row permutation slope invariance");
    assert_approx_eq(pb_perm.intercept, pb_xy.intercept, 1e-12, 1e-12, "row permutation intercept invariance");
    assert_approx_eq(pb_perm.cusum_p_value, pb_xy.cusum_p_value, 1e-12, 1e-12, "row permutation CUSUM invariance");

    // Nonlinear quadratic curve (x = 1..=36): CUSUM H = 9, L = 18, H / sqrt(19) ~ 2.0647, P < 0.001.
    let qx: Vec<f64> = (1..=36).map(f64::from).collect();
    let qy: Vec<f64> = qx.iter().map(|&x| 0.25 * x * x + 0.5 * x + 1.0).collect();
    let pb_quad = passing_bablok_regression(&qx, &qy, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(pb_quad.cusum_h, 9.0, 1e-9, 1e-9, "quadratic CUSUM H");
    assert_approx_eq(pb_quad.cusum_statistic, 9.0 / 19.0_f64.sqrt(), 1e-9, 1e-9, "quadratic CUSUM statistic");
    assert!(pb_quad.cusum_p_value < 0.001, "expected nonlinear rejection P < 0.001, got {}", pb_quad.cusum_p_value);
    assert!(!pb_quad.linearity_passed);

    // Input error guards.
    assert!(passing_bablok_regression(&[1.0, 2.0], &[3.0, 4.0], ConfidenceLevel::DEFAULT_95).is_err());
    assert!(passing_bablok_regression(&[1.0, 2.0, 3.0], &[1.0, f64::NAN, 3.0], ConfidenceLevel::DEFAULT_95).is_err());
}

#[test]
fn passing_bablok_execute_and_run_analysis_across_all_archetypes() {
    let xs = [2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0];
    let ys = [5.1, 8.9, 13.2, 16.8, 21.0, 25.1, 28.9, 33.0];

    // 1. ColumnTable
    let mut col_t = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, xs.len()).unwrap();
    col_t.data_sets[0].title = String::from("RefMethod");
    col_t.data_sets[1].title = String::from("NewMethod");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        col_t.data_sets[0].cells[r][0] = Cell::number(x);
        col_t.data_sets[1].cells[r][0] = Cell::number(y);
    }
    let col_sheet = DataSheet::new(SheetId::new("pb-col"), "Col PB", TableArchetype::Column(col_t)).unwrap();
    let pb_spec_col = PassingBablokSpec { x_column: 0, y_column: 1 };
    let res_exec = execute_passing_bablok(&col_sheet, &pb_spec_col, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(scalar_by_key(&res_exec, "Slope"), 2.0, 0.05, 0.05, "Column execute slope");
    assert_approx_eq(scalar_by_key(&res_exec, "Intercept"), 1.0, 0.4, 0.4, "Column execute intercept");
    assert_eq!(tab_by_kind(&res_exec, ResultTabKind::ParametersAndConfidenceIntervals).row_labels.len(), 2);
    assert_eq!(tab_by_kind(&res_exec, ResultTabKind::Residuals).rows.len(), xs.len());
    assert!(!res_exec.fitted_curves.is_empty());

    // 2. XyTable choosing col 2 (second Y dataset) against X (col 0)
    let mut xy = XyTable::new(SubcolumnMode::default(), 2, xs.len()).unwrap();
    xy.data_sets[0].title = String::from("Unused");
    xy.data_sets[1].title = String::from("TestY");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(999.0);
        xy.data_sets[1].cells[r][0] = Cell::number(y);
    }
    let xy_sheet = DataSheet::new(SheetId::new("pb-xy"), "XY PB", TableArchetype::Xy(xy)).unwrap();
    let res_xy =
        run_analysis(&xy_sheet, &spec_for("r-pb-xy", "pb-xy", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 2 }))).unwrap();
    assert_approx_eq(scalar_by_key(&res_xy, "Slope"), 2.0, 0.05, 0.05, "XY col 2 slope");

    // 3. MultipleVariablesTable with Label col 0 + excluded row
    let mut mv = MultipleVariablesTable::new(3, xs.len() + 1).unwrap();
    mv.variables[0].title = String::from("SampleId");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("Ref");
    mv.variables[2].title = String::from("Assay");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        mv.variables[0].cells[r] = Cell::text(format!("S{r}"));
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number(y);
    }
    mv.variables[0].cells[xs.len()] = Cell::text("OutlierExcluded");
    mv.variables[1].cells[xs.len()] = Cell::number(100.0);
    mv.variables[2].cells[xs.len()] = Cell::excluded_number(-500.0);
    let mv_sheet = DataSheet::new(SheetId::new("pb-mv"), "MV PB", TableArchetype::MultipleVariables(mv)).unwrap();
    let res_mv =
        run_analysis(&mv_sheet, &spec_for("r-pb-mv", "pb-mv", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 1, y_column: 2 }))).unwrap();
    assert_approx_eq(scalar_by_key(&res_mv, "N"), xs.len() as f64, 1e-12, 1e-12, "excluded row skipped");
    assert_approx_eq(scalar_by_key(&res_mv, "Slope"), 2.0, 0.05, 0.05, "MV col 1 vs 2 slope");
}

// ============================================================================
// 3. Multiple Imputation (MICE PMM & Rubin / Barnard–Rubin Pooling)
// ============================================================================

#[test]
fn rubin_pooling_oracle_with_barnard_rubin_df() {
    // Verified against exact Rubin (1987) / Barnard-Rubin (1999) equations + R 4.5.2 pt/qt.
    let qs = [1.80, 2.05, 1.95, 2.20, 2.00];
    let us = [0.04, 0.05, 0.045, 0.06, 0.055];
    let pooled = rubin_pool_estimates("Slope", &qs, &us, Some(30.0), ConfidenceLevel::DEFAULT_95).unwrap();

    assert_eq!(pooled.m, 5);
    assert_approx_eq(pooled.estimate, 2.0, 1e-12, 1e-12, "Rubin pooled Q_bar");
    assert_approx_eq(pooled.within_variance, 0.05, 1e-12, 1e-12, "Rubin within variance U");
    assert_approx_eq(pooled.between_variance, 0.02125, 1e-12, 1e-12, "Rubin between variance B");
    assert_approx_eq(pooled.total_variance, 0.0755, 1e-12, 1e-12, "Rubin total variance T");
    assert_approx_eq(pooled.pooled_se, 0.274772633280682, 1e-12, 1e-12, "Rubin pooled SE");
    assert_approx_eq(pooled.riv, 0.51, 1e-12, 1e-12, "Rubin RIV");
    assert_approx_eq(pooled.lambda, 0.337748344370861, 1e-12, 1e-12, "Rubin lambda");
    assert_approx_eq(pooled.df, 12.180_396_862_592_83, 1e-10, 1e-10, "Barnard-Rubin adjusted df");
    assert_approx_eq(pooled.fmi, 0.424999245862250, 1e-11, 1e-11, "Barnard-Rubin FMI");
    assert_approx_eq(pooled.t_statistic, 7.278745252468389, 1e-11, 1e-11, "Rubin t statistic");
    assert_approx_eq(pooled.p_value, 8.968027011903839e-6, 1e-10, 1e-7, "R 4.5.2 pt P value");
    assert_approx_eq(pooled.ci.0, 1.402303781090206, 1e-10, 1e-10, "R 4.5.2 qt CI lower");
    assert_approx_eq(pooled.ci.1, 2.597696218909794, 1e-10, 1e-10, "R 4.5.2 qt CI upper");

    // Identical across imputations -> B = 0, RIV = 0, FMI = 0, df = complete_data_df.
    let zero_b = rubin_pool_estimates("ZeroMiss", &[3.0, 3.0, 3.0], &[0.25, 0.25, 0.25], Some(18.0), ConfidenceLevel::DEFAULT_95).unwrap();
    assert_approx_eq(zero_b.between_variance, 0.0, 1e-12, 1e-12, "zero B");
    assert_approx_eq(zero_b.riv, 0.0, 1e-12, 1e-12, "zero RIV");
    assert_approx_eq(zero_b.fmi, 0.0, 1e-12, 1e-12, "zero FMI");
    assert_approx_eq(zero_b.df, 18.0, 1e-12, 1e-12, "complete-data df preserved when B = 0");
}

#[test]
fn mice_pmm_deterministic_reproducibility_recovery_and_zero_missingness() {
    let labels = vec![String::from("X1"), String::from("X2"), String::from("Y")];
    let n = 24;
    let mut x1 = Vec::with_capacity(n);
    let mut x2 = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for i in 0..n {
        let f1 = 1.0 + (i as f64) * 0.5;
        let f2 = ((i * 7) % 11) as f64 * 0.3 - 1.2;
        let eps = (((i * 5) % 9) as f64 - 4.0) * 0.05;
        x1.push(Some(f1));
        x2.push(Some(f2));
        y.push(Some(2.0 + 1.5 * f1 - 0.8 * f2 + eps));
    }

    // Complete case (zero missingness). Out-of-bounds m / max_iters return typed errors instead of clamping.
    let cfg_invalid_m = MultipleImputationConfig {
        imputations_m: 1,
        max_iters: 5,
        seed: 101,
        response_col: Some(2),
        predictor_cols: vec![0, 1],
        confidence: ConfidenceLevel::DEFAULT_95,
    };
    assert!(matches!(
        mice_predictive_mean_matching(&labels, &[x1.clone(), x2.clone(), y.clone()], &cfg_invalid_m),
        Err(StatsError::InvalidParameter { name: "imputations_m", .. })
    ));
    let cfg_zero = MultipleImputationConfig { imputations_m: 2, ..cfg_invalid_m };
    let res_zero = mice_predictive_mean_matching(&labels, &[x1.clone(), x2.clone(), y.clone()], &cfg_zero).unwrap();
    assert_eq!(res_zero.m, 2);
    assert_eq!(res_zero.total_missing, 0);
    for pm in &res_zero.pooled_means {
        assert_approx_eq(pm.between_variance, 0.0, 1e-12, 1e-12, "zero missingness B = 0");
        assert_approx_eq(pm.fmi, 0.0, 1e-12, 1e-12, "zero missingness FMI = 0");
    }
    let reg_zero = res_zero.pooled_regression.as_ref().unwrap();
    assert_approx_eq(reg_zero[1].estimate, 1.5, 0.05, 0.05, "complete-case X1 slope");

    // Introduce interior missing cells across both X2 and Y.
    let mut x2_mis = x2.clone();
    let mut y_mis = y.clone();
    for idx in [3, 9, 15, 20] {
        y_mis[idx] = None;
    }
    for idx in [6, 12, 18] {
        x2_mis[idx] = None;
    }

    let cfg_a = MultipleImputationConfig {
        imputations_m: 12,
        max_iters: 10,
        seed: 2026,
        response_col: Some(2),
        predictor_cols: vec![0, 1],
        confidence: ConfidenceLevel::DEFAULT_95,
    };
    let res_a = mice_predictive_mean_matching(&labels, &[x1.clone(), x2_mis.clone(), y_mis.clone()], &cfg_a).unwrap();
    let res_a_repeat = mice_predictive_mean_matching(&labels, &[x1.clone(), x2_mis.clone(), y_mis.clone()], &cfg_a).unwrap();
    assert_eq!(res_a.imputed_datasets, res_a_repeat.imputed_datasets, "seed reproducibility failed");
    assert_eq!(res_a.pooled_means, res_a_repeat.pooled_means);
    assert_eq!(res_a.total_missing, 7);

    // Out-of-bounds upper m rejects instead of clamping, while different valid seeds produce distinct chains.
    let cfg_over_m = MultipleImputationConfig { seed: 9090, imputations_m: 80, ..cfg_a.clone() };
    assert!(matches!(
        mice_predictive_mean_matching(&labels, &[x1.clone(), x2_mis.clone(), y_mis.clone()], &cfg_over_m),
        Err(StatsError::InvalidParameter { name: "imputations_m", .. })
    ));
    let cfg_b = MultipleImputationConfig { seed: 9090, imputations_m: 50, ..cfg_a };
    let res_b = mice_predictive_mean_matching(&labels, &[x1.clone(), x2_mis.clone(), y_mis.clone()], &cfg_b).unwrap();
    assert_eq!(res_b.m, 50);
    assert_ne!(res_a.imputed_datasets[0], res_b.imputed_datasets[0]);

    // Observed cells must remain bit-exact; missing cells must draw plausible observed donors.
    for ds in &res_a.imputed_datasets {
        for r in 0..n {
            if let Some(v) = x1[r] {
                assert_approx_eq(ds[r][0], v, 0.0, 0.0, "observed X1 unchanged");
            }
            if let Some(v) = x2_mis[r] {
                assert_approx_eq(ds[r][1], v, 0.0, 0.0, "observed X2 unchanged");
            } else {
                assert!(ds[r][1].is_finite() && (-1.5..=2.5).contains(&ds[r][1]));
            }
            if let Some(v) = y_mis[r] {
                assert_approx_eq(ds[r][2], v, 0.0, 0.0, "observed Y unchanged");
            } else {
                assert!(ds[r][2].is_finite());
            }
        }
    }

    let reg_a = res_a.pooled_regression.as_ref().unwrap();
    assert_approx_eq(reg_a[0].estimate, 2.0, 0.5, 0.5, "MICE recovered intercept ~ 2");
    assert_approx_eq(reg_a[1].estimate, 1.5, 0.1, 0.1, "MICE recovered X1 slope ~ 1.5");
    assert_approx_eq(reg_a[2].estimate, -0.8, 0.25, 0.25, "MICE recovered X2 slope ~ -0.8");
    assert!(reg_a[1].between_variance > 0.0 && reg_a[1].fmi > 0.0);
    assert!(reg_a[2].between_variance > 0.0 && reg_a[2].fmi > 0.0);
}

#[test]
fn multiple_imputation_run_analysis_on_mv_with_label_column_and_errors() {
    let mut mv = MultipleVariablesTable::new(4, 16).unwrap();
    mv.variables[0].title = String::from("PatientId");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[1].title = String::from("BiomarkerA");
    mv.variables[2].title = String::from("BiomarkerB");
    mv.variables[3].title = String::from("Outcome");

    for r in 0..16 {
        let a = 10.0 + (r as f64) * 1.5;
        let b = ((r * 3) % 7) as f64 - 3.0;
        let out_val = 5.0 + 0.8 * a + 1.2 * b;
        mv.variables[0].cells[r] = Cell::text(format!("ID-{r:02}"));
        mv.variables[1].cells[r] = Cell::number(a);
        mv.variables[2].cells[r] = if r == 4 || r == 11 { Cell::empty() } else { Cell::number(b) };
        mv.variables[3].cells[r] = match r {
            5 => Cell::excluded_number(9999.0),
            9 => Cell::empty(),
            _ => Cell::number(out_val),
        };
    }

    let sheet = DataSheet::new(SheetId::new("mi-mv"), "MI Clinical", TableArchetype::MultipleVariables(mv)).unwrap();

    // Means-only MICE run (ignores Label column automatically).
    let res_means = run_analysis(
        &sheet,
        &spec_for(
            "r-mi-means",
            "mi-mv",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 6,
                max_iters: 8,
                seed: 77,
                response_column: None,
                predictor_columns: Vec::new(),
            }),
        ),
    )
    .unwrap();
    assert_approx_eq(scalar_by_key(&res_means, "Imputations (m)"), 6.0, 1e-12, 1e-12, "m = 6 scalar");
    assert_approx_eq(scalar_by_key(&res_means, "Total missing cells"), 4.0, 1e-12, 1e-12, "4 missing cells (2 empty B + 2 missing/excluded Outcome)");
    assert_eq!(tab_by_kind(&res_means, ResultTabKind::MainSummary).row_labels, vec!["BiomarkerA", "BiomarkerB", "Outcome"]);
    assert!(matches!(tab_by_kind(&res_means, ResultTabKind::MainSummary).rows[2][0], ResultCell::Number { .. }));

    // Pooled regression specifying raw col 3 vs predictors [1, 2] (with Label at raw col 0!).
    let res_reg = run_analysis(
        &sheet,
        &spec_for(
            "r-mi-reg",
            "mi-mv",
            AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                imputations_m: 8,
                max_iters: 10,
                seed: 123,
                response_column: Some(3),
                predictor_columns: vec![1, 2],
            }),
        ),
    )
    .unwrap();
    let reg_tab = tab_by_kind(&res_reg, ResultTabKind::ParametersAndConfidenceIntervals);
    assert_eq!(reg_tab.row_labels, vec!["Intercept", "BiomarkerA", "BiomarkerB"]);
    assert_approx_eq(scalar_by_key(&res_reg, "Reg BiomarkerA: Pooled estimate"), 0.8, 0.15, 0.15, "MV pooled BiomarkerA slope");
    assert_approx_eq(scalar_by_key(&res_reg, "Reg BiomarkerB: Pooled estimate"), 1.2, 0.25, 0.25, "MV pooled BiomarkerB slope");
    assert!(!tab_by_kind(&res_reg, ResultTabKind::Diagnostics).rows.is_empty());
    assert!(res_reg.checklist.iter().any(|c| c.assumption.contains("FMI")));

    // Out-of-bounds response column returns MissingColumn cleanly.
    let bad_spec = spec_for(
        "r-mi-bad",
        "mi-mv",
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 5,
            seed: 1,
            response_column: Some(99),
            predictor_columns: vec![1],
        }),
    );
    assert!(matches!(run_analysis(&sheet, &bad_spec), Err(StatsError::MissingColumn { index: 99 })));

    // Column with < 2 observed numeric cells fails cleanly with InsufficientSampleSize.
    let sparse_cols = vec![vec![Some(1.0), Some(2.0), Some(3.0)], vec![Some(5.0), None, None]];
    let cfg = MultipleImputationConfig::default();
    assert!(matches!(
        mice_predictive_mean_matching(&[String::from("A"), String::from("B")], &sparse_cols, &cfg),
        Err(StatsError::InsufficientSampleSize { .. })
    ));
}
