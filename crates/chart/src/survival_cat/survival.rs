//! Kaplan-Meier survival step curves, Greenwood CI ribbons, censor ticks, and number-at-risk tables.

use curvebook_doc::{
    AnalysisKind, ErrorBarMetric, GraphSheet, GraphSpec, GraphicalDataObject, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SheetId, SurvivalAnalysisSpec, SurvivalDisplayMetric, SurvivalObservation, SurvivalStatus, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_stats::survival::kaplan_meier_curve;
use curvebook_text::{RichText, parse_markup};

use super::shared::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, alpha_fill, draw_empty_open_frame, draw_legend_entries, layout_and_render_segmented_chrome,
    path_line, path_move, polygon_path, resolve_fresh_data_sheet_or_upstream, solid_stroke, text_metrics,
};
use crate::scales::compute_segment_tick_values;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::xy::{SegmentedAxis, format_point_value};

// ============================================================================
// 1. Kaplan-Meier Survival Graph (Step Curve + CI Ribbon + Censors + At-Risk)
// ============================================================================

struct KmCurveData {
    sheet_id: Option<SheetId>,
    col_index: usize,
    label: String,
    color: Rgba8,
    stroke_width_pt: f64,
    show_censored_ticks: bool,
    show_greenwood_ci: bool,
    ci_dashed: bool,
    display_metric: SurvivalDisplayMetric,
    staircase: bool,
    error_display: Option<ErrorBarMetric>,
    n_total: usize,
    steps: Vec<KmPointStep>,
    censored_times: Vec<(f64, Option<usize>, usize)>,
    observations: Vec<SurvivalObservation>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct KmPointStep {
    pub(super) time: f64,
    pub(super) survival: f64,
    greenwood_se: f64,
    ci_lower: f64,
    ci_upper: f64,
    n_at_risk_before: usize,
}

#[derive(Debug, Clone)]
struct KmResolvedStyle {
    color: Rgba8,
    stroke_width_pt: f64,
    show_censored_ticks: bool,
    show_greenwood_ci: bool,
    ci_dashed: bool,
    display_metric: SurvivalDisplayMetric,
    staircase: bool,
    error_display: Option<ErrorBarMetric>,
    legend_label: Option<String>,
}

pub(super) fn compile_survival(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let cohorts = extract_km_cohorts(graph, doc, &mut scene.warnings);
    if cohorts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_survival_data", "no survival cohort observations or result curves available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let y_auto_max =
        cohorts.iter().map(|c| km_cohort_y_scale(&graph.spec, c.display_metric)).filter(|v| v.is_finite()).fold(0.0_f64, f64::max).max(1.0);
    let max_time = cohorts
        .iter()
        .flat_map(|c| c.steps.iter().map(|s| s.time).chain(c.censored_times.iter().map(|&(t, _, _)| t)))
        .filter(|t| t.is_finite())
        .fold(1.0_f64, f64::max)
        .max(1.0);

    let show_nar = graph.spec.survival_number_at_risk && cohorts.iter().any(|c| c.n_total > 0 || !c.observations.is_empty());
    let extra_bottom = if show_nar { 20.0 + (cohorts.len() as f64) * 13.0 } else { 0.0 };
    let max_cohort_lbl_w = if show_nar { cohorts.iter().map(|c| text_metrics(&c.label, 8.0).width_pt + 10.0).fold(0.0_f64, f64::max) } else { 0.0 };

    let info_id = doc.paired_info_sheet_id(&graph.spec.primary_source_sheet_id);
    let legend_box_width_pt = crate::scales::auto_legend_box_width_pt(&graph.spec.legend, cohorts.iter().map(|c| c.label.as_str()), doc, info_id);
    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (0.0, max_time * 1.05),
            y_bounds: (0.0, y_auto_max),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig {
                extra_bottom_pt: extra_bottom,
                extra_left_pt: max_cohort_lbl_w.saturating_sub_nonneg(36.0),
                has_legend: !cohorts.is_empty(),
                legend_box_width_pt,
                ..PlotMarginConfig::default()
            },
        },
    );

    let r = scene.plot_rect_pt;
    let mut legend_entries = Vec::with_capacity(cohorts.len());
    for cohort in &cohorts {
        let cohort_scale = km_cohort_y_scale(&graph.spec, cohort.display_metric);
        draw_single_km_cohort(scene, cohort, &x_axis, &y_axis, cohort_scale);
        legend_entries.push(LegendEntry {
            label: cohort.label.clone(),
            color: cohort.color,
            marker: None,
            dash: Some(LineDashStyle::Solid),
            is_line: true,
        });
    }

    if show_nar {
        draw_number_at_risk_table(scene, r, &cohorts, &x_axis);
    }
    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

