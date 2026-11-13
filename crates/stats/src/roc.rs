//! Receiver Operating Characteristic (ROC) curve analysis: empirical ROC thresholds,
//! trapezoidal AUC, DeLong (1988) exact structural-components SE/CI and two-curve comparison
//! (correlated & uncorrelated), Hanley–McNeil SE, Youden's *J* optimal cutoff, PPV/NPV/LR+, and
//! `ResultSheet` builders.

pub use lumencraft_doc::RocCiMethod;
use lumencraft_doc::{
    AnalysisSpec, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, ResultSheet, ResultTabKind, RocSpec, TableArchetype,
};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::{norm_two_sided_isf, norm_two_sided_p};
use serde::{Deserialize, Serialize};

use crate::common::{
    build_tab, checklist_item, ci_cell, extract_numeric_columns, init_result_sheet, num_cell, opt_num_cell, p_cell, push_scalar,
    validate_finite_slice,
};
use crate::error::StatsError;

// ============================================================================
// Typed Outputs
// ============================================================================

/// Single threshold operating point on an empirical ROC curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocOperatingPoint {
    pub cutoff: f64,
    pub sensitivity: f64,
    pub specificity: f64,
    pub one_minus_specificity: f64,
    pub youden_j: f64,
    pub ppv: f64,
    pub npv: f64,
    pub likelihood_ratio_pos: Option<f64>,
    pub likelihood_ratio_neg: Option<f64>,
}

/// Complete ROC curve analysis for `cases` vs. `controls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocResult {
    pub n_controls: usize,
    pub n_cases: usize,
    pub auc: f64,
    pub se: f64,
    pub ci: (f64, f64),
    pub p_value_vs_05: f64,
    pub optimal_cutoff: RocOperatingPoint,
    pub points: Vec<RocOperatingPoint>,
    pub delong_v10: Vec<f64>,
    pub delong_v01: Vec<f64>,
}

/// DeLong two-curve AUC comparison result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocComparisonResult {
    pub auc_a: f64,
    pub auc_b: f64,
    pub auc_diff: f64,
    pub se_diff: f64,
    pub z_statistic: f64,
    pub p_value: f64,
    pub correlated: bool,
}

// ============================================================================
// Core Empirical ROC & DeLong (1988) Structural Components
// ============================================================================

