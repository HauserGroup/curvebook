#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression and D1 oracle tests for AUD-05 (`simple_logistic`, `regression`, `survival`,
//! `categorical`) and the `crates/stats` halves of AUD-13-06 and AUD-13-16.
//!
//! Literature references:
//! - Hosmer, D. W., Lemeshow, S., & Sturdivant, R. X. (2013). *Applied Logistic Regression* (3rd ed.). Wiley.
//! - Venzon, D. J., & Moolgavkar, S. H. (1988). Profile-likelihood confidence intervals. *JRSS C*, 37(1), 87-94.
//! - Passing, H., & Bablok, W. (1983). A new biometrical procedure for testing the equality of measurements from
//!   two different analytical methods. *J. Clin. Chem. Clin. Biochem.*, 21(11), 709-720.
//! - Fleiss, J. L., Nee, J. C. M., & Landis, J. R. (1979). Large sample variance of kappa in the case of
//!   different sets of raters. *Psychological Bulletin*, 86(5), 974-977.
//! - Barnard, G. A. (1947). Significance tests for 2 x 2 tables. *Biometrika*, 34(1-2), 123-138.
//! - McNemar, Q. (1947). Note on the sampling error of the difference between correlated proportions. *Psychometrika*, 12(2), 153-157.
//! - Cox, D. R. (1972). Regression models and life-tables. *JRSS B*, 34(2), 187-220.
//! - Harrell, F. E., et al. (1982). Evaluating the yield of medical tests. *JAMA*, 247(18), 2543-2546.
//! - Durbin, J., & Watson, G. S. (1950). Testing for serial correlation in least squares regression. *Biometrika*, 37, 409-428.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ChecklistStatus, ContingencyMethod, ContingencySpec, ContingencyTable, CoxTieMethod, DataSheet,
    GlmFamily, GlmSpec, MultipleVariablesTable, PassingBablokSpec, ResultCell, SimpleLogisticSpec, SubcolumnMode, SurvivalAnalysisSpec,
    SurvivalObservation, SurvivalStatus, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::categorical::{
    BarnardOrdering, analyze_2x2_ratios, analyze_contingency, barnard_exact_2x2_with_ordering, cochran_armitage_trend, fisher_exact_rxc, fleiss_kappa,
};
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression, passing_bablok_regression};
use curvebook_stats::simple_logistic::{SimpleLogisticSeparation, run_simple_logistic_analysis, simple_logistic_regression};
use curvebook_stats::survival::{CoxSubject, cox_ph_regression, kaplan_meier_curve};
use curvebook_stats::{StatsError, run_analysis};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{label}: actual={actual:.15}, expected={expected:.15}, diff={:.3e} > tol={tol:.3e}",
        (actual - expected).abs()
    );
}

fn assert_rel_close(actual: f64, expected: f64, rel_tol: f64, label: &str) {
    let scale = expected.abs().max(1e-30);
    let diff = (actual - expected).abs();
    assert!(diff <= rel_tol * scale, "{label}: actual={actual:.15e}, expected={expected:.15e}, rel_diff={:.3e} > {rel_tol:.3e}", diff / scale);
}

fn make_contingency_sheet(id: &str, rows: &[&[u64]]) -> DataSheet {
    let n_rows = rows.len();
    let n_cols = rows[0].len();
    let mut ct = ContingencyTable::new(n_cols, n_rows).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, &val) in row.iter().enumerate() {
            ct.data_sets[c].cells[r] = vec![Cell::number(val as f64)];
        }
    }
    DataSheet::new(id, id, TableArchetype::Contingency(ct)).unwrap()
}

fn trapezoidal_auc(pts: &[(f64, f64)]) -> f64 {
    let mut sorted = pts.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    sorted.windows(2).map(|w| 0.5 * (w[1].0 - w[0].0) * (w[0].1 + w[1].1)).sum()
}

// ----------------------------------------------------------------------------
// AUD-05-01, AUD-05-25, AUD-05-26, AUD-05-27: Simple Logistic Numerics & Guards
// ----------------------------------------------------------------------------

#[test]
fn aud_05_01_simple_logistic_profile_ci_shift_and_steep_invariance() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let x_base: Vec<f64> = (0..=10).map(f64::from).collect();
    let x_age: Vec<f64> = (50..=60).map(f64::from).collect();
    let x_year: Vec<f64> = (2010..=2020).map(f64::from).collect();
    let x_steep: Vec<f64> = (0..=10).map(|i| (i as f64) * 1e-3).collect();

    let r_base = simple_logistic_regression(&x_base, &y, &[], 0.5, conf).unwrap();
    let r_age = simple_logistic_regression(&x_age, &y, &[], 0.5, conf).unwrap();
    let r_year = simple_logistic_regression(&x_year, &y, &[], 0.5, conf).unwrap();
    let r_steep = simple_logistic_regression(&x_steep, &y, &[], 0.5, conf).unwrap();

    // Calendar years & ages must yield exact same profile CI for beta1 and OR as unshifted X.
    // Exact profile-likelihood 95% CI oracle (Venzon & Moolgavkar 1988): [0.1263171675, 1.8160987638].
    assert_close(r_base.profile_ci_beta1.lower.unwrap(), 0.126_317_167_5, 1e-6, "base ci_b1.lo");
    assert_close(r_base.profile_ci_beta1.upper.unwrap(), 1.816_098_763_8, 1e-6, "base ci_b1.hi");
    assert_close(r_age.profile_ci_beta1.lower.unwrap(), r_base.profile_ci_beta1.lower.unwrap(), 1e-8, "age ci_b1.lo");
    assert_close(r_age.profile_ci_beta1.upper.unwrap(), r_base.profile_ci_beta1.upper.unwrap(), 1e-8, "age ci_b1.hi");
    assert_close(r_year.profile_ci_beta1.lower.unwrap(), r_base.profile_ci_beta1.lower.unwrap(), 1e-8, "year ci_b1.lo");
    assert_close(r_year.profile_ci_beta1.upper.unwrap(), r_base.profile_ci_beta1.upper.unwrap(), 1e-8, "year ci_b1.hi");
    assert_close(r_year.odds_ratio_ci_beta1.lower.unwrap(), r_base.odds_ratio_ci_beta1.lower.unwrap(), 1e-7, "year ci_or1.lo");
    assert_close(r_year.odds_ratio_ci_beta1.upper.unwrap(), r_base.odds_ratio_ci_beta1.upper.unwrap(), 1e-7, "year ci_or1.hi");

    // Calendar years beta0 profile CI must span [-3660.348752, -254.717654] (not pinched near MLE -1401.8).
    assert_close(r_year.profile_ci_beta0.lower.unwrap(), -3_660.348_752, 1e-3, "year ci_b0.lo");
    assert_close(r_year.profile_ci_beta0.upper.unwrap(), -254.717_654, 1e-3, "year ci_b0.hi");

    // X50 profile CI must be shift- and scale-invariant even on steep curves.
    assert_close(r_year.x50.unwrap(), r_base.x50.unwrap() + 2010.0, 1e-8, "year x50");
    assert_close(r_year.x50_profile_ci.lower.unwrap(), r_base.x50_profile_ci.lower.unwrap() + 2010.0, 1e-6, "year x50 ci lo");
    assert_close(r_year.x50_profile_ci.upper.unwrap(), r_base.x50_profile_ci.upper.unwrap() + 2010.0, 1e-6, "year x50 ci hi");
    assert_close(r_steep.x50.unwrap() * 1e3, r_base.x50.unwrap(), 1e-8, "steep x50");
    assert_close(r_steep.x50_profile_ci.lower.unwrap() * 1e3, r_base.x50_profile_ci.lower.unwrap(), 1e-6, "steep x50 ci lo");
    assert_close(r_steep.x50_profile_ci.upper.unwrap() * 1e3, r_base.x50_profile_ci.upper.unwrap(), 1e-6, "steep x50 ci hi");
}

