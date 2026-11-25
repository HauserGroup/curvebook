#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests covering Wave 22c-1 gaps:
//! - WF-39: Cox PH categorical dummy-coding & reference levels, AIC scalar, correlation matrix, Breslow curve overlays
//! - WF-40: Linear GLM AIC, Type III partial SS, parameter covariance matrix, residuals/influence, residual normality QOL-7 checklist
//! - WF-41: Logistic GLM VIF column toggle, AIC, Hosmer-Lemeshow scalars, classification 2x2 + cutoff sweep, predictions tab
//! - WF-42: Poisson GLM VIF column toggle, AIC, McFadden R^2, Pearson & deviance residuals
//! - WF-43: XY Descriptive Analysis per-dataset Trapezoidal AUC (raw & pre-summarized)
//! - WF-29: Row-wise descriptive statistics across active numeric columns (`MultipleVariables`, `Column`, `Grouped`)

use curvebook_doc::{
    AnalysisKind, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec,
    MultipleVariablesTable, ResultCell, ResultTabKind, SheetId, SubcolumnAggregation, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype,
    VariableRole, XyTable,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::linalg::DenseMatrix;
use curvebook_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression};
use curvebook_stats::survival::{CoxSubject, cox_ph_regression};

use crate::common::{
    assert_close as assert_approx, assert_diagnostics_only, cell_num, cell_p as cell_p_val, col_sheet, grouped_sheet, mv_cells_sheet, mv_num_sheet,
    run_kind, scalar_ds as scalar_value, tab_by_id, xy_rep_sheet,
};

fn design_matrix(cols: &[&[f64]]) -> DenseMatrix {
    let n = cols[0].len();
    let p = cols.len() + 1;
    let mut raw = Vec::with_capacity(n * p);
    for i in 0..n {
        raw.push(1.0);
        for col in cols {
            raw.push(col[i]);
        }
    }
    DenseMatrix::from_row_major(n, p, raw).unwrap()
}

fn glm_kind(family: GlmFamily, compute_vif: bool, predictors: Vec<usize>) -> AnalysisKind {
    AnalysisKind::GeneralizedLinearModel(GlmSpec {
        family,
        response_column: 0,
        predictor_columns: predictors,
        reference_levels: Vec::new(),
        compute_vif,
    })
}