trait NonNegSub {
    fn saturating_sub_nonneg(self, rhs: f64) -> f64;
}
impl NonNegSub for f64 {
    fn saturating_sub_nonneg(self, rhs: f64) -> f64 {
        (self - rhs).max(0.0)
    }
}

fn survival_y_scale_factor(spec: &GraphSpec) -> f64 {
    let seg_max = spec.y_axis.segments.first().and_then(|s| s.max);
    let title_low = spec.y_axis.title.to_ascii_lowercase();
    if seg_max.is_some_and(|m| m > 1.5) || title_low.contains('%') || title_low.contains("percent") { 100.0 } else { 1.0 }
}

fn km_cohort_y_scale(spec: &GraphSpec, metric: SurvivalDisplayMetric) -> f64 {
    match metric {
        SurvivalDisplayMetric::FractionSurvival => 1.0,
        SurvivalDisplayMetric::PercentCumulativeEvent => 100.0,
        SurvivalDisplayMetric::PercentSurvival => survival_y_scale_factor(spec),
    }
}

fn transform_km_value(surv: f64, metric: SurvivalDisplayMetric, scale_pct: f64) -> f64 {
    match metric {
        SurvivalDisplayMetric::PercentCumulativeEvent => (1.0 - surv) * scale_pct,
        SurvivalDisplayMetric::PercentSurvival | SurvivalDisplayMetric::FractionSurvival => surv * scale_pct,
    }
}

fn transform_km_ci(ci_lo: f64, ci_hi: f64, metric: SurvivalDisplayMetric, scale_pct: f64) -> (f64, f64) {
    match metric {
        SurvivalDisplayMetric::PercentCumulativeEvent => ((1.0 - ci_hi) * scale_pct, (1.0 - ci_lo) * scale_pct),
        SurvivalDisplayMetric::PercentSurvival | SurvivalDisplayMetric::FractionSurvival => (ci_lo * scale_pct, ci_hi * scale_pct),
    }
}

fn is_survival_result_for(res: &ResultSheet, primary: &SheetId) -> bool {
    res.analysis.source_sheet_id == *primary
        && (matches!(res.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }))
            || (matches!(res.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }))
                && !res.fitted_curves.is_empty()))
}

fn extract_km_cohorts(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<KmCurveData> {
    let primary = &graph.spec.primary_source_sheet_id;
    if let Some(res) = doc.result_sheet(primary) {
        if matches!(res.status, ResultStatus::Failed { .. }) {
            warnings
                .push(ChartWarning::new("failed_result_sheet", format!("survival result sheet '{}' failed; omitted failed result curves", res.id)));
            return Vec::new();
        }
        if !matches!(res.status, ResultStatus::UpToDate) {
            warnings.push(ChartWarning::new(
                "pending_analysis_result",
                format!("survival result sheet '{}' is pending recalculation; omitted stale result curves", res.id),
            ));
            return Vec::new();
        }
        if !res.fitted_curves.is_empty() || matches!(res.analysis.kind, AnalysisKind::Survival(_)) {
            return build_km_from_result_sheet(graph, res);
        }
    }

    let km_result = doc
        .result_sheets
        .iter()
        .find(|r| {
            r.analysis.source_sheet_id == *primary
                && matches!(r.status, ResultStatus::UpToDate)
                && matches!(r.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }))
        })
        .or_else(|| {
            doc.result_sheets.iter().find(|r| {
                r.analysis.source_sheet_id == *primary
                    && matches!(r.status, ResultStatus::UpToDate)
                    && matches!(r.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }))
                    && !r.fitted_curves.is_empty()
            })
        });
    if km_result.is_none() {
        if let Some(failed) = doc.result_sheets.iter().find(|r| is_survival_result_for(r, primary) && matches!(r.status, ResultStatus::Failed { .. }))
        {
            warnings.push(ChartWarning::new(
                "failed_result_sheet",
                format!("survival result sheet '{}' failed; fell back to raw survival table", failed.id),
            ));
        } else if let Some(pending) =
            doc.result_sheets.iter().find(|r| is_survival_result_for(r, primary) && !matches!(r.status, ResultStatus::UpToDate))
        {
            warnings.push(ChartWarning::new(
                "pending_analysis_result",
                format!("survival result sheet '{}' is pending recalculation; omitted stale result curves", pending.id),
            ));
        }
    }

    if let Some(ds) = resolve_fresh_data_sheet_or_upstream(doc, primary, warnings)
        && let TableArchetype::Survival(surv) = &ds.archetype
    {
        return build_km_from_survival_table(graph, &ds.id, surv, km_result, warnings);
    }

    if let Some(res) = km_result {
        return build_km_from_result_sheet(graph, res);
    }

    Vec::new()
}