#[test]
fn aud_05_02_simple_logistic_xy_replicate_x_and_presum_validation() {
    let x_means = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let y_vals = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];

    let mut xy_rep = XyTable::new(SubcolumnMode::Single, 1, x_means.len()).unwrap();
    xy_rep.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) };
    xy_rep.x_cells = vec![vec![Cell::empty(); 3]; x_means.len()];
    for (r, (&xm, &yv)) in x_means.iter().zip(y_vals.iter()).enumerate() {
        // Subcolumn 0 is intentionally offset by -1.0; the mean of [xm - 1, xm, xm + 1] is xm.
        xy_rep.x_cells[r][0] = Cell::number(xm - 1.0);
        xy_rep.x_cells[r][1] = Cell::number(xm);
        xy_rep.x_cells[r][2] = Cell::number(xm + 1.0);
        xy_rep.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let sheet_rep = DataSheet::new("xy-xrep", "X Replicates", TableArchetype::Xy(xy_rep)).unwrap();
    let spec_rep = AnalysisSpec::new("res-xrep", "xy-xrep", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
    let res_rep = run_simple_logistic_analysis(&sheet_rep, &spec_rep, &SimpleLogisticSpec::default()).unwrap();
    let oracle = simple_logistic_regression(&x_means, &y_vals, &[], 0.5, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(res_rep.lookup_scalar("Beta0", Some(0)).unwrap(), oracle.beta0, 1e-10, "replicate X mean beta0");
    assert_close(res_rep.lookup_scalar("Beta1", Some(0)).unwrap(), oracle.beta1, 1e-10, "replicate X mean beta1");

    // Pre-summarized X with invalid negative SD must return an error rather than silently bypassing validation.
    let mut xy_bad = XyTable::new(SubcolumnMode::Single, 1, x_means.len()).unwrap();
    xy_bad.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN };
    xy_bad.x_cells = vec![vec![Cell::empty(); 3]; x_means.len()];
    for (r, (&xm, &yv)) in x_means.iter().zip(y_vals.iter()).enumerate() {
        xy_bad.x_cells[r][0] = Cell::number(xm);
        xy_bad.x_cells[r][1] = Cell::number(if r == 2 { -2.0 } else { 0.5 });
        xy_bad.x_cells[r][2] = Cell::number(3.0);
        xy_bad.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let sheet_bad = DataSheet::new("xy-xbad", "Bad PreSum X", TableArchetype::Xy(xy_bad)).unwrap();
    let spec_bad = AnalysisSpec::new("res-xbad", "xy-xbad", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
    assert!(run_simple_logistic_analysis(&sheet_bad, &spec_bad, &SimpleLogisticSpec::default()).is_err());
}

#[test]
fn aud_05_25_simple_logistic_extreme_scales_and_x50() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let y = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let x_unit = [1.0, 2.0, 3.0, 4.0, 5.0, 2.0, 4.0];
    let x_tiny: Vec<f64> = x_unit.iter().map(|&v| v * 1e-13).collect();
    let r_unit = simple_logistic_regression(&x_unit, &y, &[], 0.5, conf).unwrap();
    let r_tiny = simple_logistic_regression(&x_tiny, &y, &[], 0.5, conf).unwrap();
    assert!(r_tiny.converged);
    assert_eq!(r_tiny.separation, SimpleLogisticSeparation::None);
    assert_rel_close(r_tiny.beta1 * 1e-13, r_unit.beta1, 1e-9, "sub-picomolar beta1");
    assert_rel_close(r_tiny.x50.unwrap() * 1e13, r_unit.x50.unwrap(), 1e-9, "sub-picomolar x50");

    let y10 = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let x10: Vec<f64> = (1..=10).map(f64::from).collect();
    let x_huge: Vec<f64> = x10.iter().map(|&v| v * 1e16).collect();
    let r10 = simple_logistic_regression(&x10, &y10, &[], 0.5, conf).unwrap();
    let r_huge = simple_logistic_regression(&x_huge, &y10, &[], 0.5, conf).unwrap();
    assert!(r_huge.converged);
    assert_rel_close(r_huge.x50.expect("huge-X must retain finite X50"), r10.x50.unwrap() * 1e16, 1e-9, "huge-X x50");
}

#[test]
fn aud_05_26_simple_logistic_centered_prediction_ci_and_overlay_bands() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let y = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let x_base: Vec<f64> = (0..=10).map(f64::from).collect();
    let r_base = simple_logistic_regression(&x_base, &y, &[(11, 5.0)], 0.5, conf).unwrap();

    for offset in [1.0e8_f64, 1.7e9_f64] {
        let x_off: Vec<f64> = x_base.iter().map(|&v| v + offset).collect();
        let r_off = simple_logistic_regression(&x_off, &y, &[(11, 5.0 + offset)], 0.5, conf).unwrap();
        assert!(r_off.converged);
        for (pb, po) in r_base.row_predictions.iter().zip(r_off.row_predictions.iter()) {
            assert_close(po.predicted_prob, pb.predicted_prob, 1e-9, "offset pred prob");
            assert_close(po.prob_ci.0, pb.prob_ci.0, 1e-8, "offset pred ci lo");
            assert_close(po.prob_ci.1, pb.prob_ci.1, 1e-8, "offset pred ci hi");
        }

        // Verify sigmoid overlay confidence bands also stay cancellation-free on shifted X.
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, x_off.len()).unwrap();
        for (r, (&xv, &yv)) in x_off.iter().zip(y.iter()).enumerate() {
            xy.x_cells[r][0] = Cell::number(xv);
            xy.data_sets[0].cells[r][0] = Cell::number(yv);
        }
        let sheet = DataSheet::new("xy-ts", "Timestamps", TableArchetype::Xy(xy)).unwrap();
        let sl = SimpleLogisticSpec { confidence_bands: true, plot_segments: Some(10), ..Default::default() };
        let spec = AnalysisSpec::new("res-ts", "xy-ts", AnalysisKind::SimpleLogisticRegression(sl)).unwrap();
        let rs = run_analysis(&sheet, &spec).unwrap();
        let sigmoid = rs.fitted_curves.iter().find(|c| c.equation_id == "simple_logistic").unwrap();
        let mid_pt = &sigmoid.points[5];
        let base_mid = r_base.row_predictions.iter().find(|r| r.is_unknown_row).unwrap();
        assert_close(mid_pt.y, base_mid.predicted_prob, 1e-9, "overlay mid p");
        assert_close(mid_pt.ci_lower.unwrap(), base_mid.prob_ci.0, 1e-8, "overlay mid ci lo");
        assert_close(mid_pt.ci_upper.unwrap(), base_mid.prob_ci.1, 1e-8, "overlay mid ci hi");
    }
}

