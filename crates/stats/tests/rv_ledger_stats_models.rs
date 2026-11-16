#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Regression and mathematical verification suite for the 2026-11-16 milestone review ledger
//! (`stats-models` slice: `RV-26`, `RV-27`, `RV-31`, `RV-32`, `RV-33`, `RV-34`, `RV-35`,
//! `RV-67`, `RV-74`).
//!
//! Literature references:
//! - Albert, A., & Anderson, J. A. (1984). On the existence of maximum likelihood estimates in
//!   logistic regression models. *Biometrika*, 71(1), 1-10.
//! - Cohen, J. (1988). *Statistical Power Analysis for the Behavioral Sciences* (2nd ed.). Erlbaum.
//! - DeLong, E. R., DeLong, D. M., & Clarke-Pearson, D. L. (1988). Comparing the areas under two
//!   or more correlated receiver operating characteristic curves: a nonparametric approach.
//!   *Biometrics*, 44(3), 837-845.
//! - Dunnett, C. W. (1955). A multiple comparison procedure for comparing several treatments with
//!   a control. *Journal of the American Statistical Association*, 50(272), 1096-1121.
//! - Glorfeld, L. W. (1995). An improvement on Horn's parallel analysis methodology for selecting
//!   the correct number of factors to retain. *Educational and Psychological Measurement*, 55(3),
//!   377-393.
//! - Grambsch, P. M., & Therneau, T. M. (1994). Proportional hazards tests and diagnostics based
//!   on weighted residuals. *Biometrika*, 81(3), 515-526.
//! - Horn, J. L. (1965). A rationale and test for the number of factors in factor analysis.
//!   *Psychometrika*, 30(2), 179-185.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). Springer.
//! - Kaplan, E. L., & Meier, P. (1958). Nonparametric estimation from incomplete observations.
//!   *Journal of the American Statistical Association*, 53(282), 457-481.
//! - McCullagh, P., & Nelder, J. A. (1989). *Generalized Linear Models* (2nd ed.). Chapman & Hall.
//! - Passing, H., & Bablok, W. (1983). A new biometrical procedure for testing the equality of
//!   measurements from two different analytical methods. *Journal of Clinical Chemistry and
//!   Clinical Biochemistry*, 21(11), 709-720.
//! - Šidák, Z. (1967). Rectangular confidence regions for the means of multivariate normal
//!   distributions. *Journal of the American Statistical Association*, 62(318), 626-633.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, CoxTieMethod, DataSheet, GlmFamily, GlmSpec, MultipleVariablesTable, MultiplicityMethod,
    MultiplicitySpec, ResultCell, SheetId, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, SurvivalTable, TableArchetype,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::SummaryMoments;
use curvebook_math::{ConfidenceLevel, MathError};
use curvebook_stats::error::StatsError;
use curvebook_stats::multiplicity::{adjust_holm_sidak, adjust_sidak};
use curvebook_stats::multivariate::pca_analysis;
use curvebook_stats::power::{
    PowerAnalysisRequest, PowerEffectSpec, PowerSolveTarget, PowerTestFamily, compute_power_curve, compute_sensitivity_table, solve_power,
};
use curvebook_stats::regression::{multiple_logistic_regression, multiple_poisson_regression, passing_bablok_regression, run_glm_analysis};
use curvebook_stats::roc::{RocCiMethod, compare_roc_delong_paired, roc_curve_analysis};
use curvebook_stats::run_analysis;
use curvebook_stats::survival::{CoxSubject, compare_survival_curves, cox_ph_regression, kaplan_meier_curve};

fn assert_close(actual: f64, expected: f64, tol: f64, ctx: &str) {
    assert!((actual - expected).abs() <= tol, "{ctx}: expected {expected} +/- {tol}, got {actual}");
}

// ============================================================================
// RV-26: Singular Fisher Information in Logistic & Poisson GLMs
// ============================================================================

