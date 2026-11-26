//! Independent `D1` oracle tests for R18.6 Standalone XY Simple Logistic Regression.
//!
//! Reference values generated with R `4.5.2` (`stats::glm(..., family = binomial(link = "logit"))`,
//! `stats::confint` / `MASS::confint.glm`, `stats::predict.glm`) using
//! `crates/stats/tests/reference/r18_6_simple_logistic.R`.
//!
//! Literature references:
//! - Hosmer, D. W., Lemeshow, S., & Sturdivant, R. X. (2013). *Applied Logistic Regression* (3rd ed.). Wiley.
//! - Tjur, T. (2009). Coefficients of determination in logistic regression models. *The American Statistician*, 63(4), 366-372.
//! - Albert, A., & Anderson, J. A. (1984). On the existence of maximum likelihood estimates in logistic regression models. *Biometrika*, 71(1), 1-10.
//! - Heinze, G., & Schemper, M. (2002). A solution to the problem of separation in logistic regression. *Statistics in Medicine*, 21(16), 2409-2419.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellValue, ChecklistStatus, ColumnDesignHint, ColumnTable, DataSheet, SimpleLogisticSpec, SubcolumnMode,
    TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::simple_logistic::{SimpleLogisticSeparation, run_simple_logistic_analysis, simple_logistic_regression};
use curvebook_stats::{StatsError, run_analysis};
use curvebook_testkit::assert_rel_close;

use crate::common::{run_kind, xy_multi_sheet};

fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{label}: actual={actual:.15}, expected={expected:.15}, diff={:.3e} > tol={tol:.3e}",
        (actual - expected).abs()
    );
}