#[test]
fn aud_05_27_simple_logistic_dimension_mismatch_and_nonfinite_y() {
    let err = simple_logistic_regression(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.0, 1.0, 1.0], &[], 0.5, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err, StatsError::DimensionMismatch { expected: 5, actual: 3 }));

    for bad_y in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
        let ys = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, bad_y];
        for (r, &yv) in ys.iter().enumerate() {
            xy.x_cells[r][0] = Cell::number((r + 1) as f64);
            xy.data_sets[0].cells[r][0] = Cell { value: CellValue::Number(yv), excluded: false };
        }
        let sheet = DataSheet::new("xy-bad-y", "Non-finite Y", TableArchetype::Xy(xy)).unwrap();
        let spec = AnalysisSpec::new("res-bad-y", "xy-bad-y", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
        let err = run_simple_logistic_analysis(&sheet, &spec, &SimpleLogisticSpec::default()).unwrap_err();
        assert!(matches!(err, StatsError::InvalidParameter { .. }), "expected InvalidParameter for {bad_y}, got {err:?}");
    }
}

// ----------------------------------------------------------------------------
// AUD-13-06 & AUD-13-16 (stats side): ROC Overlays & Per-Dataset Scalars
// ----------------------------------------------------------------------------

#[test]
fn aud_13_06_and_aud_13_16_simple_logistic_and_glm_roc_overlay_and_scalars() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 10).unwrap();
    xy.data_sets[0].title = String::from("DrugA");
    xy.data_sets[1].title = String::from("DrugB");
    let xs = [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let y0 = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let y1 = [1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    for r in 0..xs.len() {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        xy.data_sets[0].cells[r][0] = Cell::number(y0[r]);
        xy.data_sets[1].cells[r][0] = Cell::number(y1[r]);
    }
    let sheet = DataSheet::new("xy-multi", "Multi Logistic", TableArchetype::Xy(xy)).unwrap();
    let spec = AnalysisSpec::new("res-multi", "xy-multi", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
    let rs = run_analysis(&sheet, &spec).unwrap();

    // AUD-13-16: Per-dataset scalars use standard unprefixed keys with data_set_index = Some(ds_idx).
    let b0_a = rs.lookup_scalar("Beta0", Some(0)).expect("DrugA Beta0");
    let b1_a = rs.lookup_scalar("Beta1", Some(0)).expect("DrugA Beta1");
    let b0_b = rs.lookup_scalar("Beta0", Some(1)).expect("DrugB Beta0");
    let b1_b = rs.lookup_scalar("Beta1", Some(1)).expect("DrugB Beta1");
    assert!(b1_a > 0.0 && b1_b < 0.0, "expected opposite slopes: a={b1_a}, b={b1_b}");
    assert!(b0_a.is_finite() && b0_b.is_finite());
    assert!(rs.lookup_scalar("X50", Some(0)).is_some());
    assert!(rs.lookup_scalar("X50", Some(1)).is_some());
    assert!(rs.scalars.iter().all(|s| s.data_set_index.is_some()));

    // AUD-13-06: Emits simple_logistic BEFORE ROC for each dataset so XY overlays resolve simple_logistic
    // while ROC diagnostic plots resolve equation_id == "ROC" with exact DeLong AUC.
    assert_eq!(rs.fitted_curves.len(), 4);
    assert_eq!(rs.fitted_curves[0].equation_id, "simple_logistic");
    assert_eq!(rs.fitted_curves[0].data_set_index, 0);
    assert_eq!(rs.fitted_curves[1].equation_id, "ROC");
    assert_eq!(rs.fitted_curves[1].data_set_index, 0);
    assert_eq!(rs.fitted_curves[2].equation_id, "simple_logistic");
    assert_eq!(rs.fitted_curves[2].data_set_index, 1);
    assert_eq!(rs.fitted_curves[3].equation_id, "ROC");
    assert_eq!(rs.fitted_curves[3].data_set_index, 1);

    let roc0_pts: Vec<(f64, f64)> = rs.fitted_curves[1].points.iter().map(|p| (p.x, p.y)).collect();
    assert_close(trapezoidal_auc(&roc0_pts), rs.lookup_scalar("ROC_AUC", Some(0)).unwrap(), 1e-10, "SL ROC overlay AUC ds0");

    // AUD-13-06 (Logistic GLM): Emits an "ROC" FittedCurveOverlay matching DeLong ROC AUC.
    let mut mv = MultipleVariablesTable::new(2, xs.len()).unwrap();
    mv.variables[0].title = String::from("Outcome");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[0].cells = y0.iter().map(|&v| Cell::number(v)).collect();
    mv.variables[1].title = String::from("Dose");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[1].cells = xs.iter().map(|&v| Cell::number(v)).collect();
    let mv_sheet = DataSheet::new("mv-glm", "Logistic GLM", TableArchetype::MultipleVariables(mv)).unwrap();
    let glm_spec =
        GlmSpec { family: GlmFamily::Logistic, response_column: 0, predictor_columns: vec![1], reference_levels: Vec::new(), compute_vif: false };
    let glm_an = AnalysisSpec::new("res-glm", "mv-glm", AnalysisKind::GeneralizedLinearModel(glm_spec)).unwrap();
    let glm_res = run_analysis(&mv_sheet, &glm_an).unwrap();
    let glm_roc = glm_res.fitted_curves.iter().find(|c| c.equation_id == "ROC").expect("Logistic GLM ROC overlay");
    let glm_pts: Vec<(f64, f64)> = glm_roc.points.iter().map(|p| (p.x, p.y)).collect();
    let glm_auc = glm_res.scalars.iter().find(|s| s.key == "ROC AUC").unwrap().value;
    assert_close(trapezoidal_auc(&glm_pts), glm_auc, 1e-10, "Logistic GLM ROC overlay AUC");
}

// ----------------------------------------------------------------------------
// AUD-05-03, AUD-05-10, AUD-05-28, AUD-05-32, AUD-05-34: Regression & GLM
// ----------------------------------------------------------------------------

#[test]
fn aud_05_03_and_aud_05_34_passing_bablok_cusum_and_vertical_invariance() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let x: Vec<f64> = (1..=30).map(f64::from).collect();
    let y: Vec<f64> = x.iter().enumerate().map(|(i, &xi)| 1.2 * xi + if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
    let pb = passing_bablok_regression(&x, &y, conf).unwrap();
    let tol = 1e-12 * (1.0 + 40.0 + 40.0 * pb.slope.abs());
    let pos = pb.residuals.iter().filter(|&&r| r > tol).count();
    let neg = pb.residuals.iter().filter(|&&r| r < -tol).count();
    let expected_h_std = pb.cusum_h / ((pos + neg) as f64).sqrt();
    assert_close(pb.cusum_statistic, expected_h_std, 1e-12, "Passing-Bablok CUSUM H / sqrt(L + l)");

    // Sheet label must match H / sqrt(L + l).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, x.len()).unwrap();
    for (r, (&xv, &yv)) in x.iter().zip(y.iter()).enumerate() {
        xy.x_cells[r][0] = Cell::number(xv);
        xy.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let sh = DataSheet::new("pb-lbl", "PB", TableArchetype::Xy(xy)).unwrap();
    let sp = AnalysisSpec::new("res-pb", "pb-lbl", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 })).unwrap();
    let rs = run_analysis(&sh, &sp).unwrap();
    let sum_tab = rs.tabs.iter().find(|t| t.id == "pb-summary").unwrap();
    assert!(sum_tab.row_labels.iter().any(|l| l == "CUSUM standardized H / sqrt(L + l)"));

    // AUD-05-34: Swapping tied-X rows must not change k_offset.
    let pb1 = passing_bablok_regression(&[1.0, 2.0, 2.0, 3.0, 4.0], &[1.0, 1.0, 3.0, 3.0, 4.0], conf).unwrap();
    let pb2 = passing_bablok_regression(&[1.0, 2.0, 2.0, 3.0, 4.0], &[1.0, 3.0, 1.0, 3.0, 4.0], conf).unwrap();
    assert_eq!(pb1.k_offset, 0);
    assert_eq!(pb2.k_offset, 0);
    assert_eq!(pb1.valid_slopes, pb2.valid_slopes);
    assert_close(pb1.slope, pb2.slope, 1e-12, "tied-X swap slope");
}

#[test]
fn aud_05_10_multiple_linear_small_scale_adj_r2_and_zero_durbin_watson() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let x_raw: Vec<f64> = (1..=10).map(f64::from).collect();
    let y_unit = [1.2, 1.8, 3.5, 3.1, 5.4, 5.1, 7.6, 7.2, 9.5, 9.1];
    let y_tiny: Vec<f64> = y_unit.iter().map(|&v| v * 1e-16).collect();
    let x_mat = DenseMatrix::from_row_major(10, 2, x_raw.iter().flat_map(|&v| [1.0, v]).collect()).unwrap();
    let names = [String::from("Intercept"), String::from("X")];

    let r_unit = multiple_linear_regression(&x_mat, &y_unit, &names, true, false, conf).unwrap();
    let r_tiny = multiple_linear_regression(&x_mat, &y_tiny, &names, true, false, conf).unwrap();
    assert_close(r_tiny.r_squared, r_unit.r_squared, 1e-12, "tiny y R^2");
    assert_close(r_tiny.adjusted_r_squared, r_unit.adjusted_r_squared, 1e-12, "tiny y adj R^2");

    // Constant nonzero residuals in no-intercept OLS have exact Durbin-Watson = 0.0 (not 2.0).
    let x_noint = DenseMatrix::from_row_major(4, 1, vec![1.0, -1.0, 1.0, -1.0]).unwrap();
    let y_const = [1.0, 1.0, 1.0, 1.0];
    let r_dw = multiple_linear_regression(&x_noint, &y_const, &[String::from("X")], false, false, conf).unwrap();
    assert_close(r_dw.durbin_watson, 0.0, 1e-12, "exact DW = 0.0 on constant nonzero residuals");
}