#[test]
fn rv_26_logistic_and_poisson_singular_fisher_information_returns_singular_matrix() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let x1 = [0.2, 0.5, 0.9, 1.3, 1.8, 2.2, 2.7, 3.1, 3.6, 4.0];

    // 1. Logistic regression with collinear predictor x2 = 2 * x1 + 1 (rank 2 < 3).
    let flat_collinear: Vec<f64> = x1.iter().flat_map(|&v| [1.0, v, 2.0 * v + 1.0]).collect();
    let x_collinear = DenseMatrix::from_row_major(10, 3, flat_collinear).unwrap();
    let y_bin = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let names = vec![String::from("Intercept"), String::from("x1"), String::from("x2_dup")];

    let err_log_col = multiple_logistic_regression(&x_collinear, &y_bin, &names, true, conf).unwrap_err();
    assert!(
        matches!(err_log_col, StatsError::Math(MathError::SingularMatrix { rank: 2, expected_rank: 3 })),
        "expected SingularMatrix(2 < 3) for collinear logistic design, got {err_log_col:?}"
    );

    // 2. Logistic regression with extreme predictor scale saturating fitted probabilities (w = p*(1-p) -> 0).
    let x_sat_vals = [-1.0e4, -1.0e4, -1.0e4, -1.0e4, 1.0e4, 1.0e4, 1.0e4, 1.0e4];
    let y_sat = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let x_sat = DenseMatrix::from_row_major(8, 2, x_sat_vals.iter().flat_map(|&v| [1.0, v]).collect()).unwrap();
    let err_log_sat = multiple_logistic_regression(&x_sat, &y_sat, &[String::from("Intercept"), String::from("x")], false, conf).unwrap_err();
    assert!(
        matches!(err_log_sat, StatsError::Math(MathError::SingularMatrix { .. })),
        "expected SingularMatrix when logistic Fisher weights collapse, got {err_log_sat:?}"
    );

    // 3. Poisson regression with collinear predictor x2 = 2 * x1 + 1 (rank 2 < 3).
    let y_cnt = [2.0, 3.0, 5.0, 4.0, 8.0, 11.0, 14.0, 19.0, 23.0, 30.0];
    let err_pois_col = multiple_poisson_regression(&x_collinear, &y_cnt, &names, true, conf).unwrap_err();
    assert!(
        matches!(err_pois_col, StatsError::Math(MathError::SingularMatrix { rank: 2, expected_rank: 3 })),
        "expected SingularMatrix(2 < 3) for collinear Poisson design, got {err_pois_col:?}"
    );

    // 4. Poisson regression with complete zero-count separation on a binary group indicator:
    // group == 1 has exclusively zero counts, driving beta_group -> -inf and Fisher weight mu -> 0.
    let grp_sep = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let y_sep_cnt = [6.0, 8.0, 5.0, 7.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let x_pois_sep = DenseMatrix::from_row_major(10, 2, grp_sep.iter().flat_map(|&g| [1.0, g]).collect()).unwrap();
    let err_pois_sep =
        multiple_poisson_regression(&x_pois_sep, &y_sep_cnt, &[String::from("Intercept"), String::from("grp")], false, conf).unwrap_err();
    assert!(
        matches!(err_pois_sep, StatsError::Math(MathError::SingularMatrix { .. })),
        "expected SingularMatrix for separated zero-count Poisson indicator, got {err_pois_sep:?}"
    );

    // 5. Sheet-level GLM dispatch propagates SingularMatrix cleanly for both families.
    let mut mv = MultipleVariablesTable::new(3, 10).unwrap();
    for i in 0..10 {
        mv.variables[0].cells[i] = Cell::number(y_bin[i]);
        mv.variables[1].cells[i] = Cell::number(x1[i]);
        mv.variables[2].cells[i] = Cell::number(x1[i]); // exact duplicate column
    }
    let sid = SheetId::new("rv26-glm");
    let ds = DataSheet::new(sid.clone(), "RV26 GLM", TableArchetype::MultipleVariables(mv)).unwrap();
    let glm_log =
        GlmSpec { family: GlmFamily::Logistic, response_column: 0, predictor_columns: vec![1, 2], reference_levels: Vec::new(), compute_vif: true };
    let aspec_log = AnalysisSpec::new("rv26-an-log", sid, AnalysisKind::GeneralizedLinearModel(glm_log.clone())).unwrap();
    assert!(matches!(run_glm_analysis(&ds, &aspec_log, &glm_log), Err(StatsError::Math(MathError::SingularMatrix { .. }))));
}

// ============================================================================
// RV-27: Horn (1965) Parallel Analysis on Unstandardized Covariance Scale
// ============================================================================