#[test]
fn wf39_cox_categorical_reference_levels_aic_covariance_and_breslow_overlays() {
    let rows = [
        (3.0, 1.0, "Placebo", 62.0),
        (5.0, 1.0, "Placebo", 58.0),
        (7.0, 0.0, "Placebo", 55.0),
        (9.0, 1.0, "Placebo", 65.0),
        (6.0, 1.0, "LowDose", 60.0),
        (8.0, 1.0, "LowDose", 57.0),
        (11.0, 0.0, "LowDose", 54.0),
        (14.0, 1.0, "LowDose", 63.0),
        (10.0, 0.0, "HighDose", 61.0),
        (12.0, 1.0, "HighDose", 59.0),
        (15.0, 0.0, "HighDose", 56.0),
        (18.0, 1.0, "HighDose", 64.0),
        (1.0, 1.0, "HighDose", 70.0),
    ];
    let t_cells: Vec<Cell> = rows.iter().map(|r| Cell::number(r.0)).collect();
    let st_cells: Vec<Cell> = rows.iter().map(|r| Cell::number(r.1)).collect();
    let arm_cells: Vec<Cell> = rows.iter().enumerate().map(|(i, r)| if i == 12 { Cell::excluded_text(r.2) } else { Cell::text(r.2) }).collect();
    let age_cells: Vec<Cell> = rows.iter().map(|r| Cell::number(r.3)).collect();
    let sheet = mv_cells_sheet(
        "cox-cat",
        &[("Time", None, &t_cells), ("Status", None, &st_cells), ("Arm", Some(VariableRole::Categorical), &arm_cells), ("Age", None, &age_cells)],
    );

    let cox_kind = |refs: Vec<(usize, String)>| {
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: refs,
            tie_method: CoxTieMethod::Breslow,
        })
    };
    let res_low = run_kind(&sheet, cox_kind(vec![(2, String::from("LowDose"))])).unwrap();
    assert_diagnostics_only(&res_low, "Cox categorical");

    let ph_tab = tab_by_id(&res_low, "cox-ph");
    let names = vec![String::from("Arm[Placebo vs LowDose]"), String::from("Arm[HighDose vs LowDose]"), String::from("Age")];
    assert_eq!(ph_tab.row_labels, names);

    let subjects: Vec<CoxSubject> = rows[..12]
        .iter()
        .map(|&(t, st, arm, age)| CoxSubject {
            time: t,
            event: st != 0.0,
            x: vec![if arm == "Placebo" { 1.0 } else { 0.0 }, if arm == "HighDose" { 1.0 } else { 0.0 }, age],
        })
        .collect();
    let oracle = cox_ph_regression(&subjects, &names, CoxTieMethod::Breslow, ConfidenceLevel::default()).unwrap();
    let expected_aic = -2.0 * oracle.log_lik_model + 2.0 * 3.0;
    assert_approx(scalar_value(&res_low, "AIC", None), expected_aic, 1e-10, "Cox AIC scalar");

    let res_def = run_kind(&sheet, cox_kind(Vec::new())).unwrap();
    let ph_def = tab_by_id(&res_def, "cox-ph");
    assert_eq!(ph_def.row_labels, vec!["Arm[LowDose vs Placebo]", "Arm[HighDose vs Placebo]", "Age"]);
    assert_approx(scalar_value(&res_def, "AIC", None), expected_aic, 1e-8, "Cox AIC invariant to ref level");
    assert_approx(cell_num(&ph_def.rows[0][0]), -cell_num(&ph_tab.rows[0][0]), 1e-7, "Reciprocal reference contrast beta");

    let cov_tab = tab_by_id(&res_low, "cox-covariance");
    assert_eq!((cov_tab.kind, &cov_tab.column_headers, &cov_tab.row_labels), (ResultTabKind::CovarianceMatrix, &names, &names));
    for i in 0..3 {
        assert_approx(cell_num(&cov_tab.rows[i][i]), 1.0, 1e-12, "Correlation matrix diagonal == 1");
        for j in 0..3 {
            let v = cell_num(&cov_tab.rows[i][j]);
            assert!((-1.0 - 1e-12..=1.0 + 1e-12).contains(&v), "corr out of [-1, 1]: {v}");
            assert_approx(v, oracle.correlation_matrix[i][j], 1e-12, "Correlation matches oracle");
            assert_approx(v, cell_num(&cov_tab.rows[j][i]), 1e-12, "Correlation symmetric");
        }
    }

    assert_eq!(res_low.fitted_curves.len(), 1 + oracle.covariates.len());
    let base_curve = res_low.fitted_curves.iter().find(|c| c.equation_id == "cox_baseline" && c.data_set_index == 0).unwrap();
    assert_eq!(base_curve.points.len(), oracle.baseline_survival.len() + 1);
    assert_approx(base_curve.points[0].x, 0.0, 1e-12, "baseline curve t=0");
    assert_approx(base_curve.points[0].y, 1.0, 1e-12, "baseline curve S0(0)=1");
    for (idx, bpt) in oracle.baseline_survival.iter().enumerate() {
        assert_approx(base_curve.points[idx + 1].x, bpt.time, 1e-12, "baseline curve time");
        assert_approx(base_curve.points[idx + 1].y, (-bpt.cum_hazard).exp().clamp(0.0, 1.0), 1e-12, "baseline curve S0(t)");
    }
    for (j, cov) in oracle.covariates.iter().enumerate() {
        let cov_curve = res_low.fitted_curves.iter().find(|c| c.equation_id == "cox_covariate" && c.data_set_index == j + 1).unwrap();
        assert_eq!(cov_curve.points.len(), base_curve.points.len());
        let exp_b = cov.beta.clamp(-50.0, 50.0).exp();
        for (k, pt) in cov_curve.points.iter().enumerate() {
            assert_approx(pt.x, base_curve.points[k].x, 1e-12, "covariate curve x");
            assert_approx(pt.y, base_curve.points[k].y.powf(exp_b).clamp(0.0, 1.0), 1e-12, "covariate shifted survival S0^exp(beta)");
        }
    }
}