#[test]
fn aud_05_28_and_aud_05_32_glm_rare_events_and_shifted_predictors() {
    let conf = ConfidenceLevel::DEFAULT_95;

    // AUD-05-28: Non-separated rare-event cohort (prevalence = 5 / 60_000 < 1e-4) must converge without separation.
    let n_rare = 60_000_usize;
    let mut x_rare = Vec::with_capacity(n_rare * 2);
    let mut y_rare = vec![0.0_f64; n_rare];
    for i in 0..n_rare {
        let xv = ((i % 20) as f64) - 9.5;
        x_rare.push(1.0);
        x_rare.push(xv);
    }
    // Overlapping event locations across negative and positive x values.
    for &idx in &[5_usize, 10_008, 20_012, 30_014, 40_016] {
        y_rare[idx] = 1.0;
    }
    let x_rare_mat = DenseMatrix::from_row_major(n_rare, 2, x_rare).unwrap();
    let rare_fit = multiple_logistic_regression(&x_rare_mat, &y_rare, &[String::from("Int"), String::from("X")], false, conf).unwrap();
    assert!(rare_fit.converged, "rare-event logistic fit must converge");
    assert!(!rare_fit.separation_detected, "rare-event logistic fit must not falsely trigger separation");

    // AUD-05-32: Shifted predictor x = 1_000_000..=1_000_010 in Logistic & Poisson GLMs must succeed identically to 0..=10.
    let x0: Vec<f64> = (0..=10).map(f64::from).collect();
    let x_shift: Vec<f64> = x0.iter().map(|&v| v + 1_000_000.0).collect();
    let y_bin = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let y_cnt = [1.0, 2.0, 2.0, 3.0, 5.0, 4.0, 7.0, 9.0, 11.0, 15.0, 20.0];
    let m0 = DenseMatrix::from_row_major(11, 2, x0.iter().flat_map(|&v| [1.0, v]).collect()).unwrap();
    let ms = DenseMatrix::from_row_major(11, 2, x_shift.iter().flat_map(|&v| [1.0, v]).collect()).unwrap();
    let names = [String::from("Int"), String::from("X")];

    let log0 = multiple_logistic_regression(&m0, &y_bin, &names, false, conf).unwrap();
    let logs = multiple_logistic_regression(&ms, &y_bin, &names, false, conf).unwrap();
    assert!(logs.converged);
    assert_close(logs.terms[1].beta, log0.terms[1].beta, 1e-7, "shifted logistic beta1");
    assert_close(logs.terms[1].se, log0.terms[1].se, 1e-7, "shifted logistic se1");

    let poi0 = multiple_poisson_regression(&m0, &y_cnt, &names, false, conf).unwrap();
    let pois = multiple_poisson_regression(&ms, &y_cnt, &names, false, conf).unwrap();
    assert!(pois.converged);
    assert_close(pois.terms[1].beta, poi0.terms[1].beta, 1e-7, "shifted poisson beta1");
    assert_close(pois.terms[1].se, poi0.terms[1].se, 1e-7, "shifted poisson se1");
}