#[test]
fn rv_27_horn_parallel_analysis_covariance_scale_invariance() {
    // 30 observations x 6 variables driven by 2 latent factors + small independent noise.
    let mut rng = DeterministicRng::from_seed(1965);
    let n = 30_usize;
    let p = 6_usize;
    let mut base_rows = Vec::with_capacity(n * p);
    for _ in 0..n {
        let f1 = 2.5 * rng.standard_normal();
        let f2 = 1.8 * rng.standard_normal();
        let row = [
            f1 + 0.25 * rng.standard_normal(),
            0.9 * f1 + 0.25 * rng.standard_normal(),
            -1.1 * f1 + 0.25 * rng.standard_normal(),
            f2 + 0.25 * rng.standard_normal(),
            0.85 * f2 + 0.25 * rng.standard_normal(),
            -0.95 * f2 + 0.25 * rng.standard_normal(),
        ];
        base_rows.extend_from_slice(&row);
    }
    let labels: Vec<String> = (1..=p).map(|i| format!("V{i}")).collect();

    let mat_unit = DenseMatrix::from_row_major(n, p, base_rows.clone()).unwrap();
    let mat_100 = DenseMatrix::from_row_major(n, p, base_rows.iter().map(|&v| v * 100.0).collect()).unwrap();
    let mat_01 = DenseMatrix::from_row_major(n, p, base_rows.iter().map(|&v| v * 0.1).collect()).unwrap();

    let pca_unit = pca_analysis(&mat_unit, &labels, false, Some(4242)).unwrap();
    let pca_100 = pca_analysis(&mat_100, &labels, false, Some(4242)).unwrap();
    let pca_01 = pca_analysis(&mat_01, &labels, false, Some(4242)).unwrap();

    assert_eq!(pca_unit.parallel_analysis_components, Some(2));
    assert_eq!(
        pca_100.parallel_analysis_components, pca_unit.parallel_analysis_components,
        "scaling all columns by 100.0 must retain the exact same component count"
    );
    assert_eq!(
        pca_01.parallel_analysis_components, pca_unit.parallel_analysis_components,
        "scaling all columns by 0.1 must retain the exact same component count"
    );

    let th_unit = pca_unit.parallel_analysis_thresholds.as_ref().unwrap();
    let th_100 = pca_100.parallel_analysis_thresholds.as_ref().unwrap();
    let th_01 = pca_01.parallel_analysis_thresholds.as_ref().unwrap();
    for j in 0..p {
        let r100 = th_100[j] / th_unit[j];
        let r01 = th_01[j] / th_unit[j];
        assert_close(r100, 10_000.0, 1e-6, &format!("PA threshold scale^2 (100x) [{j}]"));
        assert_close(r01, 0.01, 1e-11, &format!("PA threshold scale^2 (0.1x) [{j}]"));
    }

    // Pure Gaussian noise with SD ~ 100 on covariance PCA must NOT retain all components.
    let mut noise_data = Vec::with_capacity(40 * 5);
    for _ in 0..(40 * 5) {
        noise_data.push(100.0 * rng.standard_normal());
    }
    let noise_mat = DenseMatrix::from_row_major(40, 5, noise_data).unwrap();
    let noise_labels: Vec<String> = (1..=5).map(|i| format!("N{i}")).collect();
    let pca_noise = pca_analysis(&noise_mat, &noise_labels, false, Some(99)).unwrap();
    assert!(
        pca_noise.parallel_analysis_components.unwrap() <= 1,
        "SD=100 noise must not retain all 5 covariance components, got {:?}",
        pca_noise.parallel_analysis_components
    );
}

// ============================================================================
// RV-31: Kaplan-Meier NaN / Non-Finite / Negative Time Guards
// ============================================================================

#[test]
fn rv_31_kaplan_meier_rejects_nan_infinite_and_negative_times() {
    let conf = ConfidenceLevel::DEFAULT_95;
    for bad_time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0e-9, -5.0] {
        let obs = [
            SurvivalObservation { row: 0, time: 2.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 1, time: bad_time, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 2, time: 6.0, status: SurvivalStatus::Event },
        ];
        let err = kaplan_meier_curve("Cohort", &obs, true, conf).unwrap_err();
        assert!(
            matches!(err, StatsError::InvalidParameter { name: "survival_time", .. }),
            "expected InvalidParameter(survival_time) for time={bad_time}, got {err:?}"
        );
    }
}

// ============================================================================
// RV-32: Schoenfeld PH Test Non-Estimable When Events < 3
// ============================================================================