#[test]
fn wf40_linear_glm_aic_type_iii_covariance_residuals_and_normality_checklist() {
    let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let x2 = [2.0, 1.5, 4.0, 3.5, 6.0, 5.0, 8.5, 7.5, 9.0, 11.0];
    let y = [3.1, 4.0, 6.8, 7.2, 10.1, 10.4, 13.8, 14.1, 16.5, 19.2];
    let sheet = mv_num_sheet("mv-lin", &[("Y", &y), ("X1", &x1), ("X2", &x2)]);
    let res = run_kind(&sheet, glm_kind(GlmFamily::Linear, true, vec![1, 2])).unwrap();
    assert_diagnostics_only(&res, "Linear GLM normal");

    let terms = vec![String::from("Intercept"), String::from("X1"), String::from("X2")];
    let oracle = multiple_linear_regression(&design_matrix(&[&x1, &x2]), &y, &terms, true, true, ConfidenceLevel::default()).unwrap();
    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Linear AIC");
    assert_eq!(tab_by_id(&res, "glm-linear-anova").rows.len(), 3);

    let t3_tab = tab_by_id(&res, "glm-linear-type-iii");
    assert_eq!(t3_tab.kind, ResultTabKind::AnovaTable);
    assert_eq!(t3_tab.row_labels, vec!["X1", "X2"]);
    assert_eq!(t3_tab.column_headers, vec!["SS", "df", "MS", "F", "P value"]);
    for (idx, term) in oracle.terms[1..].iter().enumerate() {
        let ss = term.type_iii_ss.unwrap();
        assert_approx(cell_num(&t3_tab.rows[idx][0]), ss, 1e-12, "Type III SS");
        assert_approx(cell_num(&t3_tab.rows[idx][1]), 1.0, 1e-12, "Type III df");
        assert_approx(cell_num(&t3_tab.rows[idx][2]), ss, 1e-12, "Type III MS");
        assert_approx(cell_num(&t3_tab.rows[idx][3]), term.type_iii_f.unwrap(), 1e-12, "Type III F");
        assert_approx(cell_p_val(&t3_tab.rows[idx][4]), term.type_iii_p.unwrap(), 1e-12, "Type III P");
    }

    let cov_tab = tab_by_id(&res, "glm-linear-covariance");
    assert_eq!((cov_tab.kind, &cov_tab.column_headers, &cov_tab.row_labels), (ResultTabKind::CovarianceMatrix, &terms, &terms));
    for j in 0..3 {
        assert_approx(cell_num(&cov_tab.rows[j][j]).sqrt(), oracle.terms[j].se, 1e-10, "sqrt(Cov diag) == SE");
        for k in 0..3 {
            assert_approx(cell_num(&cov_tab.rows[j][k]), oracle.cov_beta[j][k], 1e-12, "Cov entry matches oracle");
        }
    }

    let resid_tab = tab_by_id(&res, "glm-linear-residuals");
    assert_eq!(resid_tab.kind, ResultTabKind::Residuals);
    assert_eq!(resid_tab.column_headers, vec!["Observed Y", "Predicted Y", "Residual", "Leverage", "Cook's D"]);
    assert_eq!(resid_tab.rows.len(), 10);
    let mut lev_sum = 0.0;
    for (i, &yi) in y.iter().enumerate() {
        let (obs, pred, r_val, lev, cook) = (
            cell_num(&resid_tab.rows[i][0]),
            cell_num(&resid_tab.rows[i][1]),
            cell_num(&resid_tab.rows[i][2]),
            cell_num(&resid_tab.rows[i][3]),
            cell_num(&resid_tab.rows[i][4]),
        );
        assert_approx(obs, yi, 1e-12, "Observed Y");
        assert_approx(pred, oracle.fitted[i], 1e-12, "Predicted Y");
        assert_approx(r_val, obs - pred, 1e-12, "Residual == Observed - Predicted");
        assert_approx(lev, oracle.leverage[i], 1e-12, "Leverage");
        assert_approx(cook, oracle.cooks_distance[i], 1e-12, "Cook's D");
        assert!(cook >= 0.0);
        lev_sum += lev;
    }
    assert_approx(lev_sum, 3.0, 1e-10, "Sum of leverages == rank");
    assert!(res.checklist.iter().find(|c| c.assumption == "Residual normality").unwrap().remediation.is_none());

    let sx: Vec<f64> = (0..12).map(f64::from).collect();
    let sy: Vec<f64> = (0..12).map(|r| if r == 11 { 500.0 } else { (r as f64) * 2.0 }).collect();
    let skew_res = run_kind(&mv_num_sheet("mv-skew", &[("Y", &sy), ("X", &sx)]), glm_kind(GlmFamily::Linear, false, vec![1])).unwrap();
    assert_diagnostics_only(&skew_res, "Linear GLM skewed");
    let skew_item = skew_res.checklist.iter().find(|c| c.assumption == "Residual normality").unwrap();
    assert_eq!((skew_item.status, &skew_item.remediation), (ChecklistStatus::Warn, &None));
}