// ----------------------------------------------------------------------------
// AUD-05-04, 05, 06, 16, 51, 52, 60: Categorical & Inter-Rater Agreement
// ----------------------------------------------------------------------------

#[test]
fn aud_05_04_fleiss_kappa_1979_null_variance_oracle() {
    let conf = ConfidenceLevel::DEFAULT_95;
    // K = 2, n = 100, m = 10, prevalences p = (0.85, 0.15):
    // Fleiss, Nee & Landis (1979): SE_0 = sqrt(2 / (100 * 10 * 9)) = 0.014907119849998598.
    let mut tbl2 = Vec::with_capacity(100);
    for i in 0..100 {
        if i < 80 {
            tbl2.push(vec![10, 0]);
        } else if i < 90 {
            tbl2.push(vec![5, 5]);
        } else {
            tbl2.push(vec![0, 10]);
        }
    }
    let r2 = fleiss_kappa(&tbl2, conf).unwrap();
    let expected_se0_k2 = (2.0_f64 / (100.0 * 10.0 * 9.0)).sqrt();
    let implied_se0_k2 = r2.kappa / r2.z_statistic;
    assert_close(implied_se0_k2, expected_se0_k2, 1e-12, "Fleiss 1979 K=2 SE_0");

    // K = 3, n = 100, m = 10, prevalences p = (0.7, 0.2, 0.1):
    let mut tbl3 = Vec::with_capacity(100);
    for i in 0..100 {
        if i < 65 {
            tbl3.push(vec![10, 0, 0]);
        } else if i < 75 {
            tbl3.push(vec![5, 5, 0]);
        } else if i < 90 {
            tbl3.push(vec![0, 10, 0]);
        } else {
            tbl3.push(vec![0, 0, 10]);
        }
    }
    let r3 = fleiss_kappa(&tbl3, conf).unwrap();
    let pe = 0.7_f64.powi(2) + 0.2_f64.powi(2) + 0.1_f64.powi(2);
    let pj3 = 0.7_f64.powi(3) + 0.2_f64.powi(3) + 0.1_f64.powi(3);
    let expected_se0_k3 = (2.0 * (pe + pe * pe - 2.0 * pj3) / (100.0 * 10.0 * 9.0 * (1.0 - pe).powi(2))).sqrt();
    assert_close(r3.kappa / r3.z_statistic, expected_se0_k3, 1e-12, "Fleiss 1979 K=3 SE_0");
}