#[test]
fn rv_32_schoenfeld_ph_test_reports_non_estimable_with_fewer_than_three_events() {
    // Non-separated 2-event cohort (risk sets contain survivors both above and below event x values).
    let subjects_2evt = vec![
        CoxSubject { time: 2.0, event: true, x: vec![0.6] },
        CoxSubject { time: 4.0, event: false, x: vec![1.5] },
        CoxSubject { time: 6.0, event: true, x: vec![0.1] },
        CoxSubject { time: 9.0, event: false, x: vec![-0.8] },
        CoxSubject { time: 11.0, event: false, x: vec![0.9] },
        CoxSubject { time: 14.0, event: false, x: vec![-0.3] },
    ];
    let names = vec![String::from("x")];
    let cox = cox_ph_regression(&subjects_2evt, &names, CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(cox.n_events, 2);
    assert!(cox.converged);
    assert!(cox.global_schoenfeld_chi2.is_nan(), "expected NaN global chi2 for 2 events, got {}", cox.global_schoenfeld_chi2);
    assert!(cox.global_schoenfeld_p.is_nan(), "expected NaN global Schoenfeld p for 2 events, got {}", cox.global_schoenfeld_p);
    assert!(cox.covariates[0].schoenfeld_rho.is_nan());
    assert!(cox.covariates[0].schoenfeld_p.is_nan());

    // Sheet-level Cox PH with 2 events must omit Global Schoenfeld P scalar, leave Schoenfeld P cell
    // empty, and warn that the PH test is unavailable rather than passing with P = 1.0.
    let mut mv = MultipleVariablesTable::new(3, subjects_2evt.len()).unwrap();
    for (r, s) in subjects_2evt.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(s.time);
        mv.variables[1].cells[r] = Cell::number(if s.event { 1.0 } else { 0.0 });
        mv.variables[2].cells[r] = Cell::number(s.x[0]);
    }
    let sid = SheetId::new("rv32-cox");
    let ds = DataSheet::new(sid.clone(), "RV32 Cox", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("rv32-an"),
        sid,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    let rs = run_analysis(&ds, &spec).unwrap();
    assert!(!rs.scalars.iter().any(|s| s.key == "Global Schoenfeld P"));
    let cox_tab = rs.tabs.iter().find(|t| t.id == "cox-ph").unwrap();
    assert_eq!(cox_tab.rows[0][6], ResultCell::Empty, "Schoenfeld P cell must be Empty when n_events < 3");
    assert!(cox_tab.footnotes.iter().any(|f| f.contains("fewer than 3 events")));
    let ph_item = rs.checklist.iter().find(|c| c.assumption.contains("Schoenfeld")).unwrap();
    assert_eq!(ph_item.status, ChecklistStatus::Warn);

    // 3 events all tied at a single timestamp also have zero transformed-time variance -> NaN.
    let subjects_tied = vec![
        CoxSubject { time: 5.0, event: true, x: vec![0.8] },
        CoxSubject { time: 5.0, event: true, x: vec![-0.3] },
        CoxSubject { time: 5.0, event: true, x: vec![0.2] },
        CoxSubject { time: 10.0, event: false, x: vec![1.4] },
        CoxSubject { time: 12.0, event: false, x: vec![-1.1] },
        CoxSubject { time: 15.0, event: false, x: vec![0.1] },
    ];
    let cox_tied = cox_ph_regression(&subjects_tied, &names, CoxTieMethod::Efron, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(cox_tied.converged);
    assert!(cox_tied.global_schoenfeld_p.is_nan());
    assert!(cox_tied.covariates[0].schoenfeld_p.is_nan());
}

// ============================================================================
// RV-33: Standardized PCA Zero-Variance Guard & Deterministic Loading Signs
// ============================================================================

#[test]
fn rv_33_pca_zero_variance_guard_and_deterministic_sign_normalization() {
    let labels = vec![String::from("V1"), String::from("V2"), String::from("V3")];

    // 1. Standardized (correlation) PCA rejects constant zero-SD columns instead of distorting trace(Corr).
    let const_col_rows = [[1.0, 5.0, 2.1], [2.0, 5.0, 3.9], [3.0, 5.0, 6.2], [4.0, 5.0, 8.1], [5.0, 5.0, 9.8]];
    let mat_const = DenseMatrix::from_row_major(5, 3, const_col_rows.iter().flatten().copied().collect()).unwrap();
    let err_std = pca_analysis(&mat_const, &labels, true, None).unwrap_err();
    assert!(matches!(err_std, StatsError::ZeroVariance { .. }), "expected ZeroVariance on standardized PCA with constant column, got {err_std:?}");

    // Unstandardized (covariance) PCA allows one zero-variance column with exact trace(Cov),
    // while rejecting an all-constant matrix.
    let pca_cov_const = pca_analysis(&mat_const, &labels, false, None).unwrap();
    let v1: Vec<f64> = const_col_rows.iter().map(|r| r[0]).collect();
    let v3: Vec<f64> = const_col_rows.iter().map(|r| r[2]).collect();
    let expected_trace =
        SummaryMoments::from_slice(&v1).unwrap().sample_variance().unwrap() + SummaryMoments::from_slice(&v3).unwrap().sample_variance().unwrap();
    assert_close(pca_cov_const.eigenvalues.iter().sum(), expected_trace, 1e-12, "covariance PCA trace with one constant column");
    assert_close(*pca_cov_const.cumulative_proportion.last().unwrap(), 1.0, 1e-12, "cumulative proportion sums to 1");

    let all_const = DenseMatrix::from_row_major(4, 2, vec![7.0; 8]).unwrap();
    assert!(matches!(pca_analysis(&all_const, &[String::from("A"), String::from("B")], false, None), Err(StatsError::ZeroVariance { .. })));

    // 2. Deterministic loading signs: largest-magnitude loading of each component is >= 0,
    // X_scaled = Scores * Loadings^T holds, and negating X preserves loadings while negating scores.
    let rows = [[-2.5, -1.8, 3.1], [-1.9, -2.2, 2.4], [-0.8, -0.5, 1.1], [0.7, 0.9, -0.8], [1.8, 2.1, -2.6], [2.7, 1.5, -3.2]];
    let mat_pos = DenseMatrix::from_row_major(6, 3, rows.iter().flatten().copied().collect()).unwrap();
    let mat_neg = DenseMatrix::from_row_major(6, 3, rows.iter().flatten().map(|&v| -v).collect()).unwrap();

    let pca_pos = pca_analysis(&mat_pos, &labels, true, None).unwrap();
    let pca_neg = pca_analysis(&mat_neg, &labels, true, None).unwrap();

    for j in 0..3 {
        let max_abs = (0..3).map(|i| pca_pos.loadings[i][j].abs()).fold(0.0_f64, f64::max);
        let pivot_loading = (0..3).map(|i| pca_pos.loadings[i][j]).find(|v| v.abs() >= max_abs * (1.0 - 1e-12)).unwrap();
        assert!(pivot_loading > 0.0, "PC{} largest-magnitude loading must be positive, got {pivot_loading}", j + 1);
        for i in 0..3 {
            assert_close(pca_pos.loadings[i][j], pca_neg.loadings[i][j], 1e-12, "loading sign invariance under X -> -X");
        }
        for r in 0..6 {
            assert_close(pca_pos.scores[r][j], -pca_neg.scores[r][j], 1e-12, "score sign equivariance under X -> -X");
        }
    }
}

// ============================================================================
// RV-34: Passing & Bablok (1983) Vertical Slopes (+/-inf) Included in N and K
// ============================================================================

#[test]
fn rv_34_passing_bablok_includes_vertical_pairs_in_n_and_k_and_preserves_symmetry() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // 5 points with repeated x = 2.0 (distinct y: 5.0 vs 20.0 -> vertical slope):
    // Points: (1, 2), (2, 5), (2, 20), (4, 8), (6, 11).
    // All 10 pairs:
    // (0,1): (5-2)/(2-1) = 3.0
    // (0,2): (20-2)/(2-1) = 18.0
    // (0,3): (8-2)/(4-1) = 2.0
    // (0,4): (11-2)/(6-1) = 1.8
    // (1,2): dx=0, dy=+15 -> +inf (included in N!)
    // (1,3): (8-5)/(4-2) = 1.5
    // (1,4): (11-5)/(6-2) = 1.5
    // (2,3): (8-20)/(4-2) = -6.0 (< -1, contributes to K!)
    // (2,4): (11-20)/(6-2) = -2.25 (< -1, contributes to K!)
    // (3,4): (11-8)/(6-4) = 1.5
    // Sorted slopes (N = 10): [-6.0, -2.25, 1.5, 1.5, 1.5, 1.8, 2.0, 3.0, 18.0, +inf], K = 2.
    // Shifted angular array: [1.5, 1.5, 1.5, 1.8, 2.0, 3.0, 18.0, +inf, -6.0, -2.25].
    // Exact Passing-Bablok median of N = 10 elements (indices 4 and 5): 0.5 * (2.0 + 3.0) = 2.5!
    // (Dropping the dx = 0 pair previously gave N = 9 and median angular[4] = 2.0!)
    let xs = [1.0, 2.0, 2.0, 4.0, 6.0];
    let ys = [2.0, 5.0, 20.0, 8.0, 11.0];
    let pb = passing_bablok_regression(&xs, &ys, conf).unwrap();
    assert_eq!(pb.valid_slopes, 10);
    assert_eq!(pb.k_offset, 2);
    assert_close(pb.slope, 2.5, 1e-12, "Passing-Bablok exact slope with dy > 0 vertical pair");

    // Swapping the two tied-x rows reverses dy (+15 -> -15), turning +inf into -inf and increasing
    // K from 2 to 3 while keeping the exact same shifted angular median slope = 2.5 and intercept!
    let xs_swapped = [1.0, 2.0, 2.0, 4.0, 6.0];
    let ys_swapped = [2.0, 20.0, 5.0, 8.0, 11.0];
    let pb_swapped = passing_bablok_regression(&xs_swapped, &ys_swapped, conf).unwrap();
    assert_eq!(pb_swapped.valid_slopes, 10);
    assert_eq!(pb_swapped.k_offset, 3);
    assert_close(pb_swapped.slope, pb.slope, 1e-12, "row-swap slope invariance on vertical pair");
    assert_close(pb_swapped.intercept, pb.intercept, 1e-12, "row-swap intercept invariance on vertical pair");

    // Identical duplicate point (dx == 0 && dy == 0) is still excluded from N.
    let xs_dup = [1.0, 2.0, 2.0, 4.0, 6.0];
    let ys_dup = [2.0, 5.0, 5.0, 8.0, 11.0];
    let pb_dup = passing_bablok_regression(&xs_dup, &ys_dup, conf).unwrap();
    assert_eq!(pb_dup.valid_slopes, 9);

    // X <-> Y exact inverse equivariance on odd N = 21 (n = 7) when horizontal ties in Y become vertical ties in X:
    let hx = [1.0, 2.0, 3.0, 4.5, 6.0, 8.0, 10.0];
    let hy = [3.0, 5.2, 5.2, 10.1, 13.0, 17.4, 21.0];
    let pb_xy = passing_bablok_regression(&hx, &hy, conf).unwrap();
    let pb_yx = passing_bablok_regression(&hy, &hx, conf).unwrap();
    assert_eq!(pb_xy.valid_slopes, 21);
    assert_eq!(pb_xy.valid_slopes, pb_yx.valid_slopes, "horizontal/vertical tie swap preserves N");
    assert_close(pb_yx.slope, 1.0 / pb_xy.slope, 1e-12, "X<->Y reciprocal slope with tied values");
    assert_close(pb_yx.intercept, -pb_xy.intercept / pb_xy.slope, 1e-11, "X<->Y inverse intercept with tied values");
}

// ============================================================================
// RV-35: Paired DeLong Structural Component Length & Finiteness Validation
// ============================================================================

#[test]
fn rv_35_paired_delong_validates_v10_and_v01_lengths_without_panicking() {
    let ctrls = [1.0, 1.5, 2.0, 2.4, 3.0];
    let cases_a = [2.2, 3.1, 3.8, 4.5, 5.2];
    let cases_b = [2.0, 2.9, 3.9, 4.1, 5.5];
    let roc_a = roc_curve_analysis(&ctrls, &cases_a, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).unwrap();
    let roc_b = roc_curve_analysis(&ctrls, &cases_b, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(compare_roc_delong_paired(&roc_a, &roc_b).is_ok());

    let mut short_v10 = roc_b.clone();
    short_v10.delong_v10.pop();
    assert!(matches!(compare_roc_delong_paired(&roc_a, &short_v10), Err(StatsError::DimensionMismatch { expected: 5, actual: 4 })));

    let mut short_v01 = roc_a.clone();
    short_v01.delong_v01.pop();
    assert!(matches!(compare_roc_delong_paired(&short_v01, &roc_b), Err(StatsError::DimensionMismatch { expected: 5, actual: 4 })));

    let mut long_v01 = roc_b.clone();
    long_v01.delong_v01.push(0.5);
    assert!(matches!(compare_roc_delong_paired(&roc_a, &long_v01), Err(StatsError::DimensionMismatch { expected: 5, actual: 6 })));

    let mut nan_v10 = roc_b;
    nan_v10.delong_v10[2] = f64::NAN;
    assert!(matches!(compare_roc_delong_paired(&roc_a, &nan_v10), Err(StatsError::Math(MathError::DomainError { .. }))));
}

// ============================================================================
// RV-67: Survival Pairwise Log-Rank Multiplicity (Sidak, Dunnett, Footnotes)
// ============================================================================

fn build_four_cohort_survival_sheet() -> (DataSheet, [Vec<SurvivalObservation>; 4]) {
    let raw_cohorts: [(&str, &[(f64, bool)]); 4] = [
        ("Control", &[(2.0, true), (3.0, true), (4.0, true), (5.0, true), (6.0, true), (8.0, false)]),
        ("LowDose", &[(4.0, true), (6.0, true), (7.0, true), (9.0, true), (11.0, false), (12.0, true)]),
        ("MidDose", &[(7.0, true), (9.0, true), (12.0, true), (14.0, false), (15.0, true), (18.0, false)]),
        ("HighDose", &[(11.0, true), (14.0, false), (16.0, true), (19.0, false), (22.0, true), (25.0, false)]),
    ];
    let total_rows = raw_cohorts.iter().map(|(_, v)| v.len()).sum();
    let mut table = SurvivalTable::new(4, total_rows).unwrap();
    let mut obs_arr: [Vec<SurvivalObservation>; 4] = Default::default();
    let mut row_cursor = 0_usize;
    for (c_idx, (title, items)) in raw_cohorts.iter().enumerate() {
        table.cohorts[c_idx].title = (*title).to_owned();
        for &(t, ev) in *items {
            table.times[row_cursor] = Cell::number(t);
            table.cohorts[c_idx].cells[row_cursor][0] = Cell::number(if ev { 1.0 } else { 0.0 });
            obs_arr[c_idx].push(SurvivalObservation {
                row: row_cursor,
                time: t,
                status: if ev { SurvivalStatus::Event } else { SurvivalStatus::Censored },
            });
            row_cursor += 1;
        }
    }
    let sheet = DataSheet::new(SheetId::new("rv67-surv"), "RV67 Survival", TableArchetype::Survival(table)).unwrap();
    (sheet, obs_arr)
}

#[test]
fn rv_67_survival_pairwise_sidak_dunnett_control_only_and_anova_fallback_footnotes() {
    let (sheet, obs) = build_four_cohort_survival_sheet();
    let mk_spec = |id: &str, method: MultiplicityMethod| {
        AnalysisSpec::new(
            SheetId::new(id),
            sheet.id.clone(),
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox: true,
                gehan_breslow_wilcoxon: false,
                trend_test: false,
                asymmetrical_greenwood_ci: true,
                multiplicity: MultiplicitySpec { method, threshold: 0.05, report_confidence_intervals: true },
                mv_time_column: None,
                mv_status_column: None,
                mv_group_column: None,
            }),
        )
        .unwrap()
    };

    // 1. Single-step Sidak uses 1 - (1 - p)^6 across all 6 pairs (distinct from step-down Holm-Sidak).
    let mut all_raw_p = Vec::new();
    for i in 0..4 {
        for j in (i + 1)..4 {
            all_raw_p.push(compare_survival_curves(&[&obs[i], &obs[j]], false).unwrap().p_value);
        }
    }
    let exp_sidak = adjust_sidak(&all_raw_p);
    let exp_holm_sidak = adjust_holm_sidak(&all_raw_p);
    assert!(
        exp_sidak.iter().zip(exp_holm_sidak.iter()).any(|(a, b)| (a - b).abs() > 1e-4),
        "fixture must distinguish single-step Sidak from step-down Holm-Sidak"
    );

    let rs_sidak = run_analysis(&sheet, &mk_spec("rv67-sidak", MultiplicityMethod::Sidak)).unwrap();
    assert_eq!(rs_sidak.pairwise_comparisons.len(), 6);
    for (rec, &expected_p) in rs_sidak.pairwise_comparisons.iter().zip(exp_sidak.iter()) {
        assert_close(rec.raw_p_or_q, expected_p, 1e-12, "Sidak pairwise log-rank adjusted P");
    }
    let tab_sidak = rs_sidak.tabs.iter().find(|t| t.id == "survival-pairwise").unwrap();
    assert!(tab_sidak.footnotes.is_empty(), "native Sidak should not emit substitution footnote");

    // 2. Dunnett compares ONLY the K - 1 = 3 treatment cohorts against the designated control cohort.
    let rs_dunnett = run_analysis(&sheet, &mk_spec("rv67-dunnett", MultiplicityMethod::Dunnett { control_column: 0 })).unwrap();
    assert_eq!(rs_dunnett.pairwise_comparisons.len(), 3, "Dunnett on 4 cohorts must emit 3 vs-control comparisons, not 6");
    for (idx, rec) in rs_dunnett.pairwise_comparisons.iter().enumerate() {
        let trt_col = idx + 1;
        assert_eq!((rec.group_a, rec.group_b), (trt_col, 0));
        assert_eq!(rec.label_b, "Control");
        let rp = compare_survival_curves(&[&obs[trt_col], &obs[0]], false).unwrap().p_value;
        assert_close(rec.raw_p_or_q, (3.0 * rp).min(1.0), 1e-12, "Dunnett vs-control 3-comparison Bonferroni P");
    }
    let tab_dunnett = rs_dunnett.tabs.iter().find(|t| t.id == "survival-pairwise").unwrap();
    assert!(tab_dunnett.footnotes.iter().any(|f| f.contains("Dunnett") && f.contains("Control") && f.contains("Bonferroni")));
    assert!(rs_dunnett.checklist.iter().any(|c| c.assumption.contains("multiplicity") && c.explanation.contains("Dunnett")));

    // Out-of-bounds Dunnett control_column returns InvalidParameter("control_column").
    let err_bad_ctrl = run_analysis(&sheet, &mk_spec("rv67-bad-ctrl", MultiplicityMethod::Dunnett { control_column: 9 })).unwrap_err();
    assert!(matches!(err_bad_ctrl, StatsError::InvalidParameter { name: "control_column", .. }));

    // 3. ANOVA-specific methods (e.g. TukeyHsd, GamesHowell) emit an explicit Bonferroni fallback footnote + checklist item.
    for (id, method, expected_name) in
        [("rv67-tukey", MultiplicityMethod::TukeyHsd, "Tukey-Kramer"), ("rv67-gh", MultiplicityMethod::GamesHowell, "Games-Howell")]
    {
        let rs_fb = run_analysis(&sheet, &mk_spec(id, method)).unwrap();
        assert_eq!(rs_fb.pairwise_comparisons.len(), 6);
        let tab_fb = rs_fb.tabs.iter().find(|t| t.id == "survival-pairwise").unwrap();
        assert!(
            tab_fb.footnotes.iter().any(|f| f.contains(expected_name) && f.contains("Bonferroni")),
            "missing fallback footnote for {expected_name}: {:?}",
            tab_fb.footnotes
        );
        assert!(rs_fb.checklist.iter().any(|c| c.explanation.contains(expected_name) && c.explanation.contains("Bonferroni")));
    }
}