#[test]
fn wf41_logistic_glm_vif_aic_hosmer_classification_sweep_and_predictions() {
    let dose = [1.0, 1.5, 2.0, 2.5, 3.0, 3.2, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5];
    let bio = [2.1, 1.9, 3.0, 2.8, 3.5, 4.2, 4.1, 5.2, 5.4, 6.1, 5.9, 7.0, 7.1, 8.0];
    let y = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let sheet = mv_num_sheet("mv-log", &[("Outcome", &y), ("Dose", &dose), ("Biomarker", &bio)]);

    let res_novif = run_kind(&sheet, glm_kind(GlmFamily::Logistic, false, vec![1, 2])).unwrap();
    assert_eq!(tab_by_id(&res_novif, "glm-logistic-params").column_headers.len(), 6);

    let res = run_kind(&sheet, glm_kind(GlmFamily::Logistic, true, vec![1, 2])).unwrap();
    assert_diagnostics_only(&res, "Logistic GLM");
    let params_tab = tab_by_id(&res, "glm-logistic-params");
    assert_eq!((params_tab.column_headers.len(), params_tab.column_headers[6].as_str(), &params_tab.rows[0][6]), (7, "VIF", &ResultCell::Empty));
    assert!(cell_num(&params_tab.rows[1][6]) > 1.0 && cell_num(&params_tab.rows[2][6]) > 1.0);

    let terms = vec![String::from("Intercept"), String::from("Dose"), String::from("Biomarker")];
    let oracle = multiple_logistic_regression(&design_matrix(&[&dose, &bio]), &y, &terms, true, ConfidenceLevel::default()).unwrap();
    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Logistic AIC");
    assert_approx(scalar_value(&res, "Hosmer-Lemeshow chi2", None), oracle.hosmer_lemeshow_chi2, 1e-12, "HL chi2");
    assert_approx(scalar_value(&res, "Hosmer-Lemeshow P", None), oracle.hosmer_lemeshow_p, 1e-12, "HL P");

    let cls_tab = tab_by_id(&res, "glm-logistic-classification");
    assert_eq!(cls_tab.kind, ResultTabKind::Diagnostics);
    assert_eq!(cls_tab.column_headers, vec!["Cutoff", "TP", "FP", "FN", "TN", "Sensitivity", "Specificity", "PPV", "NPV", "Accuracy"]);
    assert_eq!(cls_tab.rows.len(), 10);
    assert_approx(cell_num(&cls_tab.rows[0][0]), 0.5, 1e-12, "Primary cutoff 0.5");
    let (tp, fp, fn_cnt, tn) =
        (cell_num(&cls_tab.rows[0][1]), cell_num(&cls_tab.rows[0][2]), cell_num(&cls_tab.rows[0][3]), cell_num(&cls_tab.rows[0][4]));
    assert_approx(tp + fp + fn_cnt + tn, 14.0, 1e-12, "Confusion counts sum to N");
    assert_approx(cell_num(&cls_tab.rows[0][5]), tp / (tp + fn_cnt), 1e-12, "Sensitivity");
    assert_approx(cell_num(&cls_tab.rows[0][6]), tn / (tn + fp), 1e-12, "Specificity");
    assert_approx(cell_num(&cls_tab.rows[0][7]), tp / (tp + fp), 1e-12, "PPV");
    assert_approx(cell_num(&cls_tab.rows[0][8]), tn / (tn + fn_cnt), 1e-12, "NPV");
    assert_approx(cell_num(&cls_tab.rows[0][9]), (tp + tn) / 14.0, 1e-12, "Accuracy");
    for k in 1..=9 {
        assert_approx(cell_num(&cls_tab.rows[k][0]), (k as f64) * 0.1, 1e-12, "Sweep cutoff value");
        let sum_k: f64 = (1..=4).map(|c| cell_num(&cls_tab.rows[k][c])).sum();
        assert_approx(sum_k, 14.0, 1e-12, "Sweep confusion counts sum to N");
    }

    let pred_tab = tab_by_id(&res, "glm-logistic-predictions");
    assert_eq!(pred_tab.kind, ResultTabKind::Residuals);
    assert_eq!(pred_tab.column_headers, vec!["Observed Y", "Predicted probability", "Predicted class"]);
    assert_eq!(pred_tab.rows.len(), 14);
    for i in 0..14 {
        let p = cell_num(&pred_tab.rows[i][1]);
        assert!(p > 0.0 && p < 1.0);
        assert_approx(p, oracle.predicted_probs[i], 1e-12, "Predicted prob");
        assert_approx(cell_num(&pred_tab.rows[i][2]), if p >= 0.5 { 1.0 } else { 0.0 }, 1e-12, "Predicted class");
    }
}

