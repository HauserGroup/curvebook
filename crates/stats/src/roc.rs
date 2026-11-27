//! Receiver Operating Characteristic (ROC) curve analysis: empirical ROC thresholds,
//! trapezoidal AUC, DeLong (1988) exact structural-components SE/CI and two-curve comparison
//! (correlated & uncorrelated), Hanley–McNeil SE, Youden's *J* optimal cutoff, PPV/NPV/LR+, and
//! `ResultSheet` builders.
//!
//! References:
//! - DeLong, E. R., DeLong, D. M., & Clarke-Pearson, D. L. (1988). Comparing the areas under two
//!   or more correlated receiver operating characteristic curves: A nonparametric approach.
//!   *Biometrics*, 44(3), 837-845. `doi:10.2307/2531595`.
//! - Hanley, J. A., & McNeil, B. J. (1982). The meaning and use of the area under a receiver
//!   operating characteristic (ROC) curve. *Radiology*, 143(1), 29-36.
//!   `doi:10.1148/radiology.143.1.7063747`.
//! - Youden, W. J. (1950). Index for rating diagnostic tests. *Cancer*, 3(1), 32-35.
//!   `doi:10.1002/1097-0142(1950)3:1<32::aid-cncr2820030106>3.0.co;2-3`.

pub use curvebook_doc::RocCiMethod;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay, ResultSheet, ResultTabKind, RocSpec,
    TableArchetype, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{norm_two_sided_isf, norm_two_sided_p};
use serde::{Deserialize, Serialize};

use crate::common::{
    append_ignored_text_checklist_and_footnote, build_tab, checklist_item, ci_cell, enforce_summary_mode_matrix, extract_numeric_columns,
    init_result_sheet, num_cell, opt_num_cell, p_cell_at_alpha, push_scalar, sanitize_result_sheet_floats, try_resolve_selected_indices,
    validate_finite_slice,
};
use crate::error::StatsError;

// ============================================================================
// Typed Outputs
// ============================================================================

/// Single threshold operating point on an empirical ROC curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocOperatingPoint {
    /// Classification threshold `t` (observations with test score `>= t` are predicted positive; initial sentinel uses `max + 1`).
    pub cutoff: f64,
    /// True-positive fraction `TP / n_cases` in `[0, 1]`.
    pub sensitivity: f64,
    /// True-negative fraction `TN / n_controls` in `[0, 1]`.
    pub specificity: f64,
    /// False-positive fraction `1 - specificity = FP / n_controls` in `[0, 1]`.
    pub one_minus_specificity: f64,
    /// Youden's *J* index `sensitivity + specificity - 1` in `[-1, 1]` (Youden 1950).
    pub youden_j: f64,
    /// Positive predictive value `TP / (TP + FP)` in `[0, 1]` at the sample prevalence (`1.0` when `TP + FP == 0`).
    pub ppv: f64,
    /// Negative predictive value `TN / (TN + FN)` in `[0, 1]` at the sample prevalence (`1.0` when `TN + FN == 0`).
    pub npv: f64,
    /// Positive likelihood ratio `sensitivity / (1 - specificity)` (`None` when `specificity == 1.0`).
    pub likelihood_ratio_pos: Option<f64>,
    /// Negative likelihood ratio `(1 - sensitivity) / specificity` (`None` when `specificity == 0.0`).
    pub likelihood_ratio_neg: Option<f64>,
}