#[test]
fn aud_05_05_mcnemar_zero_margin_and_asymmetric_3x3_and_ca_interior_gap() {
    // 1. Valid 2x2 paired table with an empty column [[0, 10], [0, 5]]: b = 10, c = 0.
    let sh_zm = make_contingency_sheet("mc-zm", &[&[0, 10], &[0, 5]]);
    let sp_mc = AnalysisSpec::new(
        "res-mc-zm",
        "mc-zm",
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::McNemarPaired, ..Default::default() }),
    )
    .unwrap();
    let res_zm = run_analysis(&sh_zm, &sp_mc).unwrap();
    let mc_p = res_zm.scalars.iter().find(|s| s.key == "McNemar exact P").unwrap().value;
    let mc_chi2 = res_zm.scalars.iter().find(|s| s.key == "McNemar chi-square").unwrap().value;
    assert_close(mc_p, 2.0 * 0.5_f64.powi(10), 1e-12, "McNemar zero-margin exact P");
    assert_close(mc_chi2, 8.1, 1e-12, "McNemar zero-margin Yates chi2");

    // 2. Asymmetric 3x3 table with empty row 1 & empty col 2 must NOT compress into a misaligned 2x2
    //    either in live McNemar sheet dispatch or in public `analyze_contingency`.
    let asym_3x3 = vec![vec![10_u64, 2, 0], vec![0, 0, 0], vec![3, 15, 0]];
    let sh_3x3 = make_contingency_sheet("mc-3x3", &[&[10, 2, 0], &[0, 0, 0], &[3, 15, 0]]);
    let sp_3x3 = AnalysisSpec::new(
        "res-mc-3x3",
        "mc-3x3",
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::McNemarPaired, ..Default::default() }),
    )
    .unwrap();
    let err = run_analysis(&sh_3x3, &sp_3x3).unwrap_err();
    assert!(matches!(err, StatsError::InvalidParameter { name: "mcnemar_dimensions", .. }));
    let res_3x3 = analyze_contingency(&asym_3x3, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_eq!(res_3x3.mcnemar_chi2, None, "reduced 3x3 must not emit misaligned McNemar chi2");
    assert_eq!(res_3x3.mcnemar_exact_p, None, "reduced 3x3 must not emit misaligned McNemar exact P");

    // 3. Cochran-Armitage preserves interior empty dose-level spacing (scores 1, 3, 4 vs contiguous 1, 2, 3)
    //    across `cochran_armitage_trend`, `analyze_contingency`, and live `run_analysis` (4x2 and 4x3 with empty col).
    let with_gap = vec![vec![20_u64, 0], vec![0, 0], vec![5, 15], vec![5, 15]];
    let contiguous = vec![vec![20_u64, 0], vec![5, 15], vec![5, 15]];
    let expected_z_gap = -5.175_491_695_067_655;
    let expected_z_contig = -4.743_416_490_252_569;
    let (z_gap, _) = cochran_armitage_trend(&with_gap).unwrap();
    let (z_contig, _) = cochran_armitage_trend(&contiguous).unwrap();
    assert_close(z_gap, expected_z_gap, 1e-12, "CA exact Z with gap [1, 3, 4]");
    assert_close(z_contig, expected_z_contig, 1e-12, "CA exact Z contiguous [1, 2, 3]");

    let ac_gap = analyze_contingency(&with_gap, false, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(ac_gap.cochran_armitage_z.unwrap(), expected_z_gap, 1e-12, "analyze_contingency CA gap Z");

    let extract_ca_z = |rs: &curvebook_doc::ResultSheet| -> f64 {
        let tab = rs.tabs.iter().find(|t| t.id == "contingency-main").expect("contingency-main tab");
        assert_eq!(tab.rows[0][1], ResultCell::Empty, "Cochran-Armitage Z must leave df Empty");
        match tab.rows[0][0] {
            ResultCell::Number { value, .. } => value,
            ref other => panic!("expected numeric Cochran-Armitage Z cell, got {other:?}"),
        }
    };

    let sh_ca_4x2 = make_contingency_sheet("ca-4x2", &[&[20, 0], &[0, 0], &[5, 15], &[5, 15]]);
    let sp_ca_4x2 = AnalysisSpec::new(
        "res-ca-4x2",
        "ca-4x2",
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CochranArmitageTrend, ..Default::default() }),
    )
    .unwrap();
    let res_ca_4x2 = run_analysis(&sh_ca_4x2, &sp_ca_4x2).unwrap();
    assert_close(extract_ca_z(&res_ca_4x2), expected_z_gap, 1e-12, "live sheet 4x2 CA gap Z");

    let sh_ca_4x3 = make_contingency_sheet("ca-4x3", &[&[20, 0, 0], &[0, 0, 0], &[5, 15, 0], &[5, 15, 0]]);
    let sp_ca_4x3 = AnalysisSpec::new(
        "res-ca-4x3",
        "ca-4x3",
        AnalysisKind::Contingency(ContingencySpec { method: ContingencyMethod::CochranArmitageTrend, ..Default::default() }),
    )
    .unwrap();
    let res_ca_4x3 = run_analysis(&sh_ca_4x3, &sp_ca_4x3).unwrap();
    assert_close(extract_ca_z(&res_ca_4x3), expected_z_gap, 1e-12, "live sheet 4x3 (empty col + interior empty row) CA gap Z");
}

#[test]
fn aud_05_06_barnard_csm_boundary_adaptive_oracle() {
    // Unbalanced 2x2 table [[2, 0], [19, 481]] (n1 = 2, n2 = 500, grid_cells = 1503 <= 1600):
    // True Barnard CSM P = 0.00085499 (coarse 200-point grid previously gave 0.00147111).
    let res = barnard_exact_2x2_with_ordering(2, 0, 19, 481, BarnardOrdering::Csm).unwrap();
    assert!(!res.used_asymptotic_fallback);
    assert_close(res.p_value, 0.000_854_99, 1e-5, "Barnard CSM exact P");
}

#[test]
fn aud_05_16_fisher_rxc_cell_cap_blocks_dos() {
    let n = 100_usize;
    let mut diag = vec![vec![0_u64; n]; n];
    for (i, row) in diag.iter_mut().enumerate() {
        row[i] = 1;
    }
    let err = fisher_exact_rxc(&diag).unwrap_err();
    assert!(matches!(err, StatsError::InvalidParameter { name: "fisher_exact_max_cells", .. }));
}