fn km_series_style(graph: &GraphSheet, data_set_idx: usize, fallback_idx: usize) -> KmResolvedStyle {
    let default_col = graph.spec.palette.color_at(fallback_idx);
    let series = graph.spec.series.iter().find(|s| s.data_set_index == data_set_idx).or_else(|| graph.spec.series.get(fallback_idx));
    if let Some(s) = series {
        if let GraphicalDataObject::KaplanMeierStep {
            line_color,
            stroke_width_pt,
            show_censored_ticks,
            show_greenwood_ci,
            ci_dashed,
            display_metric,
            staircase,
            error_display,
        } = s.gdo
        {
            return KmResolvedStyle {
                color: line_color,
                stroke_width_pt,
                show_censored_ticks,
                show_greenwood_ci,
                ci_dashed,
                display_metric,
                staircase,
                error_display,
                legend_label: s.legend_label.clone(),
            };
        }
        return KmResolvedStyle {
            color: default_col,
            stroke_width_pt: 1.8,
            show_censored_ticks: true,
            show_greenwood_ci: true,
            ci_dashed: false,
            display_metric: SurvivalDisplayMetric::default(),
            staircase: true,
            error_display: None,
            legend_label: s.legend_label.clone(),
        };
    }
    KmResolvedStyle {
        color: default_col,
        stroke_width_pt: 1.8,
        show_censored_ticks: true,
        show_greenwood_ci: true,
        ci_dashed: false,
        display_metric: SurvivalDisplayMetric::default(),
        staircase: true,
        error_display: None,
        legend_label: None,
    }
}

fn lookup_step_greenwood_se(computed: &[KmPointStep], time: f64, ci_lo: f64, ci_hi: f64) -> f64 {
    if let Some(exact) = computed.iter().find(|s| (s.time - time).abs() <= 1e-12) {
        return exact.greenwood_se;
    }
    ((ci_hi - ci_lo) / (2.0 * 1.959_963_984_540_054)).max(0.0)
}