/// Complete ROC curve analysis for `cases` vs. `controls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocResult {
    /// Number of reference negative (`control`) observations.
    pub n_controls: usize,
    /// Number of positive (`case`) observations.
    pub n_cases: usize,
    /// Empirical trapezoidal area under the ROC curve in `[0, 1]` (equivalent to the Mann–Whitney *U* concordance statistic).
    pub auc: f64,
    /// Standard error of `auc` via DeLong, DeLong & Clarke-Pearson (1988) or Hanley & McNeil (1982) as selected.
    pub se: f64,
    /// Two-sided confidence interval `(lower, upper)` in `[0, 1]` at configured `ConfidenceLevel` (normal Wald on `auc` when `se > 0`, falling back to boundary-aware Wilson score on `min(n_controls, n_cases)` under separation).
    pub ci: (f64, f64),
    /// Two-sided *P* value testing $H_0: \text{AUC} = 0.5$ (`norm_two_sided_p((auc - 0.5) / se)` when `se > 0`, else two-sided Mann–Whitney *U* / exact rank-sum tail under complete separation).
    pub p_value_vs_05: f64,
    /// Operating point maximizing Youden's *J* index (`sensitivity + specificity - 1`) across candidate thresholds.
    pub optimal_cutoff: RocOperatingPoint,
    /// Full empirical staircase operating points ordered descending in `cutoff` from `(0, 0)` to `(1, 1)`.
    pub points: Vec<RocOperatingPoint>,
    /// DeLong (1988) structural placement vector `V_{10}` indexed 1-to-1 over `cases` for paired curve comparison.
    pub delong_v10: Vec<f64>,
    /// DeLong (1988) structural placement vector `V_{01}` indexed 1-to-1 over `controls` for paired curve comparison.
    pub delong_v01: Vec<f64>,
}

/// DeLong two-curve AUC comparison result (DeLong, DeLong & Clarke-Pearson 1988).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RocComparisonResult {
    /// Trapezoidal AUC of the first ROC curve (`roc_a`) in `[0, 1]`.
    pub auc_a: f64,
    /// Trapezoidal AUC of the second ROC curve (`roc_b`) in `[0, 1]`.
    pub auc_b: f64,
    /// Signed difference `auc_a - auc_b` in `[-1, 1]`.
    pub auc_diff: f64,
    /// Standard error of `auc_a - auc_b` accounting for paired covariance when `correlated == true`, or pooled independent variance when `false`.
    pub se_diff: f64,
    /// Standard normal *z*-statistic `(auc_a - auc_b) / se_diff` (`0.0` when `se_diff == 0`).
    pub z_statistic: f64,
    /// Two-sided normal *P* value testing $H_0: \text{AUC}_A = \text{AUC}_B$.
    pub p_value: f64,
    /// True for correlated paired DeLong comparisons sharing identical case/control cohorts; false for independent unpaired comparison.
    pub correlated: bool,
}

// ============================================================================
// Core Empirical ROC & DeLong (1988) Structural Components
// ============================================================================

