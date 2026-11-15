#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests covering Wave 22c-1 gaps:
//! - WF-39: Cox PH categorical dummy-coding & reference levels, AIC scalar, correlation matrix, Breslow curve overlays
//! - WF-40: Linear GLM AIC, Type III partial SS, parameter covariance matrix, residuals/influence, residual normality QOL-7 checklist
//! - WF-41: Logistic GLM VIF column toggle, AIC, Hosmer-Lemeshow scalars, classification 2x2 + cutoff sweep, predictions tab
//! - WF-42: Poisson GLM VIF column toggle, AIC, McFadden R^2, Pearson & deviance residuals
//! - WF-43: XY Descriptive Analysis per-dataset Trapezoidal AUC (raw & pre-summarized)
//! - WF-29: Row-wise descriptive statistics across active numeric columns (`MultipleVariables`, `Column`, `Grouped`)

use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, DescriptiveSpec, GlmFamily, GlmSpec,
    GroupedTable, MultipleVariablesTable, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnAggregation, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, VariableRole, XyTable,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::linalg::DenseMatrix;
use lumencraft_stats::regression::{multiple_linear_regression, multiple_logistic_regression, multiple_poisson_regression};
use lumencraft_stats::run_analysis;
use lumencraft_stats::survival::{CoxSubject, cox_ph_regression};

const FORBIDDEN_RECOMMENDATION_PHRASES: &[&str] = &[
    "use mann-whitney",
    "use mann–whitney",
    "use wilcoxon",
    "use welch",
    "consider welch",
    "consider kruskal",
    "use kruskal",
    "use friedman",
    "consider friedman",
    "consider fisher",
    "prefer fisher",
    "try spearman",
    "apply log",
    "log transform",
    "remove outliers",
    "is recommended",
    "consider enabling",
    "consider brown-forsythe",
    "warrants larger",
];

fn assert_approx(actual: f64, expected: f64, tol: f64, label: &str) {
    assert!((actual - expected).abs() <= tol, "{label}: expected {expected} +/- {tol}, got {actual}");
}

fn tab_by_id<'a>(sheet: &'a ResultSheet, id: &str) -> &'a ResultTab {
    sheet.tabs.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("missing tab {id:?}"))
}

fn scalar_value(sheet: &ResultSheet, key: &str, ds: Option<usize>) -> f64 {
    sheet.scalars.iter().find(|s| s.key == key && s.data_set_index == ds).unwrap_or_else(|| panic!("missing scalar {key:?} for ds={ds:?}")).value
}

fn cell_num(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::Number { value, .. } => *value,
        other => panic!("expected ResultCell::Number, got {other:?}"),
    }
}

fn cell_p_val(cell: &ResultCell) -> f64 {
    match cell {
        ResultCell::PValue { raw, .. } => *raw,
        other => panic!("expected ResultCell::PValue, got {other:?}"),
    }
}

fn assert_diagnostics_only(sheet: &ResultSheet, ctx: &str) {
    for item in &sheet.checklist {
        assert!(item.remediation.is_none(), "{ctx}: expected remediation=None on {:?}", item.assumption);
        let lower = item.explanation.to_ascii_lowercase();
        for bad in FORBIDDEN_RECOMMENDATION_PHRASES {
            assert!(!lower.contains(bad), "{ctx}: forbidden phrase {bad:?} in {:?}", item.explanation);
        }
    }
}