fn build_km_from_survival_table(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    surv: &curvebook_doc::SurvivalTable,
    km_result: Option<&ResultSheet>,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<KmCurveData> {
    let indices: Vec<usize> = if graph.spec.series.is_empty() {
        (0..surv.cohorts.len()).collect()
    } else {
        graph.spec.series.iter().filter(|s| s.data_set_index < surv.cohorts.len()).map(|s| s.data_set_index).collect()
    };

    let mut out = Vec::with_capacity(indices.len());
    for (pos, &c_idx) in indices.iter().enumerate() {
        let Some(cohort_col) = surv.cohorts.get(c_idx) else { continue };
        let raw_obs = match surv.cohort_observations(c_idx) {
            Ok(obs) => obs,
            Err(err) => {
                warnings.push(ChartWarning::new(
                    "invalid_survival_cohort",
                    format!("survival cohort '{}' omitted due to invalid cell(s): {err}", cohort_col.title),
                ));
                continue;
            }
        };
        let obs = filter_finite_survival_observations(raw_obs);
        if obs.is_empty() {
            continue;
        }
        let style = km_series_style(graph, c_idx, pos);
        let label = style.legend_label.clone().unwrap_or_else(|| cohort_col.title.clone());
        let censored_times: Vec<(f64, Option<usize>, usize)> =
            obs.iter().filter(|o| o.status == SurvivalStatus::Censored).map(|o| (o.time, Some(o.row), c_idx)).collect();

        if let Some(res) = km_result
            && let Some(curve) = res.fitted_curves.iter().find(|fc| fc.data_set_index == c_idx)
            && !curve.points.is_empty()
        {
            let computed_steps = compute_km_steps_guarded(&label, &obs).map_or_else(Vec::new, |(_, s)| s);
            let steps: Vec<KmPointStep> = curve
                .points
                .iter()
                .filter(|p| p.x.is_finite() && p.y.is_finite())
                .map(|p| {
                    let ci_lo = p.ci_lower.filter(|v| v.is_finite()).unwrap_or(p.y);
                    let ci_hi = p.ci_upper.filter(|v| v.is_finite()).unwrap_or(p.y);
                    let se = lookup_step_greenwood_se(&computed_steps, p.x, ci_lo, ci_hi);
                    KmPointStep {
                        time: p.x,
                        survival: p.y,
                        greenwood_se: se,
                        ci_lower: ci_lo,
                        ci_upper: ci_hi,
                        n_at_risk_before: obs.iter().filter(|o| o.time >= p.x).count(),
                    }
                })
                .collect();
            if !steps.is_empty() {
                out.push(KmCurveData {
                    sheet_id: Some(sheet_id.clone()),
                    col_index: c_idx,
                    label,
                    color: style.color,
                    stroke_width_pt: style.stroke_width_pt,
                    show_censored_ticks: style.show_censored_ticks,
                    show_greenwood_ci: style.show_greenwood_ci,
                    ci_dashed: style.ci_dashed,
                    display_metric: style.display_metric,
                    staircase: style.staircase,
                    error_display: style.error_display,
                    n_total: obs.len(),
                    steps,
                    censored_times,
                    observations: obs,
                });
                continue;
            }
        }

        match compute_km_steps_result(&label, &obs) {
            Ok((n_total, steps)) => {
                out.push(KmCurveData {
                    sheet_id: Some(sheet_id.clone()),
                    col_index: c_idx,
                    label,
                    color: style.color,
                    stroke_width_pt: style.stroke_width_pt,
                    show_censored_ticks: style.show_censored_ticks,
                    show_greenwood_ci: style.show_greenwood_ci,
                    ci_dashed: style.ci_dashed,
                    display_metric: style.display_metric,
                    staircase: style.staircase,
                    error_display: style.error_display,
                    n_total,
                    steps,
                    censored_times,
                    observations: obs,
                });
            }
            Err(err) => {
                warnings.push(ChartWarning::new(
                    "invalid_survival_cohort",
                    format!("survival cohort '{}' omitted because Kaplan-Meier computation failed: {err}", cohort_col.title),
                ));
            }
        }
    }
    out
}

fn filter_finite_survival_observations(obs: Vec<curvebook_doc::SurvivalObservation>) -> Vec<curvebook_doc::SurvivalObservation> {
    obs.into_iter().filter(|o| o.time.is_finite()).collect()
}

fn compute_km_steps_result(label: &str, obs: &[curvebook_doc::SurvivalObservation]) -> Result<(usize, Vec<KmPointStep>), String> {
    let finite_obs: Vec<curvebook_doc::SurvivalObservation> = obs.iter().copied().filter(|o| o.time.is_finite()).collect();
    if finite_obs.is_empty() {
        return Err(String::from("no finite survival observations"));
    }
    let km = kaplan_meier_curve(label, &finite_obs, false, ConfidenceLevel::DEFAULT_95).map_err(|e| e.to_string())?;
    let mut steps = Vec::with_capacity(km.steps.len() + 1);
    steps.push(KmPointStep { time: 0.0, survival: 1.0, greenwood_se: 0.0, ci_lower: 1.0, ci_upper: 1.0, n_at_risk_before: km.n_total });
    for s in &km.steps {
        steps.push(KmPointStep {
            time: s.time,
            survival: s.survival,
            greenwood_se: s.greenwood_se,
            ci_lower: s.ci_lower,
            ci_upper: s.ci_upper,
            n_at_risk_before: s.n_at_risk,
        });
    }
    Ok((km.n_total, steps))
}

pub(super) fn compute_km_steps_guarded(label: &str, obs: &[curvebook_doc::SurvivalObservation]) -> Option<(usize, Vec<KmPointStep>)> {
    compute_km_steps_result(label, obs).ok()
}

fn result_curve_label(res: &ResultSheet, fc: &curvebook_doc::FittedCurveOverlay, idx: usize) -> String {
    match fc.equation_id.as_str() {
        "cox_baseline" => String::from("Baseline"),
        "cox_covariate" => {
            let param_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.id == "cox-ph");
            fc.data_set_index
                .checked_sub(1)
                .and_then(|cov_idx| param_tab.and_then(|t| t.row_labels.get(cov_idx)))
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Covariate {}", fc.data_set_index))
        }
        _ => res
            .tabs
            .iter()
            .find(|t| t.kind == ResultTabKind::MainSummary)
            .and_then(|t| t.row_labels.get(idx))
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Cohort {}", fc.data_set_index + 1)),
    }
}