/// Compute empirical ROC curve, trapezoidal AUC, DeLong (1988) exact SE/CI (or Hanley–McNeil),
/// and optimal Youden *J* cutoff assuming higher test values indicate `cases`.
///
/// DeLong's structural-components variance (`RocCiMethod::DeLong`) requires `controls.len() >= 2`
/// and `cases.len() >= 2` because the component sample variances divide by `n - 1` (DeLong et al.
/// 1988); single-subject groups return [`StatsError::InsufficientSampleSize`]. Under
/// `RocCiMethod::HanleyMcNeil`, single-subject groups use the Hanley & McNeil (1982) SE together
/// with a boundary-aware Wilson score CI and exact Mann–Whitney *U* rank-sum *P* value.
pub fn roc_curve_analysis(controls: &[f64], cases: &[f64], ci_method: RocCiMethod, confidence: ConfidenceLevel) -> Result<RocResult, StatsError> {
    validate_finite_slice(controls)?;
    validate_finite_slice(cases)?;
    let n0 = controls.len();
    let n1 = cases.len();
    if matches!(ci_method, RocCiMethod::DeLong) && (n0 < 2 || n1 < 2) {
        return Err(StatsError::InsufficientSampleSize { context: "DeLong ROC variance", required: 2, actual: n0.min(n1) });
    }
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
            let s10 = v10.iter().map(|&v| (v - auc).powi(2)).sum::<f64>() / ((n1 - 1) as f64);
            let s01 = v01.iter().map(|&v| (v - auc).powi(2)).sum::<f64>() / ((n0 - 1) as f64);
            ((s10 / n1f) + (s01 / n0f)).max(0.0).sqrt()
        }
        RocCiMethod::HanleyMcNeil => {
            let q1 = auc / (2.0 - auc);
            let q2 = (2.0 * auc * auc) / (1.0 + auc);
            let num = auc * (1.0 - auc) + (n1f - 1.0) * (q1 - auc * auc) + (n0f - 1.0) * (q2 - auc * auc);
            (num / (n1f * n0f)).max(0.0).sqrt()
        }
    };

    let single_subject_group = n0 < 2 || n1 < 2;
    let z_crit = norm_two_sided_isf(confidence.alpha())?;
    let ci = if se > 0.0 && !single_subject_group {
        ((auc - z_crit * se).clamp(0.0, 1.0), (auc + z_crit * se).clamp(0.0, 1.0))
    } else {
        // Complete separation, degenerate zero-variance ROC curve, or n == 1 small-sample group:
        // avoid collapsing CI or invalid normal Wald on n == 1 by using a boundary-aware Wilson score interval.
        let n_eff = (n0.min(n1) as f64).max(1.0);
        let z2 = z_crit * z_crit;
        let denom = 1.0 + z2 / n_eff;
        let center = (auc + z2 / (2.0 * n_eff)) / denom;
        let half = (z_crit / denom) * ((auc * (1.0 - auc) / n_eff) + z2 / (4.0 * n_eff * n_eff)).sqrt();
        ((center - half).clamp(0.0, 1.0), (center + half).clamp(0.0, 1.0))
    };
    let p_value_vs_05 = if se > 0.0 && !single_subject_group {
        norm_two_sided_p((auc - 0.5) / se)
    } else if (auc - 0.5).abs() <= 1e-12 {
        1.0
    } else {
        // Exact / tie-adjusted two-sided Mann-Whitney P against H0: AUC = 0.5 under separation or single-subject group.
        match crate::two_group::mann_whitney_u(controls, cases, true, confidence) {
            Ok(mw) => mw.p_value,
            Err(_) => {
                let total = n0 + n1;
                if single_subject_group { (2.0 / (total as f64)).clamp(0.0, 1.0) } else { 1.0 }
            }
        }
    };

    let optimal_cutoff = points.iter().skip(1).max_by(|a, b| a.youden_j.total_cmp(&b.youden_j)).cloned().unwrap_or_else(|| points[0].clone());

    Ok(RocResult { n_controls: n0, n_cases: n1, auc, se, ci, p_value_vs_05, optimal_cutoff, points, delong_v10: v10, delong_v01: v01 })
}