#[test]
fn wf42_poisson_glm_vif_aic_mcfadden_and_residuals() {
    let x1 = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5];
    let x2 = [1.0, 1.2, 1.8, 2.0, 2.6, 2.9, 3.6, 4.0, 4.3, 5.1, 5.5, 6.2];
    let y = [1.0, 2.0, 2.0, 3.0, 4.0, 6.0, 8.0, 11.0, 15.0, 20.0, 26.0, 34.0];
    let sheet = mv_num_sheet("mv-pois", &[("Counts", &y), ("X1", &x1), ("X2", &x2)]);

    assert_eq!(tab_by_id(&run_kind(&sheet, glm_kind(GlmFamily::Poisson, false, vec![1, 2])).unwrap(), "glm-poisson-params").column_headers.len(), 6);

    let res = run_kind(&sheet, glm_kind(GlmFamily::Poisson, true, vec![1, 2])).unwrap();
    assert_diagnostics_only(&res, "Poisson GLM");
    let params_tab = tab_by_id(&res, "glm-poisson-params");
    assert_eq!((params_tab.column_headers.len(), params_tab.column_headers[6].as_str(), &params_tab.rows[0][6]), (7, "VIF", &ResultCell::Empty));
    assert!(cell_num(&params_tab.rows[1][6]) > 1.0);

    let terms = vec![String::from("Intercept"), String::from("X1"), String::from("X2")];
    let oracle = multiple_poisson_regression(&design_matrix(&[&x1, &x2]), &y, &terms, true, ConfidenceLevel::default()).unwrap();
    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Poisson AIC");
    let expected_mcfadden = (1.0 - oracle.log_lik_model / oracle.log_lik_null).clamp(0.0, 1.0);
    assert!(expected_mcfadden > 0.0 && expected_mcfadden < 1.0);
    assert_approx(scalar_value(&res, "McFadden R squared", None), expected_mcfadden, 1e-12, "Poisson McFadden R^2");

    let resid_tab = tab_by_id(&res, "glm-poisson-residuals");
    assert_eq!(resid_tab.kind, ResultTabKind::Residuals);
    assert_eq!(resid_tab.column_headers, vec!["Observed Y", "Predicted count", "Pearson residual", "Deviance residual"]);
    assert_eq!(resid_tab.rows.len(), 12);
    let (mut sum_p2, mut sum_d2) = (0.0, 0.0);
    for (i, &yi) in y.iter().enumerate() {
        let (obs, mu, pearson, dev) =
            (cell_num(&resid_tab.rows[i][0]), cell_num(&resid_tab.rows[i][1]), cell_num(&resid_tab.rows[i][2]), cell_num(&resid_tab.rows[i][3]));
        assert_approx(obs, yi, 1e-12, "Poisson Obs Y");
        assert_approx(mu, oracle.predicted_means[i], 1e-12, "Poisson Pred mu");
        assert_approx(pearson, (obs - mu) / mu.sqrt(), 1e-12, "Pearson residual identity");
        sum_p2 += pearson * pearson;
        sum_d2 += dev * dev;
    }
    assert_approx(sum_p2, oracle.pearson_chi2, 1e-10, "Sum(Pearson^2) == Pearson chi2");
    assert_approx(sum_d2, oracle.deviance_residual, 1e-10, "Sum(Deviance^2) == Residual deviance");
}