#[test]
fn wf39_cox_categorical_reference_levels_aic_covariance_and_breslow_overlays() {
    let mut mv = MultipleVariablesTable::new(4, 13).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[1].title = String::from("Status");
    mv.variables[2].title = String::from("Arm");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Age");

    // 12 active rows across Placebo / LowDose / HighDose + 1 excluded cell row.
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
    for (r, &(t, st, arm, age)) in rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(st);
        let mut arm_cell = Cell::text(arm);
        if r == 12 {
            arm_cell.excluded = true;
        }
        mv.variables[2].cells[r] = arm_cell;
        mv.variables[3].cells[r] = Cell::number(age);
    }

    let sheet = DataSheet::new(SheetId::new("cox-cat"), "Cox Cat", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec_ref_low = AnalysisSpec::new(
        SheetId::new("r-cox-low"),
        SheetId::new("cox-cat"),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: vec![(2, String::from("LowDose"))],
            tie_method: CoxTieMethod::Breslow,
        }),
    )
    .unwrap();
    let res_low = run_analysis(&sheet, &spec_ref_low).unwrap();
    assert_diagnostics_only(&res_low, "Cox categorical");

    let ph_tab = tab_by_id(&res_low, "cox-ph");
    assert_eq!(ph_tab.row_labels, vec![String::from("Arm[Placebo vs LowDose]"), String::from("Arm[HighDose vs LowDose]"), String::from("Age"),]);

    // Direct cox_ph_regression oracle verification on active 12 subjects.
    let subjects: Vec<CoxSubject> = rows[..12]
        .iter()
        .map(|&(t, st, arm, age)| CoxSubject {
            time: t,
            event: st != 0.0,
            x: vec![if arm == "Placebo" { 1.0 } else { 0.0 }, if arm == "HighDose" { 1.0 } else { 0.0 }, age],
        })
        .collect();
    let names = vec![String::from("Arm[Placebo vs LowDose]"), String::from("Arm[HighDose vs LowDose]"), String::from("Age")];
    let oracle = cox_ph_regression(&subjects, &names, CoxTieMethod::Breslow, ConfidenceLevel::default()).unwrap();

    let expected_aic = -2.0 * oracle.log_lik_model + 2.0 * 3.0;
    assert_approx(scalar_value(&res_low, "AIC", None), expected_aic, 1e-10, "Cox AIC scalar");

    // Reference level defaulting vs explicit LowDose preserves model log-likelihood / AIC and flips beta sign.
    let spec_default = AnalysisSpec::new(
        SheetId::new("r-cox-def"),
        SheetId::new("cox-cat"),
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![2, 3],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Breslow,
        }),
    )
    .unwrap();
    let res_def = run_analysis(&sheet, &spec_default).unwrap();
    let ph_def = tab_by_id(&res_def, "cox-ph");
    assert_eq!(ph_def.row_labels, vec![String::from("Arm[LowDose vs Placebo]"), String::from("Arm[HighDose vs Placebo]"), String::from("Age"),]);
    assert_approx(scalar_value(&res_def, "AIC", None), expected_aic, 1e-8, "Cox AIC invariant to ref level");
    assert_approx(cell_num(&ph_def.rows[0][0]), -cell_num(&ph_tab.rows[0][0]), 1e-7, "Reciprocal reference contrast beta");

    // Covariance / correlation matrix tab ("cox-covariance").
    let cov_tab = tab_by_id(&res_low, "cox-covariance");
    assert_eq!(cov_tab.kind, ResultTabKind::CovarianceMatrix);
    assert_eq!(cov_tab.column_headers, names);
    assert_eq!(cov_tab.row_labels, names);
    for i in 0..3 {
        assert_approx(cell_num(&cov_tab.rows[i][i]), 1.0, 1e-12, "Correlation matrix diagonal == 1");
        for j in 0..3 {
            let v = cell_num(&cov_tab.rows[i][j]);
            assert!((-1.0 - 1e-12..=1.0 + 1e-12).contains(&v), "corr out of [-1, 1]: {v}");
            assert_approx(v, oracle.correlation_matrix[i][j], 1e-12, "Correlation matches oracle");
            assert_approx(v, cell_num(&cov_tab.rows[j][i]), 1e-12, "Correlation symmetric");
        }
    }

    // FittedCurveOverlays: baseline + 1 shifted curve per covariate.
    assert_eq!(res_low.fitted_curves.len(), 1 + oracle.covariates.len());
    let base_curve =
        res_low.fitted_curves.iter().find(|c| c.equation_id == "cox_baseline" && c.data_set_index == 0).expect("missing cox_baseline overlay");
    assert_eq!(base_curve.points.len(), oracle.baseline_survival.len() + 1);
    assert_approx(base_curve.points[0].x, 0.0, 1e-12, "baseline curve t=0");
    assert_approx(base_curve.points[0].y, 1.0, 1e-12, "baseline curve S0(0)=1");
    for (idx, bpt) in oracle.baseline_survival.iter().enumerate() {
        let s0_expected = (-bpt.cum_hazard).exp().clamp(0.0, 1.0);
        assert_approx(base_curve.points[idx + 1].x, bpt.time, 1e-12, "baseline curve time");
        assert_approx(base_curve.points[idx + 1].y, s0_expected, 1e-12, "baseline curve S0(t)");
    }

    for (j, cov) in oracle.covariates.iter().enumerate() {
        let cov_curve = res_low
            .fitted_curves
            .iter()
            .find(|c| c.equation_id == "cox_covariate" && c.data_set_index == j + 1)
            .unwrap_or_else(|| panic!("missing cox_covariate overlay for j={j}"));
        assert_eq!(cov_curve.points.len(), base_curve.points.len());
        let exp_b = cov.beta.clamp(-50.0, 50.0).exp();
        for (k, pt) in cov_curve.points.iter().enumerate() {
            let s0 = base_curve.points[k].y;
            assert_approx(pt.x, base_curve.points[k].x, 1e-12, "covariate curve x");
            assert_approx(pt.y, s0.powf(exp_b).clamp(0.0, 1.0), 1e-12, "covariate shifted survival S0^exp(beta)");
        }
    }
}