#[test]
fn aud_05_51_aud_05_52_aud_05_60_categorical_symmetry_infinity_and_nnt() {
    // AUD-05-51: Transpose symmetry on 2x5 vs 5x2 with 10% expected cells in [1, 5).
    let rows_2x5: [&[u64]; 2] = [&[5, 25, 25, 25, 25], &[4, 25, 25, 25, 25]];
    let rows_5x2: [&[u64]; 5] = [&[5, 4], &[25, 25], &[25, 25], &[25, 25], &[25, 25]];
    let sh_2x5 = make_contingency_sheet("ct-2x5", &rows_2x5);
    let sh_5x2 = make_contingency_sheet("ct-5x2", &rows_5x2);
    let sp_2x5 = AnalysisSpec::new("r-2x5", "ct-2x5", AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    let sp_5x2 = AnalysisSpec::new("r-5x2", "ct-5x2", AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    let r_2x5 = run_analysis(&sh_2x5, &sp_2x5).unwrap();
    let r_5x2 = run_analysis(&sh_5x2, &sp_5x2).unwrap();
    let st_2x5 = r_2x5.checklist.iter().find(|c| c.assumption == "Minimum expected cell count").unwrap().status;
    let st_5x2 = r_5x2.checklist.iter().find(|c| c.assumption == "Minimum expected cell count").unwrap().status;
    assert_eq!(st_2x5, ChecklistStatus::Pass);
    assert_eq!(st_5x2, ChecklistStatus::Pass);

    // AUD-05-52: Infinite 2x2 OR and RR render "∞" instead of ResultCell::Empty.
    let sh_inf = make_contingency_sheet("ct-inf", &[&[10, 0], &[0, 5]]);
    let sp_inf =
        AnalysisSpec::new("r-inf", "ct-inf", AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }))
            .unwrap();
    let r_inf = run_analysis(&sh_inf, &sp_inf).unwrap();
    let ratio_tab = r_inf.tabs.iter().find(|t| t.id == "contingency-ratios").unwrap();
    assert_eq!(ratio_tab.rows[0][0], ResultCell::Text(String::from("∞")));
    assert_eq!(ratio_tab.rows[1][0], ResultCell::Text(String::from("∞")));

    // AUD-05-60: Signed NNT uses 1e-15 threshold matching altman_nnt_from_rd.
    let r_harm = analyze_2x2_ratios(2, 8, 7, 3, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(r_harm.nnt.unwrap() < 0.0);
    assert_close(r_harm.nnt.unwrap().abs(), r_harm.nnt_value.unwrap(), 1e-12, "signed nnt magnitude");
}

// ----------------------------------------------------------------------------
// AUD-05-07, 29, 31, 33, 45, 53: Survival & Cox PH
// ----------------------------------------------------------------------------

#[test]
fn aud_05_07_harrell_c_index_tie_tolerance_symmetry() {
    // Strictly covariate-symmetric 2-covariate design: every (t, event, [a, b]) is paired with
    // (t, event, [b, a]), guaranteeing beta0 == beta1 > 0 to full precision.
    // Group A (x = [0.1, 0.2] / [0.2, 0.1], sum = 0.30000000000000004 in IEEE-754) and
    // Group B (x = [0.3, 0.0] / [0.0, 0.3], sum = 0.3) have mathematically equal linear predictors,
    // differing by ~2.78e-17 after standardization. Checking `>` before `1e-12` tie tolerance
    // previously inflated C-index from exact 220.5 / 354 = 0.622881355932 to 270.5 / 354 = 0.764124.
    let sym = vec![
        CoxSubject { time: 0.5, event: true, x: vec![2.0, 2.0] },
        CoxSubject { time: 0.6, event: true, x: vec![2.0, 2.0] },
        CoxSubject { time: 0.7, event: true, x: vec![2.0, 2.0] },
        CoxSubject { time: 11.5, event: false, x: vec![2.0, 2.0] },
        CoxSubject { time: 1.0, event: true, x: vec![0.1, 0.2] },
        CoxSubject { time: 1.0, event: true, x: vec![0.2, 0.1] },
        CoxSubject { time: 2.0, event: true, x: vec![0.1, 0.2] },
        CoxSubject { time: 2.0, event: true, x: vec![0.2, 0.1] },
        CoxSubject { time: 3.0, event: true, x: vec![0.1, 0.2] },
        CoxSubject { time: 3.0, event: true, x: vec![0.2, 0.1] },
        CoxSubject { time: 4.0, event: true, x: vec![0.1, 0.2] },
        CoxSubject { time: 4.0, event: true, x: vec![0.2, 0.1] },
        CoxSubject { time: 5.0, event: true, x: vec![0.1, 0.2] },
        CoxSubject { time: 5.0, event: true, x: vec![0.2, 0.1] },
        CoxSubject { time: 6.0, event: true, x: vec![0.3, 0.0] },
        CoxSubject { time: 6.0, event: true, x: vec![0.0, 0.3] },
        CoxSubject { time: 7.0, event: true, x: vec![0.3, 0.0] },
        CoxSubject { time: 7.0, event: true, x: vec![0.0, 0.3] },
        CoxSubject { time: 8.0, event: true, x: vec![0.3, 0.0] },
        CoxSubject { time: 8.0, event: true, x: vec![0.0, 0.3] },
        CoxSubject { time: 9.0, event: true, x: vec![0.3, 0.0] },
        CoxSubject { time: 9.0, event: true, x: vec![0.0, 0.3] },
        CoxSubject { time: 10.0, event: false, x: vec![0.3, 0.0] },
        CoxSubject { time: 10.0, event: false, x: vec![0.0, 0.3] },
        CoxSubject { time: 0.8, event: true, x: vec![0.0, 0.0] },
        CoxSubject { time: 11.0, event: false, x: vec![0.0, 0.0] },
        CoxSubject { time: 12.0, event: false, x: vec![0.0, 0.0] },
        CoxSubject { time: 13.0, event: false, x: vec![0.0, 0.0] },
    ];
    let res = cox_ph_regression(&sym, &[String::from("x1"), String::from("x2")], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).unwrap();
    assert!(res.converged, "symmetric Cox model must converge cleanly");
    assert!(res.covariates[0].beta > 0.1, "beta must be strictly positive");
    assert_close(res.covariates[0].beta, res.covariates[1].beta, 1e-12, "exact covariate symmetry beta0 == beta1");
    assert_close(res.concordance.unwrap(), 220.5 / 354.0, 1e-12, "exact tied Harrell C-index (not 270.5/354 = 0.7641)");

    let sym_swapped: Vec<CoxSubject> = sym.iter().map(|s| CoxSubject { time: s.time, event: s.event, x: vec![s.x[1], s.x[0]] }).collect();
    let res_sw =
        cox_ph_regression(&sym_swapped, &[String::from("x2"), String::from("x1")], CoxTieMethod::Breslow, ConfidenceLevel::DEFAULT_95).unwrap();
    assert_close(res.concordance.unwrap(), res_sw.concordance.unwrap(), 1e-12, "Harrell C-index tie symmetry");
}

#[test]
fn aud_05_29_cox_separation_respects_tie_method() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let subjects = vec![
        CoxSubject { time: 1.0, event: true, x: vec![2.0] },
        CoxSubject { time: 1.0, event: true, x: vec![1.0] },
        CoxSubject { time: 2.0, event: false, x: vec![1.0] },
        CoxSubject { time: 2.0, event: false, x: vec![0.0] },
        CoxSubject { time: 2.0, event: false, x: vec![0.0] },
    ];
    let names = [String::from("X")];

    // Under Breslow and Efron, tied events with distinct x have a unique finite interior MLE.
    let bres = cox_ph_regression(&subjects, &names, CoxTieMethod::Breslow, conf).unwrap();
    assert!(bres.converged, "Breslow tied-event MLE must converge");
    assert_close(bres.covariates[0].beta, 1.289_9, 5e-3, "Breslow beta");
    assert!(bres.covariates[0].se.is_finite() && bres.covariates[0].se > 0.0);

    let efron = cox_ph_regression(&subjects, &names, CoxTieMethod::Efron, conf).unwrap();
    assert!(efron.converged, "Efron tied-event MLE must converge");
    assert_close(efron.covariates[0].beta, 1.490_4, 5e-3, "Efron beta");
    assert!(efron.covariates[0].se.is_finite() && efron.covariates[0].se > 0.0);

    // Under Exact discrete ties, {2, 1} strictly dominates all other 2-subsets -> monotone separation.
    let exact = cox_ph_regression(&subjects, &names, CoxTieMethod::Exact, conf).unwrap();
    assert!(!exact.converged, "Exact tied-event model has monotone separation");
}