/// Compute empirical ROC curve, trapezoidal AUC, DeLong (1988) exact SE/CI (or Hanley–McNeil),
/// and optimal Youden *J* cutoff assuming higher test values indicate `cases`.
pub fn roc_curve_analysis(controls: &[f64], cases: &[f64], ci_method: RocCiMethod, confidence: ConfidenceLevel) -> Result<RocResult, StatsError> {
    validate_finite_slice(controls)?;
    validate_finite_slice(cases)?;
    let n0 = controls.len();
    let n1 = cases.len();
    let n0f = n0 as f64;
    let n1f = n1 as f64;

    // Collect sorted unique thresholds descending so curve traces smoothly from (0,0) to (1,1).
    let mut thresholds: Vec<f64> = controls.iter().chain(cases.iter()).copied().collect();
    thresholds.sort_by(f64::total_cmp);
    thresholds.dedup();
    thresholds.reverse();

    let mut points = Vec::with_capacity(thresholds.len() + 1);
    points.push(RocOperatingPoint {
        cutoff: thresholds.first().map_or(f64::INFINITY, |&t| t + 1.0),
        sensitivity: 0.0,
        specificity: 1.0,
        one_minus_specificity: 0.0,
        youden_j: 0.0,
        ppv: 1.0,
        npv: n0f / (n0f + n1f),
        likelihood_ratio_pos: None,
        likelihood_ratio_neg: Some(1.0),
    });

    for &t in &thresholds {
        let tp = cases.iter().filter(|&&x| x >= t).count() as f64;
        let fp = controls.iter().filter(|&&x| x >= t).count() as f64;
        let tn = n0f - fp;
        let fn_cnt = n1f - tp;
        let sens = tp / n1f;
        let spec = tn / n0f;
        let fpr = 1.0 - spec;
        let ppv = if tp + fp > 0.0 { tp / (tp + fp) } else { 1.0 };
        let npv = if tn + fn_cnt > 0.0 { tn / (tn + fn_cnt) } else { 1.0 };
        let lr_pos = if fpr > 0.0 { Some(sens / fpr) } else { None };
        let lr_neg = if spec > 0.0 { Some((1.0 - sens) / spec) } else { None };
        points.push(RocOperatingPoint {
            cutoff: t,
            sensitivity: sens,
            specificity: spec,
            one_minus_specificity: fpr,
            youden_j: sens + spec - 1.0,
            ppv,
            npv,
            likelihood_ratio_pos: lr_pos,
            likelihood_ratio_neg: lr_neg,
        });
    }

    // Trapezoidal AUC across points.
    let mut auc = 0.0_f64;
    for w in points.windows(2) {
        let dx = w[1].one_minus_specificity - w[0].one_minus_specificity;
        let avg_y = 0.5 * (w[1].sensitivity + w[0].sensitivity);
        auc += dx * avg_y;
    }
    auc = auc.clamp(0.0, 1.0);

    // DeLong structural components V_10 (for each case) and V_01 (for each control).
    let mut v10 = vec![0.0_f64; n1];
    let mut v01 = vec![0.0_f64; n0];
    for (i, &x_case) in cases.iter().enumerate() {
        let mut s = 0.0_f64;
        for (j, &y_ctrl) in controls.iter().enumerate() {
            let psi = if x_case > y_ctrl {
                1.0
            } else if x_case == y_ctrl {
                0.5
            } else {
                0.0
            };
            s += psi;
            v01[j] += psi;
        }
        v10[i] = s / n0f;
    }
    for v in &mut v01 {
        *v /= n1f;
    }

    let se = match ci_method {
        RocCiMethod::DeLong => {
            let s10 = if n1 > 1 { v10.iter().map(|&v| (v - auc).powi(2)).sum::<f64>() / ((n1 - 1) as f64) } else { 0.0 };
            let s01 = if n0 > 1 { v01.iter().map(|&v| (v - auc).powi(2)).sum::<f64>() / ((n0 - 1) as f64) } else { 0.0 };
            ((s10 / n1f) + (s01 / n0f)).max(0.0).sqrt()
        }
        RocCiMethod::HanleyMcNeil => {
            let q1 = auc / (2.0 - auc);
            let q2 = (2.0 * auc * auc) / (1.0 + auc);
            let num = auc * (1.0 - auc) + (n1f - 1.0) * (q1 - auc * auc) + (n0f - 1.0) * (q2 - auc * auc);
            (num / (n1f * n0f)).max(0.0).sqrt()
        }
    };

    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let ci = ((auc - z_crit * se).clamp(0.0, 1.0), (auc + z_crit * se).clamp(0.0, 1.0));
    let p_value_vs_05 = if se > 0.0 { norm_two_sided_p((auc - 0.5) / se) } else { 1.0 };

    let optimal_cutoff = points.iter().skip(1).max_by(|a, b| a.youden_j.total_cmp(&b.youden_j)).cloned().unwrap_or_else(|| points[0].clone());

    Ok(RocResult { n_controls: n0, n_cases: n1, auc, se, ci, p_value_vs_05, optimal_cutoff, points, delong_v10: v10, delong_v01: v01 })
}

/// Compare two correlated ROC curves evaluated on the **same** controls and cases (DeLong 1988).
pub fn compare_roc_delong_paired(roc_a: &RocResult, roc_b: &RocResult) -> Result<RocComparisonResult, StatsError> {
    if roc_a.n_cases != roc_b.n_cases || roc_a.n_controls != roc_b.n_controls || roc_a.n_cases < 2 || roc_a.n_controls < 2 {
        return Err(StatsError::InsufficientSampleSize {
            context: "DeLong paired ROC comparison",
            required: 2,
            actual: roc_a.n_cases.min(roc_a.n_controls),
        });
    }
    let n1 = roc_a.n_cases as f64;
    let n0 = roc_a.n_controls as f64;

    let mut cov10 = 0.0_f64;
    for i in 0..roc_a.n_cases {
        let da = roc_a.delong_v10[i] - roc_a.auc;
        let db = roc_b.delong_v10[i] - roc_b.auc;
        cov10 += (da - db) * (da - db);
    }
    cov10 /= n1 - 1.0;

    let mut cov01 = 0.0_f64;
    for j in 0..roc_a.n_controls {
        let da = roc_a.delong_v01[j] - roc_a.auc;
        let db = roc_b.delong_v01[j] - roc_b.auc;
        cov01 += (da - db) * (da - db);
    }
    cov01 /= n0 - 1.0;

    let se_diff = ((cov10 / n1) + (cov01 / n0)).max(0.0).sqrt();
    let auc_diff = roc_a.auc - roc_b.auc;
    let z = if se_diff > 0.0 { auc_diff / se_diff } else { 0.0 };
    let p = norm_two_sided_p(z);

    Ok(RocComparisonResult { auc_a: roc_a.auc, auc_b: roc_b.auc, auc_diff, se_diff, z_statistic: z, p_value: p, correlated: true })
}

/// Compare two independent (unpaired) ROC curves using DeLong / Hanley–McNeil SEs.
#[must_use]
pub fn compare_roc_unpaired(roc_a: &RocResult, roc_b: &RocResult) -> RocComparisonResult {
    let se_diff = (roc_a.se * roc_a.se + roc_b.se * roc_b.se).sqrt();
    let auc_diff = roc_a.auc - roc_b.auc;
    let z = if se_diff > 0.0 { auc_diff / se_diff } else { 0.0 };
    let p = norm_two_sided_p(z);
    RocComparisonResult { auc_a: roc_a.auc, auc_b: roc_b.auc, auc_diff, se_diff, z_statistic: z, p_value: p, correlated: false }
}