#[test]
fn wf40_linear_glm_aic_type_iii_covariance_residuals_and_normality_checklist() {
    let mut mv = MultipleVariablesTable::new(3, 10).unwrap();
    mv.variables[0].title = String::from("Y");
    mv.variables[1].title = String::from("X1");
    mv.variables[2].title = String::from("X2");

    let x1 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let x2 = [2.0, 1.5, 4.0, 3.5, 6.0, 5.0, 8.5, 7.5, 9.0, 11.0];
    let y = [3.1, 4.0, 6.8, 7.2, 10.1, 10.4, 13.8, 14.1, 16.5, 19.2];
    for r in 0..10 {
        mv.variables[0].cells[r] = Cell::number(y[r]);
        mv.variables[1].cells[r] = Cell::number(x1[r]);
        mv.variables[2].cells[r] = Cell::number(x2[r]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-lin"), "Linear GLM", TableArchetype::MultipleVariables(mv)).unwrap();
    let spec = AnalysisSpec::new(
        SheetId::new("r-lin"),
        SheetId::new("mv-lin"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_diagnostics_only(&res, "Linear GLM normal");

    let mut x_raw = Vec::with_capacity(30);
    for i in 0..10 {
        x_raw.extend_from_slice(&[1.0, x1[i], x2[i]]);
    }
    let x_mat = DenseMatrix::from_row_major(10, 3, x_raw).unwrap();
    let terms = vec![String::from("Intercept"), String::from("X1"), String::from("X2")];
    let oracle = multiple_linear_regression(&x_mat, &y, &terms, true, true, ConfidenceLevel::default()).unwrap();

    // AIC scalar.
    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Linear AIC");

    // Existing ANOVA tab unchanged (3 rows: Regression, Residual, Total).
    let anova_tab = tab_by_id(&res, "glm-linear-anova");
    assert_eq!(anova_tab.rows.len(), 3);

    // Type III partial SS tab.
    let t3_tab = tab_by_id(&res, "glm-linear-type-iii");
    assert_eq!(t3_tab.kind, ResultTabKind::AnovaTable);
    assert_eq!(t3_tab.row_labels, vec![String::from("X1"), String::from("X2")]);
    assert_eq!(t3_tab.column_headers, vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")]);
    for (idx, term) in oracle.terms[1..].iter().enumerate() {
        let ss = term.type_iii_ss.unwrap();
        assert_approx(cell_num(&t3_tab.rows[idx][0]), ss, 1e-12, "Type III SS");
        assert_approx(cell_num(&t3_tab.rows[idx][1]), 1.0, 1e-12, "Type III df");
        assert_approx(cell_num(&t3_tab.rows[idx][2]), ss, 1e-12, "Type III MS");
        assert_approx(cell_num(&t3_tab.rows[idx][3]), term.type_iii_f.unwrap(), 1e-12, "Type III F");
        assert_approx(cell_p_val(&t3_tab.rows[idx][4]), term.type_iii_p.unwrap(), 1e-12, "Type III P");
    }

    // Covariance matrix tab: sigma^2 * (X'X)^-1.
    let cov_tab = tab_by_id(&res, "glm-linear-covariance");
    assert_eq!(cov_tab.kind, ResultTabKind::CovarianceMatrix);
    assert_eq!(cov_tab.column_headers, terms);
    assert_eq!(cov_tab.row_labels, terms);
    for j in 0..3 {
        assert_approx(cell_num(&cov_tab.rows[j][j]).sqrt(), oracle.terms[j].se, 1e-10, "sqrt(Cov diag) == SE");
        for k in 0..3 {
            assert_approx(cell_num(&cov_tab.rows[j][k]), oracle.cov_beta[j][k], 1e-12, "Cov entry matches oracle");
        }
    }

    // Residuals tab.
    let resid_tab = tab_by_id(&res, "glm-linear-residuals");
    assert_eq!(resid_tab.kind, ResultTabKind::Residuals);
    assert_eq!(
        resid_tab.column_headers,
        vec![String::from("Observed Y"), String::from("Predicted Y"), String::from("Residual"), String::from("Leverage"), String::from("Cook's D"),]
    );
    assert_eq!(resid_tab.rows.len(), 10);
    let mut lev_sum = 0.0;
    for (i, &yi) in y.iter().enumerate() {
        let obs = cell_num(&resid_tab.rows[i][0]);
        let pred = cell_num(&resid_tab.rows[i][1]);
        let r_val = cell_num(&resid_tab.rows[i][2]);
        let lev = cell_num(&resid_tab.rows[i][3]);
        let cook = cell_num(&resid_tab.rows[i][4]);
        assert_approx(obs, yi, 1e-12, "Observed Y");
        assert_approx(pred, oracle.fitted[i], 1e-12, "Predicted Y");
        assert_approx(r_val, obs - pred, 1e-12, "Residual == Observed - Predicted");
        assert_approx(lev, oracle.leverage[i], 1e-12, "Leverage");
        assert_approx(cook, oracle.cooks_distance[i], 1e-12, "Cook's D");
        assert!(cook >= 0.0);
        lev_sum += lev;
    }
    assert_approx(lev_sum, 3.0, 1e-10, "Sum of leverages == rank");

    // Residual normality checklist item exists and uses QOL-7 diagnostics-only text.
    let norm_item = res.checklist.iter().find(|c| c.assumption == "Residual normality").expect("missing Residual normality checklist");
    assert!(norm_item.remediation.is_none());

    // Skewed non-normal residual case warns with remediation: None.
    let mut mv_skew = MultipleVariablesTable::new(2, 12).unwrap();
    mv_skew.variables[0].title = String::from("Y");
    mv_skew.variables[1].title = String::from("X");
    for r in 0..12 {
        mv_skew.variables[1].cells[r] = Cell::number(r as f64);
        mv_skew.variables[0].cells[r] = Cell::number(if r == 11 { 500.0 } else { (r as f64) * 2.0 });
    }
    let skew_sheet = DataSheet::new(SheetId::new("mv-skew"), "Skew", TableArchetype::MultipleVariables(mv_skew)).unwrap();
    let skew_spec = AnalysisSpec::new(
        SheetId::new("r-skew"),
        SheetId::new("mv-skew"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Linear,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    let skew_res = run_analysis(&skew_sheet, &skew_spec).unwrap();
    assert_diagnostics_only(&skew_res, "Linear GLM skewed");
    let skew_item = skew_res.checklist.iter().find(|c| c.assumption == "Residual normality").unwrap();
    assert_eq!(skew_item.status, ChecklistStatus::Warn);
    assert!(skew_item.remediation.is_none());
}

#[test]
fn wf41_logistic_glm_vif_aic_hosmer_classification_sweep_and_predictions() {
    let mut mv = MultipleVariablesTable::new(3, 14).unwrap();
    mv.variables[0].title = String::from("Outcome");
    mv.variables[1].title = String::from("Dose");
    mv.variables[2].title = String::from("Biomarker");

    let dose = [1.0, 1.5, 2.0, 2.5, 3.0, 3.2, 4.0, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 7.5];
    let bio = [2.1, 1.9, 3.0, 2.8, 3.5, 4.2, 4.1, 5.2, 5.4, 6.1, 5.9, 7.0, 7.1, 8.0];
    let y = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    for r in 0..14 {
        mv.variables[0].cells[r] = Cell::number(y[r]);
        mv.variables[1].cells[r] = Cell::number(dose[r]);
        mv.variables[2].cells[r] = Cell::number(bio[r]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-log"), "Logistic", TableArchetype::MultipleVariables(mv)).unwrap();

    // Without VIF: 6 parameter columns.
    let spec_novif = AnalysisSpec::new(
        SheetId::new("r-log0"),
        SheetId::new("mv-log"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    let res_novif = run_analysis(&sheet, &spec_novif).unwrap();
    assert_eq!(tab_by_id(&res_novif, "glm-logistic-params").column_headers.len(), 6);

    // With VIF: 7 parameter columns including "VIF".
    let spec_vif = AnalysisSpec::new(
        SheetId::new("r-log1"),
        SheetId::new("mv-log"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec_vif).unwrap();
    assert_diagnostics_only(&res, "Logistic GLM");

    let params_tab = tab_by_id(&res, "glm-logistic-params");
    assert_eq!(params_tab.column_headers.len(), 7);
    assert_eq!(params_tab.column_headers[6], "VIF");
    assert_eq!(params_tab.rows[0][6], ResultCell::Empty); // Intercept has no VIF
    assert!(cell_num(&params_tab.rows[1][6]) > 1.0);
    assert!(cell_num(&params_tab.rows[2][6]) > 1.0);

    let mut x_raw = Vec::with_capacity(14 * 3);
    for i in 0..14 {
        x_raw.extend_from_slice(&[1.0, dose[i], bio[i]]);
    }
    let x_mat = DenseMatrix::from_row_major(14, 3, x_raw).unwrap();
    let terms = vec![String::from("Intercept"), String::from("Dose"), String::from("Biomarker")];
    let oracle = multiple_logistic_regression(&x_mat, &y, &terms, true, ConfidenceLevel::default()).unwrap();

    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Logistic AIC");
    assert_approx(scalar_value(&res, "Hosmer-Lemeshow chi2", None), oracle.hosmer_lemeshow_chi2, 1e-12, "HL chi2");
    assert_approx(scalar_value(&res, "Hosmer-Lemeshow P", None), oracle.hosmer_lemeshow_p, 1e-12, "HL P");

    // Classification tab: 2x2 at 0.50 + sweep 0.1..=0.9 (10 rows total).
    let cls_tab = tab_by_id(&res, "glm-logistic-classification");
    assert_eq!(cls_tab.kind, ResultTabKind::Diagnostics);
    assert_eq!(
        cls_tab.column_headers,
        vec![
            String::from("Cutoff"),
            String::from("TP"),
            String::from("FP"),
            String::from("FN"),
            String::from("TN"),
            String::from("Sensitivity"),
            String::from("Specificity"),
            String::from("PPV"),
            String::from("NPV"),
            String::from("Accuracy"),
        ]
    );
    assert_eq!(cls_tab.rows.len(), 10);
    assert_approx(cell_num(&cls_tab.rows[0][0]), 0.5, 1e-12, "Primary cutoff 0.5");
    let tp = cell_num(&cls_tab.rows[0][1]);
    let fp = cell_num(&cls_tab.rows[0][2]);
    let fn_cnt = cell_num(&cls_tab.rows[0][3]);
    let tn = cell_num(&cls_tab.rows[0][4]);
    assert_approx(tp + fp + fn_cnt + tn, 14.0, 1e-12, "Confusion counts sum to N");
    assert_approx(cell_num(&cls_tab.rows[0][5]), tp / (tp + fn_cnt), 1e-12, "Sensitivity");
    assert_approx(cell_num(&cls_tab.rows[0][6]), tn / (tn + fp), 1e-12, "Specificity");
    assert_approx(cell_num(&cls_tab.rows[0][7]), tp / (tp + fp), 1e-12, "PPV");
    assert_approx(cell_num(&cls_tab.rows[0][8]), tn / (tn + fn_cnt), 1e-12, "NPV");
    assert_approx(cell_num(&cls_tab.rows[0][9]), (tp + tn) / 14.0, 1e-12, "Accuracy");
    for k in 1..=9 {
        assert_approx(cell_num(&cls_tab.rows[k][0]), (k as f64) * 0.1, 1e-12, "Sweep cutoff value");
        let sum_k = cell_num(&cls_tab.rows[k][1]) + cell_num(&cls_tab.rows[k][2]) + cell_num(&cls_tab.rows[k][3]) + cell_num(&cls_tab.rows[k][4]);
        assert_approx(sum_k, 14.0, 1e-12, "Sweep confusion counts sum to N");
    }

    // Predictions tab.
    let pred_tab = tab_by_id(&res, "glm-logistic-predictions");
    assert_eq!(pred_tab.kind, ResultTabKind::Residuals);
    assert_eq!(pred_tab.column_headers, vec![String::from("Observed Y"), String::from("Predicted probability"), String::from("Predicted class")]);
    assert_eq!(pred_tab.rows.len(), 14);
    for i in 0..14 {
        let p = cell_num(&pred_tab.rows[i][1]);
        assert!(p > 0.0 && p < 1.0);
        assert_approx(p, oracle.predicted_probs[i], 1e-12, "Predicted prob");
        let cls = cell_num(&pred_tab.rows[i][2]);
        assert_approx(cls, if p >= 0.5 { 1.0 } else { 0.0 }, 1e-12, "Predicted class");
    }
}

#[test]
fn wf42_poisson_glm_vif_aic_mcfadden_and_residuals() {
    let mut mv = MultipleVariablesTable::new(3, 12).unwrap();
    mv.variables[0].title = String::from("Counts");
    mv.variables[1].title = String::from("X1");
    mv.variables[2].title = String::from("X2");

    let x1 = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5];
    let x2 = [1.0, 1.2, 1.8, 2.0, 2.6, 2.9, 3.6, 4.0, 4.3, 5.1, 5.5, 6.2];
    let y = [1.0, 2.0, 2.0, 3.0, 4.0, 6.0, 8.0, 11.0, 15.0, 20.0, 26.0, 34.0];
    for r in 0..12 {
        mv.variables[0].cells[r] = Cell::number(y[r]);
        mv.variables[1].cells[r] = Cell::number(x1[r]);
        mv.variables[2].cells[r] = Cell::number(x2[r]);
    }
    let sheet = DataSheet::new(SheetId::new("mv-pois"), "Poisson", TableArchetype::MultipleVariables(mv)).unwrap();

    let spec_novif = AnalysisSpec::new(
        SheetId::new("r-p0"),
        SheetId::new("mv-pois"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Poisson,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )
    .unwrap();
    let res_novif = run_analysis(&sheet, &spec_novif).unwrap();
    assert_eq!(tab_by_id(&res_novif, "glm-poisson-params").column_headers.len(), 6);

    let spec_vif = AnalysisSpec::new(
        SheetId::new("r-p1"),
        SheetId::new("mv-pois"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: GlmFamily::Poisson,
            response_column: 0,
            predictor_columns: vec![1, 2],
            reference_levels: Vec::new(),
            compute_vif: true,
        }),
    )
    .unwrap();
    let res = run_analysis(&sheet, &spec_vif).unwrap();
    assert_diagnostics_only(&res, "Poisson GLM");

    let params_tab = tab_by_id(&res, "glm-poisson-params");
    assert_eq!(params_tab.column_headers.len(), 7);
    assert_eq!(params_tab.column_headers[6], "VIF");
    assert_eq!(params_tab.rows[0][6], ResultCell::Empty);
    assert!(cell_num(&params_tab.rows[1][6]) > 1.0);

    let mut x_raw = Vec::with_capacity(12 * 3);
    for i in 0..12 {
        x_raw.extend_from_slice(&[1.0, x1[i], x2[i]]);
    }
    let x_mat = DenseMatrix::from_row_major(12, 3, x_raw).unwrap();
    let terms = vec![String::from("Intercept"), String::from("X1"), String::from("X2")];
    let oracle = multiple_poisson_regression(&x_mat, &y, &terms, true, ConfidenceLevel::default()).unwrap();

    assert_approx(scalar_value(&res, "AIC", None), oracle.aic, 1e-12, "Poisson AIC");
    let expected_mcfadden = (1.0 - oracle.log_lik_model / oracle.log_lik_null).clamp(0.0, 1.0);
    assert!(expected_mcfadden > 0.0 && expected_mcfadden < 1.0);
    assert_approx(scalar_value(&res, "McFadden R squared", None), expected_mcfadden, 1e-12, "Poisson McFadden R^2");

    let resid_tab = tab_by_id(&res, "glm-poisson-residuals");
    assert_eq!(resid_tab.kind, ResultTabKind::Residuals);
    assert_eq!(
        resid_tab.column_headers,
        vec![String::from("Observed Y"), String::from("Predicted count"), String::from("Pearson residual"), String::from("Deviance residual"),]
    );
    assert_eq!(resid_tab.rows.len(), 12);
    let mut sum_p2 = 0.0;
    let mut sum_d2 = 0.0;
    for (i, &yi) in y.iter().enumerate() {
        let obs = cell_num(&resid_tab.rows[i][0]);
        let mu = cell_num(&resid_tab.rows[i][1]);
        let pearson = cell_num(&resid_tab.rows[i][2]);
        let dev = cell_num(&resid_tab.rows[i][3]);
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
    // 1. Unsorted XY table with 2 datasets: ds0 has 2 replicates per row, ds1 has single replicate & 1 excluded cell.
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).unwrap();
    // Out-of-order X values: [2, 0, 6, 4] -> sorted X: [0, 2, 4, 6].
    let xs = [2.0, 0.0, 6.0, 4.0];
    // ds0 means at X=(0,2,4,6): (0.0, 4.0, 4.0, 0.0) -> AUC = 0.5*2*4 + 0.5*2*8 + 0.5*2*4 = 4 + 8 + 4 = 16.0.
    let ds0_reps = [(3.0, 5.0), (-1.0, 1.0), (-2.0, 2.0), (2.0, 6.0)];
    // ds1 valid points at X=(0,2,6) (row X=4 excluded): y=(2, 4, 8) -> AUC = 0.5*2*(2+4) + 0.5*4*(4+8) = 6 + 24 = 30.0.
    let ds1_vals = [(4.0, true), (2.0, true), (8.0, true), (999.0, false)];
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number(xs[r]);
        xy.data_sets[0].cells[r][0] = Cell::number(ds0_reps[r].0);
        xy.data_sets[0].cells[r][1] = Cell::number(ds0_reps[r].1);
        let mut c = Cell::number(ds1_vals[r].0);
        if !ds1_vals[r].1 {
            c.excluded = true;
        }
        xy.data_sets[1].cells[r][0] = c;
    }
    let sheet = DataSheet::new(SheetId::new("xy-auc"), "XY AUC", TableArchetype::Xy(xy)).unwrap();
    let spec =
        AnalysisSpec::new(SheetId::new("r-xy-auc"), SheetId::new("xy-auc"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let res = run_analysis(&sheet, &spec).unwrap();
    assert_approx(scalar_value(&res, "Trapezoidal AUC", Some(0)), 16.0, 1e-12, "XY AUC ds0 replicate mean");
    assert_approx(scalar_value(&res, "Trapezoidal AUC", Some(1)), 30.0, 1e-12, "XY AUC ds1 skipping excluded");

    // 2. Pre-summarized MeanSdN XY table also emits Trapezoidal AUC!
    let mut xy_pre = XyTable::new(SubcolumnMode::MeanSdN, 1, 3).unwrap();
    for (r, &(x, mean_y)) in [(0.0, 1.0), (2.0, 3.0), (5.0, 9.0)].iter().enumerate() {
        xy_pre.x_cells[r][0] = Cell::number(x);
        xy_pre.data_sets[0].cells[r][0] = Cell::number(mean_y);
        xy_pre.data_sets[0].cells[r][1] = Cell::number(0.5);
        xy_pre.data_sets[0].cells[r][2] = Cell::number(4.0);
    }
    let pre_sheet = DataSheet::new(SheetId::new("xy-pre"), "XY Pre", TableArchetype::Xy(xy_pre)).unwrap();
    let pre_spec =
        AnalysisSpec::new(SheetId::new("r-xy-pre"), SheetId::new("xy-pre"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let pre_res = run_analysis(&pre_sheet, &pre_spec).unwrap();
    // AUC = 0.5*(2-0)*(1+3) + 0.5*(5-2)*(3+9) = 4 + 18 = 22.0.
    assert_approx(scalar_value(&pre_res, "Trapezoidal AUC", Some(0)), 22.0, 1e-12, "Pre-summarized XY AUC");

    // 3. Fewer than 2 XY points does NOT emit Trapezoidal AUC.
    let mut xy_one = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy_one.x_cells[0][0] = Cell::number(1.0);
    xy_one.data_sets[0].cells[0][0] = Cell::number(5.0);
    let one_sheet = DataSheet::new(SheetId::new("xy-1"), "XY 1pt", TableArchetype::Xy(xy_one)).unwrap();
    let one_spec =
        AnalysisSpec::new(SheetId::new("r-xy-1"), SheetId::new("xy-1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let one_res = run_analysis(&one_sheet, &one_spec).unwrap();
    assert!(!one_res.scalars.iter().any(|s| s.key == "Trapezoidal AUC"));
}

#[test]
fn wf29_row_descriptive_stats_across_archetypes() {
    // 1. MultipleVariablesTable: categorical ID column + 3 numeric columns + excluded cell.
    let mut mv = MultipleVariablesTable::new(4, 3).unwrap();
    mv.row_titles = vec![String::from("Subj-A"), String::from("Subj-B"), String::from("Subj-C")];
    mv.variables[0].title = String::from("SubjectID");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Rep1");
    mv.variables[2].title = String::from("Rep2");
    mv.variables[3].title = String::from("Rep3");

    mv.variables[0].cells[0] = Cell::text("S1");
    mv.variables[1].cells[0] = Cell::number(10.0);
    mv.variables[2].cells[0] = Cell::number(12.0);
    mv.variables[3].cells[0] = Cell::number(14.0);

    mv.variables[0].cells[1] = Cell::text("S2");
    mv.variables[1].cells[1] = Cell::number(20.0);
    mv.variables[2].cells[1] = Cell::number(30.0);
    let mut ex_cell = Cell::number(999.0);
    ex_cell.excluded = true;
    mv.variables[3].cells[1] = ex_cell;

    mv.variables[0].cells[2] = Cell::text("S3");
    mv.variables[1].cells[2] = Cell::number(5.0);
    // Single active value in row 2 -> N=1, Mean=5, SD=Empty, Min=5, Max=5.

    let mv_sheet = DataSheet::new(SheetId::new("mv-row"), "MV Row", TableArchetype::MultipleVariables(mv)).unwrap();
    let mv_spec = AnalysisSpec::new(
        SheetId::new("r-mv-row"),
        SheetId::new("mv-row"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..DescriptiveSpec::default() }),
    )
    .unwrap();
    let mv_res = run_analysis(&mv_sheet, &mv_spec).unwrap();
    let mv_tab = tab_by_id(&mv_res, "row-descriptive-stats");
    assert_eq!(mv_tab.kind, ResultTabKind::Diagnostics);
    assert_eq!(mv_tab.column_headers, vec!["N", "Mean", "SD", "Min", "Max"]);
    assert_eq!(mv_tab.row_labels, vec!["Subj-A", "Subj-B", "Subj-C"]);
    assert_approx(cell_num(&mv_tab.rows[0][0]), 3.0, 1e-12, "Row 0 N");
    assert_approx(cell_num(&mv_tab.rows[0][1]), 12.0, 1e-12, "Row 0 Mean");
    assert_approx(cell_num(&mv_tab.rows[0][2]), 2.0, 1e-12, "Row 0 SD");
    assert_approx(cell_num(&mv_tab.rows[0][3]), 10.0, 1e-12, "Row 0 Min");
    assert_approx(cell_num(&mv_tab.rows[0][4]), 14.0, 1e-12, "Row 0 Max");
    assert_approx(cell_num(&mv_tab.rows[1][0]), 2.0, 1e-12, "Row 1 N (excluded skipped)");
    assert_approx(cell_num(&mv_tab.rows[1][1]), 25.0, 1e-12, "Row 1 Mean");
    assert_approx(cell_num(&mv_tab.rows[1][2]), 50.0_f64.sqrt(), 1e-12, "Row 1 SD");
    assert_approx(cell_num(&mv_tab.rows[2][0]), 1.0, 1e-12, "Row 2 N=1");
    assert_eq!(mv_tab.rows[2][2], ResultCell::Empty); // N < 2 -> SD empty

    // 2. ColumnTable with 3 numeric columns -> emits row-descriptive-stats.
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 2).unwrap();
    col.row_titles = vec![String::from("Mouse 1"), String::from("Mouse 2")];
    col.data_sets[0].cells[0][0] = Cell::number(2.0);
    col.data_sets[1].cells[0][0] = Cell::number(4.0);
    col.data_sets[2].cells[0][0] = Cell::number(6.0);
    col.data_sets[0].cells[1][0] = Cell::number(1.0);
    col.data_sets[1].cells[1][0] = Cell::number(3.0);
    col.data_sets[2].cells[1][0] = Cell::number(5.0);
    let col_sheet = DataSheet::new(SheetId::new("col-row"), "Col Row", TableArchetype::Column(col)).unwrap();
    let col_spec = AnalysisSpec::new(
        SheetId::new("r-col-row"),
        SheetId::new("col-row"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..DescriptiveSpec::default() }),
    )
    .unwrap();
    let col_res = run_analysis(&col_sheet, &col_spec).unwrap();
    let col_tab = tab_by_id(&col_res, "row-descriptive-stats");
    assert_eq!(col_tab.row_labels, vec!["Mouse 1", "Mouse 2"]);
    assert_approx(cell_num(&col_tab.rows[0][1]), 4.0, 1e-12, "Col Row 0 Mean");
    assert_approx(cell_num(&col_tab.rows[1][1]), 3.0, 1e-12, "Col Row 1 Mean");

    // 3. Single active numeric ColumnTable does NOT emit row-descriptive-stats.
    let mut single_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    for r in 0..3 {
        single_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let single_sheet = DataSheet::new(SheetId::new("col-1"), "Col 1", TableArchetype::Column(single_col)).unwrap();
    let single_spec = AnalysisSpec::new(
        SheetId::new("r-col-1"),
        SheetId::new("col-1"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec { run_normality_battery: false, ..DescriptiveSpec::default() }),
    )
    .unwrap();
    let single_res = run_analysis(&single_sheet, &single_spec).unwrap();
    assert!(!single_res.tabs.iter().any(|t| t.id == "row-descriptive-stats"));

    // 4. GroupedTable with 2 datasets x 2 replicates -> emits row-descriptive-stats.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap();
    grp.row_titles = vec![String::from("Day 1"), String::from("Day 2")];
    grp.data_sets[0].cells[0][0] = Cell::number(1.0);
    grp.data_sets[0].cells[0][1] = Cell::number(3.0);
    grp.data_sets[1].cells[0][0] = Cell::number(5.0);
    grp.data_sets[1].cells[0][1] = Cell::number(7.0);
    grp.data_sets[0].cells[1][0] = Cell::number(10.0);
    grp.data_sets[1].cells[1][0] = Cell::number(20.0);
    let grp_sheet = DataSheet::new(SheetId::new("grp-row"), "Grouped", TableArchetype::Grouped(grp)).unwrap();
    let grp_spec = AnalysisSpec::new(
        SheetId::new("r-grp-row"),
        SheetId::new("grp-row"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec {
            aggregation: SubcolumnAggregation::RowMeans,
            run_normality_battery: false,
            ..DescriptiveSpec::default()
        }),
    )
    .unwrap();
    let grp_res = run_analysis(&grp_sheet, &grp_spec).unwrap();
    let grp_tab = tab_by_id(&grp_res, "row-descriptive-stats");
    assert_eq!(grp_tab.row_labels, vec!["Day 1", "Day 2"]);
    // Under RowMeans: Day 1 has column means [2.0, 6.0] -> N=2, Mean=4.0; Day 2 has [10.0, 20.0] -> N=2, Mean=15.0.
    assert_approx(cell_num(&grp_tab.rows[0][0]), 2.0, 1e-12, "Grouped Day 1 N");
    assert_approx(cell_num(&grp_tab.rows[0][1]), 4.0, 1e-12, "Grouped Day 1 Mean");
    assert_approx(cell_num(&grp_tab.rows[1][1]), 15.0, 1e-12, "Grouped Day 2 Mean");
}