fn build_km_from_result_sheet(graph: &GraphSheet, res: &ResultSheet) -> Vec<KmCurveData> {
    let nar_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::NumberAtRisk);
    let mut out = Vec::with_capacity(res.fitted_curves.len());
    for (idx, fc) in res.fitted_curves.iter().enumerate() {
        let style = km_series_style(graph, fc.data_set_index, idx);
        let canonical_label = result_curve_label(res, fc, idx);
        let label = style.legend_label.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| canonical_label.clone());
        let steps: Vec<KmPointStep> = fc
            .points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| {
                let ci_lo = p.ci_lower.filter(|v| v.is_finite()).unwrap_or(p.y);
                let ci_hi = p.ci_upper.filter(|v| v.is_finite()).unwrap_or(p.y);
                let se = ((ci_hi - ci_lo) / (2.0 * 1.959_963_984_540_054)).max(0.0);
                KmPointStep {
                    time: p.x,
                    survival: p.y,
                    greenwood_se: se,
                    ci_lower: ci_lo,
                    ci_upper: ci_hi,
                    n_at_risk_before: nar_at_time_from_tab(nar_tab, &canonical_label, p.x).unwrap_or(0),
                }
            })
            .collect();
        if steps.is_empty() {
            continue;
        }
        let has_ci = fc
            .points
            .iter()
            .any(|p| p.x.is_finite() && p.y.is_finite() && p.ci_lower.is_some_and(f64::is_finite) && p.ci_upper.is_some_and(f64::is_finite));
        let n_total = steps.first().map_or(0, |s| s.n_at_risk_before);
        let mut censored_times = Vec::new();
        if let Some(tab) = nar_tab {
            extract_censored_from_nar_tab(tab, &canonical_label, fc.data_set_index, &mut censored_times);
        }
        out.push(KmCurveData {
            sheet_id: None,
            col_index: fc.data_set_index,
            label,
            color: style.color,
            stroke_width_pt: style.stroke_width_pt,
            show_censored_ticks: style.show_censored_ticks,
            show_greenwood_ci: style.show_greenwood_ci && has_ci,
            ci_dashed: style.ci_dashed,
            display_metric: style.display_metric,
            staircase: style.staircase,
            error_display: style.error_display,
            n_total,
            steps,
            censored_times,
            observations: Vec::new(),
        });
    }
    out
}

fn nar_at_time_from_tab(nar_tab: Option<&ResultTab>, cohort_label: &str, t: f64) -> Option<usize> {
    let tab = nar_tab?;
    for row in &tab.rows {
        let Some(ResultCell::Text(lbl)) = row.first() else { continue };
        if lbl != cohort_label {
            continue;
        }
        let Some(ResultCell::Number { value: row_t, .. }) = row.get(1) else { continue };
        let Some(ResultCell::Number { value: risk, .. }) = row.get(2) else { continue };
        if row_t.is_finite() && *row_t >= t - 1e-9 && risk.is_finite() && *risk >= 0.0 {
            return Some(*risk as usize);
        }
    }
    None
}

fn extract_censored_from_nar_tab(tab: &ResultTab, cohort_label: &str, col_idx: usize, out: &mut Vec<(f64, Option<usize>, usize)>) {
    // Each KM schedule row i records (t_i, n_i = at_risk_before, d_i = events at t_i)
    // with recurrence n_{i+1} = n_i - d_i - c_i (and n_m = 0 after the final row m - 1).
    // Therefore c_i = n_i - d_i - n_{i+1} subjects are censored at time t_i.
    let mut rows: Vec<(f64, usize, usize)> = Vec::new();
    for row in &tab.rows {
        let Some(ResultCell::Text(lbl)) = row.first() else { continue };
        if lbl != cohort_label {
            continue;
        }
        let Some(ResultCell::Number { value: row_t, .. }) = row.get(1) else { continue };
        let Some(ResultCell::Number { value: risk, .. }) = row.get(2) else { continue };
        let Some(ResultCell::Number { value: events, .. }) = row.get(3) else { continue };
        if !row_t.is_finite() || !risk.is_finite() || !events.is_finite() || *risk < 0.0 || *events < 0.0 {
            continue;
        }
        rows.push((*row_t, *risk as usize, *events as usize));
    }
    for (i, &(t_i, n_i, d_i)) in rows.iter().enumerate() {
        let next_n = rows.get(i + 1).map_or(0, |&(_, n_next, _)| n_next);
        if n_i > d_i.saturating_add(next_n) {
            out.push((t_i, None, col_idx));
        }
    }
}