#[test]
fn d1_oracle_dose_response_positive_slope_matches_r_4_5_2() {
    let x = [0.5, 0.8, 1.0, 1.2, 1.5, 1.8, 2.0, 2.2, 2.5, 2.8, 3.0, 3.2, 3.5, 3.8, 4.0, 4.2, 4.5, 4.8, 5.0, 5.2, 5.5, 5.8, 6.0, 6.5];
    let y = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let unknowns = [(24, 1.0), (25, 3.0), (26, 5.0)];

    let res = simple_logistic_regression(&x, &y, &unknowns, 0.5, ConfidenceLevel::DEFAULT_95).expect("dose_response_pos should converge");

    assert!(res.converged);
    assert_eq!(res.separation, SimpleLogisticSeparation::None);
    assert_eq!(res.n, 24);
    assert_eq!(res.n_events, 13);
    assert_eq!(res.n_non_events, 11);

    // Exact MLE, SE, Wald Z, Wald P against R 4.5.2 stats::glm (epsilon = 1e-14)
    assert_close(res.beta0, -3.447980073807965, 1e-10, "beta0");
    assert_close(res.se_beta0, 1.453431290872538, 1e-10, "se_beta0");
    assert_close(res.wald_z_beta0, -2.372303455595786, 1e-10, "z0");
    assert_close(res.wald_p_beta0, 1.767756541336416e-02, 1e-10, "p0");

    assert_close(res.beta1, 1.109138526061426, 1e-10, "beta1");
    assert_close(res.se_beta1, 0.425163358030098, 1e-10, "se_beta1");
    assert_close(res.wald_z_beta1, 2.608734984125583, 1e-10, "z1");
    assert_close(res.wald_p_beta1, 9.087_759_276_781_47e-3, 1e-10, "p1");

    // Asymmetrical profile-likelihood CIs against R 4.5.2 MASS::confint.glm
    // (MASS::confint.glm interpolates a coarse profile spline; tolerance 1e-3 covers spline approx vs exact root)
    assert_close(res.profile_ci_beta0.lower.expect("ci_b0 lo"), -6.994990996045427, 1e-3, "ci_beta0.lower");
    assert_close(res.profile_ci_beta0.upper.expect("ci_b0 hi"), -1.041053056993924, 1e-3, "ci_beta0.upper");
    assert_close(res.profile_ci_beta1.lower.expect("ci_b1 lo"), 0.418665768698604, 1e-3, "ci_beta1.lower");
    assert_close(res.profile_ci_beta1.upper.expect("ci_b1 hi"), 2.161544449532241, 1e-3, "ci_beta1.upper");

    // Propagated Odds Ratios & Profile CIs
    assert_close(res.odds_ratio_beta0, 0.031809825026813, 1e-10, "or0");
    assert_close(res.odds_ratio_beta1, 3.031745499390664, 1e-10, "or1");
    assert_close(res.odds_ratio_ci_beta0.lower.expect("or0 lo"), 0.000916461044665, 5e-5, "ci_or0.lower");
    assert_close(res.odds_ratio_ci_beta0.upper.expect("or0 hi"), 0.353082669943214, 5e-4, "ci_or0.upper");
    assert_close(res.odds_ratio_ci_beta1.lower.expect("or1 lo"), 1.519932260946883, 1e-3, "ci_or1.lower");
    assert_close(res.odds_ratio_ci_beta1.upper.expect("or1 hi"), 8.684540140028151, 5e-3, "ci_or1.upper");

    // X at 50% and exact profile-likelihood CI against R uniroot
    assert_close(res.x50.expect("x50"), 3.108701025877997, 1e-10, "x50");
    assert_close(res.x50_profile_ci.lower.expect("x50 lo"), 1.934827225267252, 1e-8, "ci_x50.lower");
    assert_close(res.x50_profile_ci.upper.expect("x50 hi"), 4.182750886114867, 1e-8, "ci_x50.upper");

    // Deviance, LR test, AIC, AICc, Tjur R^2, Cox-Snell R^2, ROC AUC
    assert_close(res.deviance_null, 33.104_204_561_137_28, 1e-10, "d0");
    assert_close(res.deviance_model, 20.894633181271125, 1e-10, "d1");
    assert_close(res.lr_chi2, 12.209571379866155, 1e-10, "g2");
    assert_close(res.lr_p, 4.754495638303053e-04, 1e-10, "lr_p");
    assert_close(res.aic, 24.894633181271125, 1e-10, "aic");
    assert_close(res.aicc, 25.466061752699698, 1e-10, "aicc");
    assert_close(res.tjur_r2, 0.430698511591905, 1e-10, "tjur_r2");
    assert_close(res.cox_snell_r2, 0.398742594517831, 1e-10, "cox_snell_r2");
    let roc = res.roc.as_ref().expect("ROC result present");
    assert_close(roc.auc, 0.881118881118881, 1e-10, "auc");
    assert!(roc.se.is_finite() && roc.se > 0.0);

    // Out-of-sample predictions for (X_new, blank Y) rows
    let unk_rows: Vec<_> = res.row_predictions.iter().filter(|r| r.is_unknown_row).collect();
    assert_eq!(unk_rows.len(), 3);
    assert_close(unk_rows[0].predicted_prob, 0.087956801987462, 1e-10, "pred[1.0]");
    assert_close(unk_rows[0].prob_ci.0, 0.011669270373778, 1e-10, "pred[1.0].lo");
    assert_close(unk_rows[0].prob_ci.1, 0.440625316835918, 1e-10, "pred[1.0].hi");
    assert_close(unk_rows[1].predicted_prob, 0.469895333540912, 1e-10, "pred[3.0]");
    assert_close(unk_rows[1].prob_ci.0, 0.231141225245603, 1e-10, "pred[3.0].lo");
    assert_close(unk_rows[1].prob_ci.1, 0.723271827415206, 1e-10, "pred[3.0].hi");
    assert_close(unk_rows[2].predicted_prob, 0.890680652529522, 1e-10, "pred[5.0]");
    assert_close(unk_rows[2].prob_ci.0, 0.557618222435619, 1e-10, "pred[5.0].lo");
    assert_close(unk_rows[2].prob_ci.1, 0.981365382330917, 1e-10, "pred[5.0].hi");
}