// ============================================================================
// ResultSheet Builder
// ============================================================================

/// Execute a [`RocSpec`] against a 2-column `Column` or `MultipleVariables` [`DataSheet`]
/// (column 0 = Controls / Negatives, column 1 = Cases / Positives).
pub fn run_roc_analysis(sheet: &DataSheet, spec: &AnalysisSpec, roc_spec: &RocSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    let cols = match &sheet.archetype {
        TableArchetype::Column(_) | TableArchetype::MultipleVariables(_) => extract_numeric_columns(sheet, &spec.selected_data_sets)?,
        other => return Err(StatsError::IncompatibleTable { analysis: "ROC curve", actual: other.kind() }),
    };
    if cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "ROC groups", required: 2, actual: cols.len() });
    }
    let controls = &cols[0];
    let cases = &cols[1];
    let res = roc_curve_analysis(&controls.values, &cases.values, roc_spec.ci_method, confidence)?;

    push_scalar(&mut out, "Area under ROC curve (AUC)", None, res.auc);
    push_scalar(&mut out, "AUC Std. Error", None, res.se);
    push_scalar(&mut out, "P value (vs. 0.5)", None, res.p_value_vs_05);
    push_scalar(&mut out, "Optimal cutoff (Youden J)", None, res.optimal_cutoff.cutoff);
    push_scalar(&mut out, "Max Youden J", None, res.optimal_cutoff.youden_j);

    let summary_headers = vec![
        String::from("AUC"),
        String::from("SE"),
        format!("{}% CI", confidence.percent()),
        String::from("P value (vs. 0.5)"),
        String::from("Optimal cutoff"),
        String::from("Sensitivity"),
        String::from("Specificity"),
        String::from("Youden J"),
    ];
    let summary_row = vec![
        num_cell(res.auc),
        num_cell(res.se),
        ci_cell(res.ci.0, res.ci.1),
        p_cell(res.p_value_vs_05, p_fmt),
        num_cell(res.optimal_cutoff.cutoff),
        num_cell(res.optimal_cutoff.sensitivity),
        num_cell(res.optimal_cutoff.specificity),
        num_cell(res.optimal_cutoff.youden_j),
    ];
    out.tabs.push(build_tab(
        "roc-summary",
        "ROC curve summary",
        ResultTabKind::MainSummary,
        summary_headers,
        vec![format!("{} vs. {}", cases.label, controls.label)],
        vec![summary_row],
    ));

    let coord_headers = vec![
        String::from("Cutoff"),
        String::from("Sensitivity"),
        String::from("Specificity"),
        String::from("1 - Specificity"),
        String::from("Youden J"),
        String::from("PPV"),
        String::from("NPV"),
        String::from("LR+"),
    ];
    let mut coord_labels = Vec::with_capacity(res.points.len());
    let mut coord_rows = Vec::with_capacity(res.points.len());
    let mut curve_pts = Vec::with_capacity(res.points.len());
    for (idx, pt) in res.points.iter().enumerate() {
        coord_labels.push(format!("Pt {}", idx + 1));
        coord_rows.push(vec![
            num_cell(pt.cutoff),
            num_cell(pt.sensitivity),
            num_cell(pt.specificity),
            num_cell(pt.one_minus_specificity),
            num_cell(pt.youden_j),
            num_cell(pt.ppv),
            num_cell(pt.npv),
            opt_num_cell(pt.likelihood_ratio_pos),
        ]);
        curve_pts.push(CurveSamplePoint {
            x: pt.one_minus_specificity,
            y: pt.sensitivity,
            ci_lower: None,
            ci_upper: None,
            pred_lower: None,
            pred_upper: None,
        });
    }
    out.tabs.push(build_tab(
        "roc-coordinates",
        "Sensitivity & specificity by cutoff",
        ResultTabKind::CurveCoordinates,
        coord_headers,
        coord_labels,
        coord_rows,
    ));
    out.fitted_curves.push(FittedCurveOverlay {
        data_set_index: cases.index,
        equation_id: String::from("ROC"),
        converged: true,
        ambiguous: false,
        points: curve_pts,
    });

    out.checklist.push(checklist_item(
        "ROC discrimination direction",
        if res.auc >= 0.5 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("AUC = {:.4} ({} controls, {} cases)", res.auc, res.n_controls, res.n_cases)),
        if res.auc >= 0.5 {
            "DeLong / Hanley-McNeil SE computed assuming higher values correspond to the positive (case) group."
        } else {
            "AUC < 0.5 indicates lower values occur in the positive group or discrimination is below chance."
        },
    ));

    Ok(out)
}