fn draw_single_km_cohort(scene: &mut ChartScene, cohort: &KmCurveData, x_axis: &SegmentedAxis, y_axis: &SegmentedAxis, scale_pct: f64) {
    let r = scene.plot_rect_pt;
    if cohort.steps.is_empty() {
        return;
    }

    let show_ci_band = cohort.show_greenwood_ci || matches!(cohort.error_display, Some(ErrorBarMetric::ConfidenceInterval95));
    if show_ci_band {
        draw_km_ci_band(scene, r, cohort, x_axis, y_axis, scale_pct);
    }

    let max_cens = cohort.censored_times.iter().map(|&(t, _, _)| t).filter(|t| t.is_finite()).max_by(f64::total_cmp);
    let mut step_path = Path2D::new();
    for seg_x in &x_axis.segments {
        let seg_lo = seg_x.domain_min.min(seg_x.domain_max);
        let seg_hi = seg_x.domain_min.max(seg_x.domain_max);
        let (Some(first_all), Some(last_all)) = (cohort.steps.first(), cohort.steps.last()) else {
            continue;
        };
        let tail_hi = max_cens.map_or(last_all.time, |mc| mc.max(last_all.time));
        if tail_hi < seg_lo || first_all.time > seg_hi {
            continue;
        }

        let mut started = false;
        let mut prev_ya: Option<f64> = None;
        if first_all.time < seg_lo {
            let y0_val = transform_km_value(eval_km_at(&cohort.steps, seg_lo, cohort.staircase), cohort.display_metric, scale_pct);
            if let (Some(x0), Some((_, y0))) = (seg_x.map_strict(seg_lo), y_axis.map_value(y0_val)) {
                path_move(&mut step_path, PointPt::new(x0, y0));
                started = true;
                prev_ya = Some(y0);
            }
        }
        for st in &cohort.steps {
            if st.time < seg_lo - 1e-12 || st.time > seg_hi + 1e-12 {
                continue;
            }
            let y_val = transform_km_value(st.survival, cohort.display_metric, scale_pct);
            let (Some(xb), Some((_, yb))) = (seg_x.map_strict(st.time), y_axis.map_value(y_val)) else {
                prev_ya = None;
                started = false;
                continue;
            };
            if !started {
                path_move(&mut step_path, PointPt::new(xb, yb));
                started = true;
            } else {
                if cohort.staircase
                    && let Some(ya) = prev_ya
                {
                    path_line(&mut step_path, PointPt::new(xb, ya));
                }
                path_line(&mut step_path, PointPt::new(xb, yb));
            }
            prev_ya = Some(yb);
        }
        if tail_hi > last_all.time && last_all.time <= seg_hi {
            let last_val = transform_km_value(last_all.survival, cohort.display_metric, scale_pct);
            if let (Some(xe), Some((_, yl))) = (seg_x.map_strict(tail_hi.min(seg_hi)), y_axis.map_value(last_val)) {
                if started {
                    path_line(&mut step_path, PointPt::new(xe, yl));
                } else if let Some(xs) = seg_x.map_strict(seg_lo.max(last_all.time)) {
                    path_move(&mut step_path, PointPt::new(xs, yl));
                    path_line(&mut step_path, PointPt::new(xe, yl));
                }
            }
        }
    }

    scene.primitives.push(ScenePrimitive::Path {
        path: step_path,
        fill: None,
        stroke: Some(solid_stroke(cohort.color, cohort.stroke_width_pt, LineDashStyle::Solid)),
        clip: Some(r),
    });

    if matches!(cohort.error_display, Some(ErrorBarMetric::Sem)) {
        let se_stroke = solid_stroke(cohort.color, (cohort.stroke_width_pt * 0.75).max(0.85), LineDashStyle::Solid);
        for st in &cohort.steps {
            if st.time <= 0.0 || !st.greenwood_se.is_finite() || st.greenwood_se <= 0.0 {
                continue;
            }
            let lo_raw = (st.survival - st.greenwood_se).clamp(0.0, 1.0);
            let hi_raw = (st.survival + st.greenwood_se).clamp(0.0, 1.0);
            let (lo_v, hi_v) = transform_km_ci(lo_raw, hi_raw, cohort.display_metric, scale_pct);
            if let (Some((_, xp)), Some((_, yp_lo)), Some((_, yp_hi))) = (x_axis.map_value(st.time), y_axis.map_value(lo_v), y_axis.map_value(hi_v)) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xp, yp_lo), PointPt::new(xp, yp_hi)),
                    fill: None,
                    stroke: Some(se_stroke),
                    clip: Some(r),
                });
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xp - 2.0, yp_lo), PointPt::new(xp + 2.0, yp_lo)),
                    fill: None,
                    stroke: Some(se_stroke),
                    clip: Some(r),
                });
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xp - 2.0, yp_hi), PointPt::new(xp + 2.0, yp_hi)),
                    fill: None,
                    stroke: Some(se_stroke),
                    clip: Some(r),
                });
            }
        }
    }

    if let Some(sheet_id) = &cohort.sheet_id {
        for obs in &cohort.observations {
            if obs.status != SurvivalStatus::Event {
                continue;
            }
            let s_val = transform_km_value(eval_km_at(&cohort.steps, obs.time, cohort.staircase), cohort.display_metric, scale_pct);
            if let (Some((_, xe)), Some((_, ye))) = (x_axis.map_value(obs.time), y_axis.map_value(s_val)) {
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt: PointPt::new(xe, ye),
                    radius_pt: 4.0,
                    source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: obs.row, col: cohort.col_index, subcol: 0 },
                    series_label: cohort.label.clone(),
                    value_summary: format!("t={}, S={}", format_point_value(obs.time), format_point_value(s_val)),
                });
            }
        }
    }

    if cohort.show_censored_ticks {
        let censor_stroke = solid_stroke(cohort.color, (cohort.stroke_width_pt * 0.85).max(1.0), LineDashStyle::Solid);
        for &(tc, row_opt, col_idx) in &cohort.censored_times {
            let s_val = transform_km_value(eval_km_at(&cohort.steps, tc, cohort.staircase), cohort.display_metric, scale_pct);
            if let (Some((_, xc)), Some((_, yc))) = (x_axis.map_value(tc), y_axis.map_value(s_val)) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xc, yc - 3.5), PointPt::new(xc, yc + 3.5)),
                    fill: None,
                    stroke: Some(censor_stroke),
                    clip: Some(r),
                });
                if let (Some(sheet_id), Some(row)) = (&cohort.sheet_id, row_opt) {
                    scene.plotted_points.push(PlottedPointMeta {
                        center_pt: PointPt::new(xc, yc),
                        radius_pt: 4.5,
                        source_cell: SourceCellId { sheet_id: sheet_id.clone(), row, col: col_idx, subcol: 0 },
                        series_label: cohort.label.clone(),
                        value_summary: format!("t={}, S={}", format_point_value(tc), format_point_value(s_val)),
                    });
                }
            }
        }
    }
}