#[test]
fn d1_oracle_negative_slope_biomarker_matches_r_4_5_2() {
    let x = [10.0, 12.0, 14.0, 15.0, 17.0, 19.0, 21.0, 22.0, 24.0, 26.0, 28.0, 30.0, 32.0, 35.0, 37.0, 40.0, 42.0, 45.0];
    let y = [1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let res = simple_logistic_regression(&x, &y, &[(18, 25.0)], 0.5, ConfidenceLevel::DEFAULT_95).expect("biomarker_neg should converge");

    assert_close(res.beta0, 3.794019767207117, 1e-9, "beta0");
    assert_close(res.se_beta0, 1.848958387507453, 1e-9, "se_beta0");
    assert_close(res.beta1, -0.147449874060621, 1e-9, "beta1");
    assert_close(res.se_beta1, 0.069461441137133, 1e-9, "se_beta1");
    assert_close(res.x50.expect("x50"), 25.730912226125632, 1e-10, "x50");
    assert_close(res.x50_profile_ci.lower.expect("x50 lo"), 14.976427317255457, 1e-7, "ci_x50.lower");
    assert_close(res.x50_profile_ci.upper.expect("x50 hi"), 37.005_024_311_015_49, 1e-7, "ci_x50.upper");
    assert_close(res.deviance_null, 24.953_298_500_158_03, 1e-10, "d0");
    assert_close(res.deviance_model, 18.056008785946325, 1e-10, "d1");
    assert_close(res.lr_chi2, 6.897289714211706, 1e-10, "g2");
    assert_close(res.lr_p, 8.632652366618714e-03, 1e-10, "lr_p");
    assert_close(res.aicc, 22.856008785946326, 1e-10, "aicc");
    assert_close(res.tjur_r2, 0.335577764146870, 1e-10, "tjur_r2");
    assert_close(res.cox_snell_r2, 0.318311698749137, 1e-10, "cox_snell_r2");
    assert_close(res.roc.as_ref().expect("roc").auc, 0.839506172839506, 1e-10, "auc");

    let pred = res.row_predictions.iter().find(|r| r.is_unknown_row).expect("unknown row");
    assert_close(pred.predicted_prob, 0.526917180374498, 1e-10, "pred[25]");
    assert_close(pred.prob_ci.0, 0.262877912413833, 1e-10, "pred[25].lo");
    assert_close(pred.prob_ci.1, 0.776712423925928, 1e-10, "pred[25].hi");
}

#[test]
fn d1_oracle_weak_nonsignificant_has_open_x50_profile_ci() {
    let x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
    let y = [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let res = simple_logistic_regression(&x, &y, &[], 0.5, ConfidenceLevel::DEFAULT_95).expect("weak_nonsig should converge");

    assert_close(res.beta0, -0.715584998781728, 1e-10, "beta0");
    assert_close(res.beta1, 0.084186470444909, 1e-10, "beta1");
    assert_close(res.lr_chi2, 0.582813498823686, 1e-10, "g2");
    assert_close(res.x50.expect("x50"), 8.5, 1e-10, "x50");
    // Because G^2 = 0.583 < chi^2_{1,0.95} = 3.841, X50 profile CI must be unbounded (open).
    assert!(res.x50_profile_ci.lower.is_none());
    assert!(res.x50_profile_ci.upper.is_none());
    // beta0 and beta1 profile CIs are still finite and match R confint(fit).
    assert_close(res.profile_ci_beta0.lower.expect("b0 lo"), -3.045949310327833, 5e-4, "ci_beta0.lower");
    assert_close(res.profile_ci_beta0.upper.expect("b0 hi"), 1.373626044567996, 5e-4, "ci_beta0.upper");
    assert_close(res.profile_ci_beta1.lower.expect("b1 lo"), -0.131122335889005, 5e-4, "ci_beta1.lower");
    assert_close(res.profile_ci_beta1.upper.expect("b1 hi"), 0.324609582887144, 5e-4, "ci_beta1.upper");
}

#[test]
fn detects_complete_quasi_separation_and_collinearity_without_diverging() {
    // Complete separation: Y=0 for X in {1,2,3}, Y=1 for X in {4,5,6}
    let sep = simple_logistic_regression(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], &[], 0.5, ConfidenceLevel::DEFAULT_95)
        .expect("complete separation returns diagnostic result");
    assert!(!sep.converged);
    assert_eq!(sep.separation, SimpleLogisticSeparation::CompleteSeparation);
    assert!(sep.beta1.is_nan());

    // Quasi-complete separation: boundary tie at X = 3.0 with both Y=0 and Y=1
    let quasi = simple_logistic_regression(&[1.0, 2.0, 3.0, 3.0, 4.0, 5.0], &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], &[], 0.5, ConfidenceLevel::DEFAULT_95)
        .expect("quasi separation returns diagnostic result");
    assert!(!quasi.converged);
    assert_eq!(quasi.separation, SimpleLogisticSeparation::QuasiCompleteSeparation);
    assert!(quasi.beta1.is_nan());

    // Collinearity: constant X
    let col = simple_logistic_regression(&[5.0, 5.0, 5.0, 5.0], &[0.0, 0.0, 1.0, 1.0], &[], 0.5, ConfidenceLevel::DEFAULT_95)
        .expect("collinear X returns diagnostic result");
    assert!(!col.converged);
    assert_eq!(col.separation, SimpleLogisticSeparation::Collinear);
    assert!(col.beta1.is_nan());
}