#[test]
fn aud_05_31_cox_exact_ties_budget_blocks_dos() {
    let p = 20_usize;
    let n = 2_000_usize;
    let subjects: Vec<CoxSubject> = (0..n)
        .map(|i| CoxSubject { time: if i < 1_000 { 1.0 } else { 2.0 }, event: i < 1_000, x: (0..p).map(|j| ((i + j) % 7) as f64).collect() })
        .collect();
    let names: Vec<String> = (0..p).map(|j| format!("X{j}")).collect();
    let err = cox_ph_regression(&subjects, &names, CoxTieMethod::Exact, ConfidenceLevel::DEFAULT_95).unwrap_err();
    assert!(matches!(err, StatsError::InvalidParameter { name: "cox_exact_ties_budget", .. }));
}

#[test]
fn aud_05_33_cox_standardizes_multiscale_orthogonal_covariates() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let base_data = [
        (1.0, true, 40.0, 1.0),
        (2.0, true, 55.0, 4.0),
        (3.0, true, 45.0, 2.0),
        (4.0, true, 65.0, 3.0),
        (5.0, false, 50.0, 5.0),
        (6.0, true, 70.0, 2.5),
        (7.0, false, 35.0, 4.5),
        (8.0, true, 60.0, 1.5),
        (9.0, false, 42.0, 3.5),
        (10.0, false, 48.0, 6.0),
    ];
    let scale = 1e-8_f64;
    let sub_unit: Vec<CoxSubject> = base_data.iter().map(|&(t, e, a, c)| CoxSubject { time: t, event: e, x: vec![a, c] }).collect();
    let sub_molar: Vec<CoxSubject> = base_data.iter().map(|&(t, e, a, c)| CoxSubject { time: t, event: e, x: vec![a, c * scale] }).collect();
    let names = [String::from("Age"), String::from("Conc")];

    let r_unit = cox_ph_regression(&sub_unit, &names, CoxTieMethod::Efron, conf).unwrap();
    let r_molar = cox_ph_regression(&sub_molar, &names, CoxTieMethod::Efron, conf).unwrap();
    assert!(r_molar.converged, "multi-scale Cox PH must converge");
    assert_close(r_molar.covariates[0].beta, r_unit.covariates[0].beta, 1e-8, "Age beta");
    assert_close(r_molar.covariates[0].se, r_unit.covariates[0].se, 1e-8, "Age SE");
    assert_rel_close(r_molar.covariates[1].beta * scale, r_unit.covariates[1].beta, 1e-8, "Molar beta");
    assert_rel_close(r_molar.covariates[1].se * scale, r_unit.covariates[1].se, 1e-8, "Molar SE");
}

#[test]
fn aud_05_45_survival_signed_zero_normalization() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let km = kaplan_meier_curve(
        "zero",
        &[
            SurvivalObservation { row: 0, time: -0.0, status: SurvivalStatus::Censored },
            SurvivalObservation { row: 1, time: 0.0, status: SurvivalStatus::Event },
            SurvivalObservation { row: 2, time: 1.0, status: SurvivalStatus::Event },
        ],
        true,
        conf,
    )
    .unwrap();
    assert_eq!(km.steps.len(), 2, "KM must merge -0.0 and +0.0 into a single t = 0 step");
    assert_eq!(km.steps[0].n_events, 1);
    assert_eq!(km.steps[0].n_censored, 1);

    let sub_neg = vec![
        CoxSubject { time: -0.0, event: false, x: vec![0.0] },
        CoxSubject { time: 0.0, event: true, x: vec![1.0] },
        CoxSubject { time: 1.0, event: true, x: vec![0.0] },
        CoxSubject { time: 2.0, event: false, x: vec![1.0] },
    ];
    let sub_pos = vec![
        CoxSubject { time: 0.0, event: false, x: vec![0.0] },
        CoxSubject { time: 0.0, event: true, x: vec![1.0] },
        CoxSubject { time: 1.0, event: true, x: vec![0.0] },
        CoxSubject { time: 2.0, event: false, x: vec![1.0] },
    ];
    let names = [String::from("X")];
    let r_neg = cox_ph_regression(&sub_neg, &names, CoxTieMethod::Breslow, conf).unwrap();
    let r_pos = cox_ph_regression(&sub_pos, &names, CoxTieMethod::Breslow, conf).unwrap();
    assert_close(r_neg.log_lik_null, r_pos.log_lik_null, 1e-12, "Cox -0.0 vs +0.0 ll_null");
    assert_close(r_neg.log_lik_model, r_pos.log_lik_model, 1e-12, "Cox -0.0 vs +0.0 ll_model");
}

#[test]
fn aud_05_53_cox_design_validation_and_single_level_categorical() {
    let mut mv = MultipleVariablesTable::new(4, 6).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[0].cells = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0].into_iter().map(Cell::number).collect();
    mv.variables[1].title = String::from("Status");
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[1].cells = [1.0, 0.0, 1.0, 1.0, 0.0, 1.0].into_iter().map(Cell::number).collect();
    mv.variables[2].title = String::from("Age");
    mv.variables[2].role = VariableRole::Continuous;
    mv.variables[2].cells = [40.0, 50.0, 60.0, 45.0, 55.0, 65.0].into_iter().map(Cell::number).collect();
    mv.variables[3].title = String::from("Sex");
    mv.variables[3].role = VariableRole::Categorical;
    mv.variables[3].cells = vec![Cell::text("F"); 6];
    let sheet = DataSheet::new("mv-cox", "Cox", TableArchetype::MultipleVariables(mv)).unwrap();

    // time_column == status_column
    let sp_same = AnalysisSpec::new(
        "c1",
        "mv-cox",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 0,
            predictor_columns: vec![2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &sp_same), Err(StatsError::InvalidParameter { name: "status_column", .. })));

    // predictor overlaps time_column
    let sp_overlap = AnalysisSpec::new(
        "c2",
        "mv-cox",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![0, 2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &sp_overlap), Err(StatsError::InvalidParameter { name: "predictor_columns", .. })));

    // duplicate predictor
    let sp_dup = AnalysisSpec::new(
        "c3",
        "mv-cox",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 2],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &sp_dup), Err(StatsError::InvalidParameter { name: "predictor_columns", .. })));

    // single-level categorical predictor alongside numeric predictor must not silently vanish
    let sp_cat = AnalysisSpec::new(
        "c4",
        "mv-cox",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    assert!(matches!(run_analysis(&sheet, &sp_cat), Err(StatsError::ZeroVariance { .. })));
}