fn draw_km_ci_band(scene: &mut ChartScene, r: RectPt, cohort: &KmCurveData, x_axis: &SegmentedAxis, y_axis: &SegmentedAxis, scale_pct: f64) {
    for seg_x in &x_axis.segments {
        let seg_lo = seg_x.domain_min.min(seg_x.domain_max);
        let seg_hi = seg_x.domain_min.max(seg_x.domain_max);
        let mut upper_pts = Vec::new();
        let mut lower_pts = Vec::new();
        if cohort.staircase {
            for win in cohort.steps.windows(2) {
                let (Some(a), Some(b)) = (win.first(), win.get(1)) else { continue };
                if b.time < seg_lo - 1e-12 || a.time > seg_hi + 1e-12 {
                    continue;
                }
                let t0 = a.time.max(seg_lo);
                let t1 = b.time.min(seg_hi);
                let (Some(xa), Some(xb)) = (seg_x.map_strict(t0), seg_x.map_strict(t1)) else {
                    continue;
                };
                let (lo_v, hi_v) = transform_km_ci(a.ci_lower, a.ci_upper, cohort.display_metric, scale_pct);
                if let Some((_, y_hi)) = y_axis.map_value(hi_v) {
                    upper_pts.push((xa, y_hi));
                    upper_pts.push((xb, y_hi));
                }
                if let Some((_, y_lo)) = y_axis.map_value(lo_v) {
                    lower_pts.push((xa, y_lo));
                    lower_pts.push((xb, y_lo));
                }
            }
        } else {
            for st in &cohort.steps {
                if st.time < seg_lo - 1e-12 || st.time > seg_hi + 1e-12 {
                    continue;
                }
                let Some(xs) = seg_x.map_strict(st.time) else { continue };
                let (lo_v, hi_v) = transform_km_ci(st.ci_lower, st.ci_upper, cohort.display_metric, scale_pct);
                if let (Some((_, y_hi)), Some((_, y_lo))) = (y_axis.map_value(hi_v), y_axis.map_value(lo_v)) {
                    upper_pts.push((xs, y_hi));
                    lower_pts.push((xs, y_lo));
                }
            }
        }
        if upper_pts.is_empty() || lower_pts.is_empty() {
            continue;
        }

        if cohort.ci_dashed {
            let mut up_p = Path2D::new();
            if let Some(&(x0, y0)) = upper_pts.first() {
                path_move(&mut up_p, PointPt::new(x0, y0));
                for &(x, y) in upper_pts.iter().skip(1) {
                    path_line(&mut up_p, PointPt::new(x, y));
                }
            }
            let mut lo_p = Path2D::new();
            if let Some(&(x0, y0)) = lower_pts.first() {
                path_move(&mut lo_p, PointPt::new(x0, y0));
                for &(x, y) in lower_pts.iter().skip(1) {
                    path_line(&mut lo_p, PointPt::new(x, y));
                }
            }
            let dash_stroke = solid_stroke(cohort.color, 1.0, LineDashStyle::Dashed);
            scene.primitives.push(ScenePrimitive::Path { path: up_p, fill: None, stroke: Some(dash_stroke), clip: Some(r) });
            scene.primitives.push(ScenePrimitive::Path { path: lo_p, fill: None, stroke: Some(dash_stroke), clip: Some(r) });
        } else {
            let mut poly = Vec::with_capacity(upper_pts.len() + lower_pts.len());
            poly.extend(upper_pts);
            poly.extend(lower_pts.into_iter().rev());
            scene.primitives.push(ScenePrimitive::Path {
                path: polygon_path(&poly),
                fill: Some(alpha_fill(cohort.color, 46)),
                stroke: None,
                clip: Some(r),
            });
        }
    }
}