#[test]
fn rejects_non_binary_y_and_incompatible_tables() {
    let err = simple_logistic_regression(&[1.0, 2.0, 3.0, 4.0], &[0.0, 0.5, 1.0, 1.0], &[], 0.5, ConfidenceLevel::DEFAULT_95)
        .expect_err("non-binary Y = 0.5 must fail");
    assert!(matches!(err, StatsError::InvalidParameter { .. }));

    // Column table rejected
    let col_table = ColumnTable::new(ColumnDesignHint::default(), 2, 4).expect("col table");
    let col_sheet = DataSheet::new("col-1", "Columns", TableArchetype::Column(col_table)).expect("sheet");
    let spec = AnalysisSpec::new("res-1", "col-1", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec");
    let err = run_analysis(&col_sheet, &spec).expect_err("column table must be rejected");
    assert!(matches!(err, StatsError::IncompatibleTable { actual: TableKind::Column, .. }));

    // Multi-subcolumn XY table rejected
    let xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 6).expect("xy rep");
    let xy_rep_sheet = DataSheet::new("xy-rep", "Replicates", TableArchetype::Xy(xy_rep)).expect("sheet");
    let spec_rep = AnalysisSpec::new("res-2", "xy-rep", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec");
    let err = run_simple_logistic_analysis(&xy_rep_sheet, &spec_rep, &SimpleLogisticSpec::default())
        .expect_err("multi-subcolumn XY table must be rejected");
    assert!(matches!(err, StatsError::InvalidParameter { .. }));
}

#[test]
fn result_sheet_includes_tabs_overlay_unknown_predictions_and_separation_checklist() {
    let x = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 2.0, 4.0];
    let y_cells = [
        Cell::number(0.0),
        Cell::number(0.0),
        Cell::number(0.0),
        Cell::number(1.0),
        Cell::number(0.0),
        Cell::number(1.0),
        Cell::number(0.0),
        Cell::number(1.0),
        Cell::number(1.0),
        Cell::number(1.0),
        Cell::empty(),
        Cell::empty(),
    ];
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, x.len()).expect("xy");
    for (r, (&xv, yc)) in x.iter().zip(y_cells).enumerate() {
        xy.x_cells[r][0] = Cell::number(xv);
        xy.data_sets[0].cells[r][0] = yc;
    }
    let sheet = DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy)).expect("sheet");
    let sl_spec = SimpleLogisticSpec {
        emit_row_classification: true,
        confidence_bands: true,
        classification_cutoff: 0.5,
        plot_x_min: Some(0.0),
        plot_x_max: Some(6.0),
        plot_segments: Some(60),
    };
    let spec = AnalysisSpec::new("res-sl", "xy-1", AnalysisKind::SimpleLogisticRegression(sl_spec)).expect("spec");
    let res = run_analysis(&sheet, &spec).expect("run_analysis");

    assert_eq!(res.tabs.len(), 4);
    assert_eq!(res.tabs[0].title, "Parameter Estimates");
    assert_eq!(res.tabs[1].title, "Goodness of Fit");
    assert_eq!(res.tabs[2].title, "ROC Curve");
    assert_eq!(res.tabs[3].title, "Row Classification");
    assert_eq!(res.tabs[3].rows.len(), 12);
    assert_eq!(res.fitted_curves.len(), 2);
    let curve = &res.fitted_curves[0];
    assert!(curve.converged);
    assert_eq!(curve.equation_id, "simple_logistic");
    assert_eq!(curve.points.len(), 61);
    assert_close(curve.points[0].x, 0.0, 1e-12, "plot_x_min");
    assert_close(curve.points[60].x, 6.0, 1e-12, "plot_x_max");
    assert!(curve.points[30].ci_lower.is_some() && curve.points[30].ci_upper.is_some());
    assert_eq!(res.fitted_curves[1].equation_id, "ROC");
    assert!(!res.fitted_curves[1].points.is_empty());

    // Verify optional suppression of Row Classification tab
    let sl_no_class = SimpleLogisticSpec { emit_row_classification: false, ..sl_spec };
    let spec_no_class = AnalysisSpec::new("res-sl2", "xy-1", AnalysisKind::SimpleLogisticRegression(sl_no_class)).expect("spec");
    let res_no_class = run_analysis(&sheet, &spec_no_class).expect("run_analysis");
    assert_eq!(res_no_class.tabs.len(), 3);
    assert!(res_no_class.tabs.iter().all(|t| t.title != "Row Classification"));

    // Verify separation checklist emits Fail status
    let mut xy_sep = XyTable::new(SubcolumnMode::Single, 1, 6).expect("xy sep");
    for r in 0..6 {
        xy_sep.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy_sep.data_sets[0].cells[r][0] = Cell::number(if r < 3 { 0.0 } else { 1.0 });
    }
    let sep_sheet = DataSheet::new("xy-sep", "Separated", TableArchetype::Xy(xy_sep)).expect("sheet");
    let sep_spec = AnalysisSpec::new("res-sep", "xy-sep", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).expect("spec");
    let sep_res = run_analysis(&sep_sheet, &sep_spec).expect("separated analysis succeeds with diagnostic");
    assert!(sep_res.checklist.iter().any(|c| c.status == ChecklistStatus::Violated));
    assert_eq!(sep_res.fitted_curves.len(), 1);
    assert!(!sep_res.fitted_curves[0].converged);
}