#[test]
fn wf43_xy_trapezoidal_auc_raw_and_presummarized() {
    let desc = AnalysisKind::DescriptiveStats(DescriptiveSpec::default());
    // 1. Unsorted XY table with 2 datasets: ds0 has 2 replicates, ds1 has 1 excluded cell.
    let mut sheet = xy_rep_sheet(
        "xy-auc",
        SubcolumnMode::RawReplicates(2),
        &[2.0, 0.0, 6.0, 4.0],
        &[("DS0", &[&[3.0, 5.0], &[-1.0, 1.0], &[-2.0, 2.0], &[2.0, 6.0]]), ("DS1", &[&[4.0], &[2.0], &[8.0], &[999.0]])],
    );
    if let TableArchetype::Xy(xy) = &mut sheet.archetype {
        xy.data_sets[1].cells[3][0].excluded = true;
    }
    let res = run_kind(&sheet, desc.clone()).unwrap();
    assert_approx(scalar_value(&res, "Trapezoidal AUC", Some(0)), 16.0, 1e-12, "XY AUC ds0 replicate mean");
    assert_approx(scalar_value(&res, "Trapezoidal AUC", Some(1)), 30.0, 1e-12, "XY AUC ds1 skipping excluded");

    // 2. Pre-summarized MeanSdN XY table also emits Trapezoidal AUC.
    let pre_sheet =
        xy_rep_sheet("xy-pre", SubcolumnMode::MeanSdN, &[0.0, 2.0, 5.0], &[("Y", &[&[1.0, 0.5, 4.0], &[3.0, 0.5, 4.0], &[9.0, 0.5, 4.0]])]);
    let pre_res = run_kind(&pre_sheet, desc.clone()).unwrap();
    assert_approx(scalar_value(&pre_res, "Trapezoidal AUC", Some(0)), 22.0, 1e-12, "Pre-summarized XY AUC");

    // 3. Fewer than 2 XY points does NOT emit Trapezoidal AUC.
    let mut xy_one = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy_one.x_cells[0][0] = Cell::number(1.0);
    xy_one.data_sets[0].cells[0][0] = Cell::number(5.0);
    let one_sheet = DataSheet::new(SheetId::new("xy-1"), "XY 1pt", TableArchetype::Xy(xy_one)).unwrap();
    assert!(!run_kind(&one_sheet, desc).unwrap().scalars.iter().any(|s| s.key == "Trapezoidal AUC"));
}