// ============================================================================
// RV-74: Power Curve Respects Explicit `sample_size_group2`
// ============================================================================

#[test]
fn rv_74_power_curve_respects_fixed_and_paired_sample_size_group2() {
    // 1. Fixed sample_size_group2 without sample_size: curve varies n1 while keeping n2 fixed at 60.
    let fixed_n2_req = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(60.0).with_allocation_ratio(1.0);

    let direct_curve = compute_power_curve(&fixed_n2_req, 10, 50, 5).unwrap();
    assert_eq!(direct_curve.len(), 5);
    for pt in &direct_curve {
        assert_close(pt.total_n, pt.n + 60.0, 1e-12, "fixed n2=60 total_n on direct curve");
        let pt_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
            .with_target(PowerSolveTarget::Power)
            .with_sample_size(pt.n)
            .with_sample_size_group2(60.0)
            .with_alpha(0.05)
            .with_effect(PowerEffectSpec::Standardized(0.5));
        let pt_solved = solve_power(&pt_req).unwrap();
        assert_close(pt.power, pt_solved.achieved_power, 1e-12, "curve power matches exact (n1, 60) point power");
    }

    // SampleSize solve with fixed n2 = 80 solves n1 against n2 = 80 and aligns with its own power_curve.
    let solve_fixed_n2 = PowerAnalysisRequest::unpaired_t(0.5, 0.05, 0.80).with_sample_size_group2(80.0);
    let solved = solve_power(&solve_fixed_n2).unwrap();
    assert_eq!(solved.n_group2, Some(80));
    assert_eq!(solved.total_n, solved.n_per_group + 80);
    let center_pt = solved.power_curve.iter().find(|pt| (pt.n - (solved.n_per_group as f64)).abs() < 0.5).unwrap();
    assert_close(center_pt.total_n, solved.total_n as f64, 1e-12, "solved curve center total_n with fixed n2=80");
    assert_close(center_pt.power, solved.achieved_power, 1e-12, "solved curve center power matches achieved_power");

    // 2. Explicit (n1 = 11, n2 = 25) where IEEE-754 11.0 * (25.0 / 11.0) = 25.000000000000004 > 25.0:
    // BOTH solve_power's internal power_curve AND direct compute_power_curve at n = 11 must report
    // exact total_n = 36 (not 37) and match achieved_power to machine precision, and DetectableEffect
    // sensitivity_table must also preserve exact (n1 = 11, total_n = 36).
    let k_ulp = 25.0_f64 / 11.0_f64;
    assert!(11.0_f64 * k_ulp > 25.0_f64, "verify IEEE-754 1-ULP overshoot precondition");

    let pair_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::Power)
        .with_sample_size(11.0)
        .with_sample_size_group2(25.0)
        .with_allocation_ratio(1.0)
        .with_alpha(0.05)
        .with_effect(PowerEffectSpec::Standardized(0.7));
    let pair_res = solve_power(&pair_req).unwrap();
    assert_eq!((pair_res.n_per_group, pair_res.n_group2, pair_res.total_n), (11, Some(25), 36));
    let res_pt_11 = pair_res.power_curve.iter().find(|pt| (pt.n - 11.0).abs() < 0.5).unwrap();
    assert_close(res_pt_11.total_n, 36.0, 1e-12, "solve_power.power_curve at n=11 exact total_n=36");
    assert_close(res_pt_11.power, pair_res.achieved_power, 1e-14, "solve_power.power_curve at n=11 matches achieved_power");

    let pair_curve = compute_power_curve(&pair_req, 11, 22, 12).unwrap();
    assert_close(pair_curve[0].n, 11.0, 1e-12, "curve n=11");
    assert_close(pair_curve[0].total_n, 36.0, 1e-12, "direct compute_power_curve exact total_n=36 without ULP overshoot");
    assert_close(pair_curve[0].power, pair_res.achieved_power, 1e-14, "direct curve power at n=11 matches achieved_power");

    let mde_pair_req = PowerAnalysisRequest::new(PowerTestFamily::TwoSampleUnpairedT)
        .with_target(PowerSolveTarget::DetectableEffect)
        .with_sample_size(11.0)
        .with_sample_size_group2(25.0)
        .with_allocation_ratio(1.0)
        .with_alpha(0.05)
        .with_target_power(0.80);
    let mde_pair_res = solve_power(&mde_pair_req).unwrap();
    assert_eq!((mde_pair_res.n_per_group, mde_pair_res.n_group2, mde_pair_res.total_n), (11, Some(25), 36));
    let mde_pt_11 = mde_pair_res.power_curve.iter().find(|pt| (pt.n - 11.0).abs() < 0.5).unwrap();
    assert_close(mde_pt_11.total_n, 36.0, 1e-12, "MDE power_curve at n=11 exact total_n=36");
    assert_close(mde_pt_11.power, mde_pair_res.achieved_power, 1e-14, "MDE power_curve at n=11 matches achieved_power");
    assert!(!mde_pair_res.sensitivity_table.is_empty());
    for cell in &mde_pair_res.sensitivity_table {
        assert_eq!((cell.sample_size, cell.total_n), (11, 36), "MDE sensitivity_table must preserve exact (11, 25)");
    }
    let direct_mde_sens = compute_sensitivity_table(&mde_pair_req, &[0.05, 0.01], &[0.80, 0.90]).unwrap();
    for cell in &direct_mde_sens {
        assert_eq!((cell.sample_size, cell.total_n), (11, 36));
    }
}