#[test]
fn simple_logistic_aud05_profile_ci_scales_replicates_guards_and_roc_overlays() {
    let conf = ConfidenceLevel::DEFAULT_95;
    let y11 = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let x_base: Vec<f64> = (0..=10).map(f64::from).collect();
    let x_age: Vec<f64> = (50..=60).map(f64::from).collect();
    let x_year: Vec<f64> = (2010..=2020).map(f64::from).collect();
    let x_steep: Vec<f64> = (0..=10).map(|i| (i as f64) * 1e-3).collect();

    // AUD-05-01 (Venzon & Moolgavkar 1988): Shift/scale invariance of profile-likelihood CIs & X50.
    let (rb, ra, ry, rs) = (
        simple_logistic_regression(&x_base, &y11, &[(11, 5.0)], 0.5, conf).unwrap(),
        simple_logistic_regression(&x_age, &y11, &[], 0.5, conf).unwrap(),
        simple_logistic_regression(&x_year, &y11, &[], 0.5, conf).unwrap(),
        simple_logistic_regression(&x_steep, &y11, &[], 0.5, conf).unwrap(),
    );
    assert_close(rb.profile_ci_beta1.lower.unwrap(), 0.126_317_167_5, 1e-6, "base ci_b1.lo");
    assert_close(rb.profile_ci_beta1.upper.unwrap(), 1.816_098_763_8, 1e-6, "base ci_b1.hi");
    for (r, lbl) in [(&ra, "age"), (&ry, "year")] {
        assert_close(r.profile_ci_beta1.lower.unwrap(), rb.profile_ci_beta1.lower.unwrap(), 1e-8, lbl);
        assert_close(r.profile_ci_beta1.upper.unwrap(), rb.profile_ci_beta1.upper.unwrap(), 1e-8, lbl);
    }
    assert_close(ry.odds_ratio_ci_beta1.lower.unwrap(), rb.odds_ratio_ci_beta1.lower.unwrap(), 1e-7, "year or1.lo");
    assert_close(ry.odds_ratio_ci_beta1.upper.unwrap(), rb.odds_ratio_ci_beta1.upper.unwrap(), 1e-7, "year or1.hi");
    assert_close(ry.profile_ci_beta0.lower.unwrap(), -3_660.348_752, 1e-3, "year ci_b0.lo");
    assert_close(ry.profile_ci_beta0.upper.unwrap(), -254.717_654, 1e-3, "year ci_b0.hi");
    assert_close(ry.x50.unwrap(), rb.x50.unwrap() + 2010.0, 1e-8, "year x50");
    assert_close(ry.x50_profile_ci.lower.unwrap(), rb.x50_profile_ci.lower.unwrap() + 2010.0, 1e-6, "year x50.lo");
    assert_close(ry.x50_profile_ci.upper.unwrap(), rb.x50_profile_ci.upper.unwrap() + 2010.0, 1e-6, "year x50.hi");
    assert_close(rs.x50.unwrap() * 1e3, rb.x50.unwrap(), 1e-8, "steep x50");
    assert_close(rs.x50_profile_ci.lower.unwrap() * 1e3, rb.x50_profile_ci.lower.unwrap(), 1e-6, "steep x50.lo");
    assert_close(rs.x50_profile_ci.upper.unwrap() * 1e3, rb.x50_profile_ci.upper.unwrap(), 1e-6, "steep x50.hi");

    // AUD-05-02: XY replicate-X averaging & pre-summarized negative-SD rejection.
    let xm = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let y10 = [0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
    let mut xy_rep = XyTable::new(SubcolumnMode::Single, 1, 10).unwrap();
    xy_rep.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) };
    xy_rep.x_cells = xm.iter().map(|&v| vec![Cell::number(v - 1.0), Cell::number(v), Cell::number(v + 1.0)]).collect();
    for (r, &yv) in y10.iter().enumerate() {
        xy_rep.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    let res_rep =
        run_kind(&DataSheet::new("xy-xr", "XR", TableArchetype::Xy(xy_rep)).unwrap(), AnalysisKind::SimpleLogisticRegression(Default::default()))
            .unwrap();
    let o10 = simple_logistic_regression(&xm, &y10, &[], 0.5, conf).unwrap();
    assert_close(res_rep.lookup_scalar("Beta0", Some(0)).unwrap(), o10.beta0, 1e-10, "rep X beta0");
    assert_close(res_rep.lookup_scalar("Beta1", Some(0)).unwrap(), o10.beta1, 1e-10, "rep X beta1");
    let mut xy_bad = XyTable::new(SubcolumnMode::Single, 1, 10).unwrap();
    xy_bad.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN };
    xy_bad.x_cells =
        xm.iter().enumerate().map(|(r, &v)| vec![Cell::number(v), Cell::number(if r == 2 { -2.0 } else { 0.5 }), Cell::number(3.0)]).collect();
    for (r, &yv) in y10.iter().enumerate() {
        xy_bad.data_sets[0].cells[r][0] = Cell::number(yv);
    }
    assert!(
        run_kind(&DataSheet::new("xy-xb", "XB", TableArchetype::Xy(xy_bad)).unwrap(), AnalysisKind::SimpleLogisticRegression(Default::default()))
            .is_err()
    );

    // AUD-05-25: Extreme X scales (1e-13 sub-picomolar & 1e16 huge) preserve beta1 & finite X50.
    let (x7, y7) = ([1.0, 2.0, 3.0, 4.0, 5.0, 2.0, 4.0], [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0]);
    let r_u7 = simple_logistic_regression(&x7, &y7, &[], 0.5, conf).unwrap();
    let r_t7 = simple_logistic_regression(&x7.map(|v| v * 1e-13), &y7, &[], 0.5, conf).unwrap();
    assert!(r_t7.converged && r_t7.separation == SimpleLogisticSeparation::None);
    assert_rel_close(r_t7.beta1 * 1e-13, r_u7.beta1, 1e-9, "sub-picomolar beta1");
    assert_rel_close(r_t7.x50.unwrap() * 1e13, r_u7.x50.unwrap(), 1e-9, "sub-picomolar x50");
    let r_h10 = simple_logistic_regression(&xm.map(|v| v * 1e16), &y10, &[], 0.5, conf).unwrap();
    assert!(r_h10.converged);
    assert_rel_close(r_h10.x50.unwrap(), o10.x50.unwrap() * 1e16, 1e-9, "huge-X x50");

    // AUD-05-26: Centered prediction CI & sigmoid overlay bands stay cancellation-free at 1e8 / 1.7e9 offsets.
    let base_mid = rb.row_predictions.iter().find(|r| r.is_unknown_row).unwrap();
    for off in [1.0e8_f64, 1.7e9] {
        let xo: Vec<f64> = x_base.iter().map(|&v| v + off).collect();
        let ro = simple_logistic_regression(&xo, &y11, &[(11, 5.0 + off)], 0.5, conf).unwrap();
        assert!(ro.converged);
        for (pb, po) in rb.row_predictions.iter().zip(&ro.row_predictions) {
            assert_close(po.predicted_prob, pb.predicted_prob, 1e-9, "off pred p");
            assert_close(po.prob_ci.0, pb.prob_ci.0, 1e-8, "off pred lo");
            assert_close(po.prob_ci.1, pb.prob_ci.1, 1e-8, "off pred hi");
        }
        let rs_ov = run_kind(
            &xy_multi_sheet("xy-ts", &xo, &[("Y", &y11)]),
            AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec { confidence_bands: true, plot_segments: Some(10), ..Default::default() }),
        )
        .unwrap();
        let mid = &rs_ov.fitted_curves.iter().find(|c| c.equation_id == "simple_logistic").unwrap().points[5];
        assert_close(mid.y, base_mid.predicted_prob, 1e-9, "overlay mid p");
        assert_close(mid.ci_lower.unwrap(), base_mid.prob_ci.0, 1e-8, "overlay mid lo");
        assert_close(mid.ci_upper.unwrap(), base_mid.prob_ci.1, 1e-8, "overlay mid hi");
    }

    // AUD-05-27: DimensionMismatch & non-finite Y rejection.
    assert!(matches!(
        simple_logistic_regression(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.0, 1.0, 1.0], &[], 0.5, conf),
        Err(StatsError::DimensionMismatch { expected: 5, actual: 3 })
    ));
    for bad_y in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
        for (r, yv) in [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, bad_y].into_iter().enumerate() {
            xy.x_cells[r][0] = Cell::number((r + 1) as f64);
            xy.data_sets[0].cells[r][0] = Cell { value: CellValue::Number(yv), excluded: false };
        }
        assert!(matches!(
            run_kind(&DataSheet::new("xy-by", "BY", TableArchetype::Xy(xy)).unwrap(), AnalysisKind::SimpleLogisticRegression(Default::default())),
            Err(StatsError::InvalidParameter { .. })
        ));
    }

    // AUD-13-06 & AUD-13-16: Multi-dataset per-dataset scalars & simple_logistic-before-ROC overlay order & AUC parity.
    let xs_m = [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let (y0, y1) = ([0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    let rs_m =
        run_kind(&xy_multi_sheet("xy-m", &xs_m, &[("DrugA", &y0), ("DrugB", &y1)]), AnalysisKind::SimpleLogisticRegression(Default::default()))
            .unwrap();
    assert!(rs_m.lookup_scalar("Beta1", Some(0)).unwrap() > 0.0 && rs_m.lookup_scalar("Beta1", Some(1)).unwrap() < 0.0);
    assert!(rs_m.lookup_scalar("Beta0", Some(0)).unwrap().is_finite() && rs_m.lookup_scalar("Beta0", Some(1)).unwrap().is_finite());
    assert!(
        rs_m.lookup_scalar("X50", Some(0)).is_some()
            && rs_m.lookup_scalar("X50", Some(1)).is_some()
            && rs_m.scalars.iter().all(|s| s.data_set_index.is_some())
    );
    let order: Vec<(&str, usize)> = rs_m.fitted_curves.iter().map(|c| (c.equation_id.as_str(), c.data_set_index)).collect();
    assert_eq!(order, vec![("simple_logistic", 0), ("ROC", 0), ("simple_logistic", 1), ("ROC", 1)]);
    let mut rpts: Vec<(f64, f64)> = rs_m.fitted_curves[1].points.iter().map(|p| (p.x, p.y)).collect();
    rpts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let auc: f64 = rpts.windows(2).map(|w| 0.5 * (w[1].0 - w[0].0) * (w[0].1 + w[1].1)).sum();
    assert_close(auc, rs_m.lookup_scalar("ROC_AUC", Some(0)).unwrap(), 1e-10, "SL ROC overlay AUC ds0");
}