#[test]
fn wf29_row_descriptive_stats_across_archetypes() {
    let desc_fast = AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..Default::default() });

    // 1. MultipleVariablesTable: categorical ID column + 3 numeric columns + excluded cell.
    let mut mv = MultipleVariablesTable::new(4, 3).unwrap();
    mv.row_titles = vec!["Subj-A".into(), "Subj-B".into(), "Subj-C".into()];
    for (c, t) in ["SubjectID", "Rep1", "Rep2", "Rep3"].into_iter().enumerate() {
        mv.variables[c].title = t.into();
    }
    mv.variables[0].lock_role(VariableRole::Categorical);
    let rows = [
        [Cell::text("S1"), Cell::number(10.0), Cell::number(12.0), Cell::number(14.0)],
        [Cell::text("S2"), Cell::number(20.0), Cell::number(30.0), Cell::excluded_number(999.0)],
        [Cell::text("S3"), Cell::number(5.0), Cell::empty(), Cell::empty()],
    ];
    for (r, cells) in rows.into_iter().enumerate() {
        for (c, cell) in cells.into_iter().enumerate() {
            mv.variables[c].cells[r] = cell;
        }
    }
    let mv_sheet = DataSheet::new(SheetId::new("mv-row"), "MV Row", TableArchetype::MultipleVariables(mv)).unwrap();
    let mv_res = run_kind(&mv_sheet, desc_fast.clone()).unwrap();
    let mv_tab = tab_by_id(&mv_res, "row-descriptive-stats");
    assert_eq!(mv_tab.kind, ResultTabKind::Diagnostics);
    assert_eq!(mv_tab.column_headers, vec!["N", "Mean", "SD", "Min", "Max"]);
    assert_eq!(mv_tab.row_labels, vec!["Subj-A", "Subj-B", "Subj-C"]);
    for (c, (exp, lbl)) in [(3.0, "N"), (12.0, "Mean"), (2.0, "SD"), (10.0, "Min"), (14.0, "Max")].into_iter().enumerate() {
        assert_approx(cell_num(&mv_tab.rows[0][c]), exp, 1e-12, lbl);
    }
    assert_approx(cell_num(&mv_tab.rows[1][0]), 2.0, 1e-12, "Row 1 N (excluded skipped)");
    assert_approx(cell_num(&mv_tab.rows[1][1]), 25.0, 1e-12, "Row 1 Mean");
    assert_approx(cell_num(&mv_tab.rows[1][2]), 50.0_f64.sqrt(), 1e-12, "Row 1 SD");
    assert_approx(cell_num(&mv_tab.rows[2][0]), 1.0, 1e-12, "Row 2 N=1");
    assert_eq!(mv_tab.rows[2][2], ResultCell::Empty);

    // 2. ColumnTable with 3 numeric columns -> emits row-descriptive-stats.
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 2).unwrap();
    col.row_titles = vec!["Mouse 1".into(), "Mouse 2".into()];
    for (c, vals) in [[2.0, 1.0], [4.0, 3.0], [6.0, 5.0]].into_iter().enumerate() {
        col.data_sets[c].cells[0][0] = Cell::number(vals[0]);
        col.data_sets[c].cells[1][0] = Cell::number(vals[1]);
    }
    let col_res = run_kind(&DataSheet::new(SheetId::new("col-row"), "Col Row", TableArchetype::Column(col)).unwrap(), desc_fast.clone()).unwrap();
    let col_tab = tab_by_id(&col_res, "row-descriptive-stats");
    assert_eq!(col_tab.row_labels, vec!["Mouse 1", "Mouse 2"]);
    assert_approx(cell_num(&col_tab.rows[0][1]), 4.0, 1e-12, "Col Row 0 Mean");
    assert_approx(cell_num(&col_tab.rows[1][1]), 3.0, 1e-12, "Col Row 1 Mean");

    // 3. Single active numeric ColumnTable does NOT emit row-descriptive-stats.
    let single_res = run_kind(&col_sheet("col-1", &[("C", &[1.0, 2.0, 3.0])]), desc_fast).unwrap();
    assert!(!single_res.tabs.iter().any(|t| t.id == "row-descriptive-stats"));

    // 4. GroupedTable with 2 datasets x 2 replicates -> emits row-descriptive-stats.
    let grp_sheet = grouped_sheet(
        "grp-row",
        SubcolumnMode::RawReplicates(2),
        &["Day 1", "Day 2"],
        &[("A", &[&[1.0, 3.0], &[10.0]]), ("B", &[&[5.0, 7.0], &[20.0]])],
    );
    let grp_res = run_kind(
        &grp_sheet,
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::RowMeans,
            run_normality_battery: false,
            ..Default::default()
        }),
    )
    .unwrap();
    let grp_tab = tab_by_id(&grp_res, "row-descriptive-stats");
    assert_eq!(grp_tab.row_labels, vec!["Day 1", "Day 2"]);
    assert_approx(cell_num(&grp_tab.rows[0][0]), 2.0, 1e-12, "Grouped Day 1 N");
    assert_approx(cell_num(&grp_tab.rows[0][1]), 4.0, 1e-12, "Grouped Day 1 Mean");
    assert_approx(cell_num(&grp_tab.rows[1][1]), 15.0, 1e-12, "Grouped Day 2 Mean");
}