/// Compare two correlated ROC curves evaluated on the **same** controls and cases (DeLong, DeLong
/// & Clarke-Pearson 1988, *Biometrics* 44:837-845).
pub fn compare_roc_delong_paired(roc_a: &RocResult, roc_b: &RocResult) -> Result<RocComparisonResult, StatsError> {
    if roc_a.n_cases < 2 || roc_a.n_controls < 2 || roc_b.n_cases < 2 || roc_b.n_controls < 2 {
        return Err(StatsError::InsufficientSampleSize {
            context: "DeLong paired ROC comparison",
            required: 2,
            actual: roc_a.n_cases.min(roc_a.n_controls).min(roc_b.n_cases).min(roc_b.n_controls),
        });
    }
    if roc_a.n_cases != roc_b.n_cases {
        return Err(StatsError::DimensionMismatch { expected: roc_a.n_cases, actual: roc_b.n_cases });
    }
    if roc_a.n_controls != roc_b.n_controls {
        return Err(StatsError::DimensionMismatch { expected: roc_a.n_controls, actual: roc_b.n_controls });
    }
    if roc_a.delong_v10.len() != roc_a.n_cases {
        return Err(StatsError::DimensionMismatch { expected: roc_a.n_cases, actual: roc_a.delong_v10.len() });
    }
    if roc_b.delong_v10.len() != roc_b.n_cases {
        return Err(StatsError::DimensionMismatch { expected: roc_b.n_cases, actual: roc_b.delong_v10.len() });
    }
    if roc_a.delong_v01.len() != roc_a.n_controls {
        return Err(StatsError::DimensionMismatch { expected: roc_a.n_controls, actual: roc_a.delong_v01.len() });
    }
    if roc_b.delong_v01.len() != roc_b.n_controls {
        return Err(StatsError::DimensionMismatch { expected: roc_b.n_controls, actual: roc_b.delong_v01.len() });
    }
    if !roc_a.auc.is_finite() || !roc_b.auc.is_finite() {
        let bad = if !roc_a.auc.is_finite() { roc_a.auc } else { roc_b.auc };
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "roc_auc", value: bad }));
    }

    let n1 = roc_a.n_cases as f64;
    let n0 = roc_a.n_controls as f64;

    let mut cov10 = 0.0_f64;
    for i in 0..roc_a.n_cases {
        let va = *roc_a.delong_v10.get(i).ok_or(StatsError::DimensionMismatch { expected: roc_a.n_cases, actual: roc_a.delong_v10.len() })?;
        let vb = *roc_b.delong_v10.get(i).ok_or(StatsError::DimensionMismatch { expected: roc_b.n_cases, actual: roc_b.delong_v10.len() })?;
        if !va.is_finite() || !vb.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError {
                name: "delong_v10",
                value: if !va.is_finite() { va } else { vb },
            }));
        }
        let da = va - roc_a.auc;
        let db = vb - roc_b.auc;
        cov10 += (da - db) * (da - db);
    }
    cov10 /= n1 - 1.0;

    let mut cov01 = 0.0_f64;
    for j in 0..roc_a.n_controls {
        let va = *roc_a.delong_v01.get(j).ok_or(StatsError::DimensionMismatch { expected: roc_a.n_controls, actual: roc_a.delong_v01.len() })?;
        let vb = *roc_b.delong_v01.get(j).ok_or(StatsError::DimensionMismatch { expected: roc_b.n_controls, actual: roc_b.delong_v01.len() })?;
        if !va.is_finite() || !vb.is_finite() {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError {
                name: "delong_v01",
                value: if !va.is_finite() { va } else { vb },
            }));
        }
        let da = va - roc_a.auc;
        let db = vb - roc_b.auc;
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
    enforce_summary_mode_matrix(sheet, &AnalysisKind::RocCurve(*roc_spec))?;
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    let cols = match &sheet.archetype {
        TableArchetype::Column(_) => {
            let extracted = extract_numeric_columns(sheet, &spec.selected_data_sets)?;
            if !spec.selected_data_sets.is_empty() && extracted.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: extracted.len() as f64 });
            }
            extracted
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?;
            let numeric_indices: Vec<usize> =
                indices.into_iter().filter(|&i| !matches!(mv.variables[i].role, VariableRole::Categorical | VariableRole::Label)).collect();
            if !spec.selected_data_sets.is_empty() && numeric_indices.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: numeric_indices.len() as f64 });
            }
            if numeric_indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "ROC groups", required: 2, actual: numeric_indices.len() });
            }
            extract_numeric_columns(sheet, &numeric_indices[..2])?
        }
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
        p_cell_at_alpha(res.p_value_vs_05, p_fmt, confidence.alpha()),
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
    if res.n_controls < 2 || res.n_cases < 2 {
        out.checklist.push(checklist_item(
            "ROC small-sample inference",
            ChecklistStatus::Warn,
            Some(format!("{} controls, {} cases", res.n_controls, res.n_cases)),
            "Single-subject group prevents DeLong variance estimation and normal Wald inference; Hanley-McNeil SE, Wilson score CI, and exact Mann-Whitney U P value are reported.",
        ));
    }

    let ignored: Vec<(String, usize)> =
        [controls, cases].into_iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect();
    append_ignored_text_checklist_and_footnote(&mut out, "ROC curve", &ignored);
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}