fn eval_km_at(steps: &[KmPointStep], t: f64, staircase: bool) -> f64 {
    if !staircase && steps.len() >= 2 {
        let idx = steps.partition_point(|s| s.time <= t);
        if idx == 0 {
            return steps[0].survival;
        }
        if idx >= steps.len() {
            return steps[steps.len() - 1].survival;
        }
        let a = steps[idx - 1];
        let b = steps[idx];
        let span = b.time - a.time;
        if span.abs() > 1e-12 {
            let w = ((t - a.time) / span).clamp(0.0, 1.0);
            return a.survival + w * (b.survival - a.survival);
        }
        return b.survival;
    }
    let mut current = steps.first().map_or(1.0, |s| s.survival);
    for s in steps {
        if s.time <= t + 1e-12 {
            current = s.survival;
        } else {
            break;
        }
    }
    current
}

fn draw_number_at_risk_table(scene: &mut ChartScene, r: RectPt, cohorts: &[KmCurveData], x_axis: &SegmentedAxis) {
    let top_y = r.bottom() + 40.0;
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>Number at risk</b>"),
        anchor_pt: PointPt::new(r.x - 6.0, top_y),
        anchor: TextAnchor::End,
        font_size_pt: 8.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let mut dummy_warnings = Vec::new();
    let mut tick_points = Vec::new();
    for seg in &x_axis.segments {
        let (majors, _) = compute_segment_tick_values(seg, None, false, &mut dummy_warnings);
        for t_val in majors {
            if let Some(x_pt) = seg.map_strict(t_val)
                && x_pt >= r.x - 2.0
                && x_pt <= r.right() + 2.0
            {
                tick_points.push((t_val, x_pt));
            }
        }
    }

    for (idx, cohort) in cohorts.iter().enumerate() {
        let row_y = top_y + 13.0 + (idx as f64) * 12.5;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(cohort.label.clone()),
            anchor_pt: PointPt::new(r.x - 6.0, row_y),
            anchor: TextAnchor::End,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: cohort.color,
        }));
        for &(t_val, x_pt) in &tick_points {
            let n_risk = if !cohort.observations.is_empty() {
                cohort.observations.iter().filter(|o| o.time >= t_val - 1e-9).count()
            } else if t_val <= 1e-9 {
                cohort.n_total
            } else {
                cohort.steps.iter().find(|s| s.time >= t_val - 1e-9).map_or(0, |s| s.n_at_risk_before)
            };
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(format!("{n_risk}")),
                anchor_pt: PointPt::new(x_pt, row_y),
                anchor: TextAnchor::Middle,
                font_size_pt: 8.0,
                rotation_deg: 0.0,
                color: Rgba8::rgb(25, 25, 25),
            }));
        }
    }
}
