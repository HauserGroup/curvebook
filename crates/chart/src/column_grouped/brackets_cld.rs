//! Pairwise significance bracket resolution/stacking and Compact Letter Display (CLD) overlays.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, BracketLabelMode, BracketStyle, GraphSheet, GraphSpec, GroupedArrangement, PairwiseBracketSpec, PairwiseComparisonRecord,
    PairwiseDisplayMode, ProjectDocument, ResultStatus,
};
use curvebook_palette::Rgba8;
use curvebook_stats::cld::{CldOptions, compute_cld_from_result_sheet};
use curvebook_text::{LocaleNumberPolicy, RichText, format_pvalue_label, measure_text_pt};

use super::{CategorySlotItem, ExtractedKind, RenderedSlotBound, gdo_summary_measure, raw_values};
use crate::scene::{ChartScene, ChartWarning, Path2D, PathCmd, PointPt, PositionedText, ScenePrimitive, StrokePaint, TextAnchor};
use crate::xy::{compute_central_tendency, expand_doc_text, primary_info_sheet_id};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCldSlot {
    pub(crate) group_key: usize,
    pub(crate) label: String,
    pub(crate) letters: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCldLabels {
    pub(crate) font_size_pt: f64,
    pub(crate) offset_pt: f64,
    pub(crate) color: Rgba8,
    pub(crate) labels: Vec<ResolvedCldSlot>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedBracket {
    pub(super) slot_a_idx: usize,
    pub(super) slot_b_idx: usize,
    pub(super) x_lo: f64,
    pub(super) x_hi: f64,
    pub(super) style: BracketStyle,
    pub(super) label: RichText,
    pub(super) stroke_width_pt: f64,
    pub(super) font_size_pt: f64,
    pub(super) step_pt: f64,
}

pub(super) fn count_active_brackets(graph: &GraphSheet, doc: &ProjectDocument, items: &[CategorySlotItem]) -> usize {
    graph
        .spec
        .comparisons
        .iter()
        .filter(|b| {
            let res_opt = doc.result_sheet(&b.result_sheet_id);
            if res_opt.is_some_and(|res| matches!(res.status, ResultStatus::Failed { .. })) {
                return false;
            }
            let rec_opt = res_opt.and_then(|res| find_comparison_record(&res.pairwise_comparisons, b.group_a, b.group_b));
            if b.hide_if_non_significant && !rec_opt.is_some_and(|rec| rec.significant) {
                return false;
            }
            if !items.is_empty() {
                let Some(slot_a) = find_item_for_group(items, b.group_a, rec_opt.map(|r| r.label_a.as_str())) else {
                    return false;
                };
                let Some(slot_b) = find_item_for_group(items, b.group_b, rec_opt.map(|r| r.label_b.as_str())) else {
                    return false;
                };
                if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked) && (slot_a.x_pos - slot_b.x_pos).abs() < 1e-6 {
                    return false;
                }
            }
            true
        })
        .count()
}

fn find_item_for_group<'a>(items: &'a [CategorySlotItem], group_idx: usize, label_hint: Option<&str>) -> Option<&'a CategorySlotItem> {
    let idx = find_matched_index(items, group_idx, label_hint, |it| (it.group_key, it.match_label.as_str(), it.alt_label.as_str()))?;
    items.get(idx)
}

pub(super) fn find_matched_index<T>(
    slice: &[T],
    group_idx: usize,
    label_hint: Option<&str>,
    meta: impl Fn(&T) -> (usize, &str, &str),
) -> Option<usize> {
    if let Some(hint) = label_hint {
        let needle = hint.trim();
        if !needle.is_empty()
            && let Some((pos, _)) = slice.iter().enumerate().find(|(_, item)| {
                let (_, lbl, alt) = meta(item);
                lbl.trim() == needle || alt.trim() == needle
            })
        {
            return Some(pos);
        }
    }
    slice.iter().enumerate().find(|(_, item)| meta(item).0 == group_idx).map(|(pos, _)| pos)
}

fn find_comparison_record(records: &[PairwiseComparisonRecord], a: usize, b: usize) -> Option<&PairwiseComparisonRecord> {
    records.iter().find(|r| (r.group_a == a && r.group_b == b) || (r.group_a == b && r.group_b == a))
}

pub(super) fn item_summary_value(item: &CategorySlotItem) -> Option<f64> {
    match &item.payload {
        ExtractedKind::PreSummarized { center, .. } => center.is_finite().then_some(*center),
        ExtractedKind::Raw(obs) => {
            let vals = raw_values(obs);
            let mut unused = Vec::new();
            compute_central_tendency(&vals, gdo_summary_measure(&item.gdo), &mut unused)
        }
    }
}

pub(crate) fn cld_reserved_top_margin_pt(spec: &GraphSpec) -> f64 {
    if !matches!(spec.pairwise_display_mode, PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay) {
        return 0.0;
    }
    let Some(cld) = &spec.compact_letter_display else {
        return 0.0;
    };
    let font = if cld.font_size_pt.is_finite() && cld.font_size_pt > 0.0 { cld.font_size_pt } else { 9.5 };
    let offset = if cld.offset_pt.is_finite() && cld.offset_pt >= 0.0 { cld.offset_pt } else { 6.0 };
    (font + offset + 6.0).clamp(16.0, 34.0)
}

fn collect_cld_means_override(res: &curvebook_doc::ResultSheet, slot_metas: &[(usize, &str, &str, Option<f64>, f64)]) -> Option<Vec<(usize, f64)>> {
    let needs_override = res.pairwise_comparisons.iter().any(|r| r.estimate_diff.is_none()) || matches!(res.analysis.kind, AnalysisKind::Survival(_));
    if !needs_override {
        return None;
    }
    let mut seen: BTreeMap<usize, String> = BTreeMap::new();
    for rec in &res.pairwise_comparisons {
        seen.entry(rec.group_a).or_insert_with(|| rec.label_a.clone());
        seen.entry(rec.group_b).or_insert_with(|| rec.label_b.clone());
    }
    let mut out = Vec::new();
    for (idx, lbl) in seen {
        if let Some(pos) = find_matched_index(slot_metas, idx, Some(lbl.as_str()), |m| (m.0, m.1, m.2))
            && let Some(mean) = slot_metas[pos].3
            && mean.is_finite()
        {
            out.push((idx, mean));
        }
    }
    (!out.is_empty()).then_some(out)
}

fn match_cld_assignments_to_slots(
    assignments: &[curvebook_stats::cld::CldGroupAssignment],
    slot_metas: &[(usize, &str, &str, Option<f64>, f64)],
    grouped_arrangement: GroupedArrangement,
    warnings: &mut Vec<ChartWarning>,
) -> Option<Vec<ResolvedCldSlot>> {
    let mut matched_slots: Vec<(usize, f64)> = Vec::new();
    let mut labels = Vec::new();

    for assign in assignments {
        let Some(pos) = find_matched_index(slot_metas, assign.group_index, Some(assign.label.as_str()), |m| (m.0, m.1, m.2)) else {
            continue;
        };
        let x_pos = slot_metas[pos].4;
        if let Some(&(_, prev_x)) = matched_slots.iter().find(|&&(prev_pos, prev_x)| prev_pos == pos || (prev_x - x_pos).abs() < 1e-6) {
            let reason = if matches!(grouped_arrangement, GroupedArrangement::Stacked | GroupedArrangement::Superimposed) {
                "stacked_cld_guarded"
            } else {
                "overlapping_cld_x_guarded"
            };
            warnings.push(ChartWarning::new(reason, format!("skipped compact letter display for overlapping groups at X={prev_x:.2}")));
            return None;
        }
        matched_slots.push((pos, x_pos));
        labels.push(ResolvedCldSlot { group_key: slot_metas[pos].0, label: assign.label.clone(), letters: assign.letters.clone() });
    }

    (!labels.is_empty()).then_some(labels)
}

pub(crate) fn resolve_cld_overlay(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    slot_metas: &[(usize, &str, &str, Option<f64>, f64)],
    warnings: &mut Vec<ChartWarning>,
) -> Option<ResolvedCldLabels> {
    if !matches!(
        graph.spec.pairwise_display_mode,
        PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay
    ) {
        return None;
    }
    let cld_spec = graph.spec.compact_letter_display.as_ref()?;
    let Some(res) = doc.result_sheet(&cld_spec.result_sheet_id) else {
        warnings.push(ChartWarning::new("missing_cld_result", format!("CLD skipped: result sheet '{}' not found", cld_spec.result_sheet_id)));
        return None;
    };
    if let ResultStatus::Failed { message, .. } = &res.status {
        let detail = if message.trim().is_empty() { "result sheet failed" } else { message.trim() };
        warnings.push(ChartWarning::new("failed_cld_result", format!("CLD skipped: {detail}")));
        return None;
    }

    let options = CldOptions { ordering: cld_spec.ordering, uppercase: cld_spec.uppercase, alpha_override: cld_spec.alpha_override };
    let means_override = collect_cld_means_override(res, slot_metas);
    let cld_res = match compute_cld_from_result_sheet(res, means_override.as_deref(), options) {
        Ok(r) => r,
        Err(err) => {
            warnings.push(ChartWarning::new("incompatible_cld_result", format!("CLD skipped: {err}")));
            return None;
        }
    };
    let labels = match_cld_assignments_to_slots(&cld_res.sorted_assignments, slot_metas, graph.spec.grouped_arrangement, warnings)?;
    let font_size_pt = if cld_spec.font_size_pt.is_finite() && cld_spec.font_size_pt > 0.0 { cld_spec.font_size_pt } else { 9.5 };
    let offset_pt = if cld_spec.offset_pt.is_finite() && cld_spec.offset_pt >= 0.0 { cld_spec.offset_pt } else { 6.0 };
    Some(ResolvedCldLabels { font_size_pt, offset_pt, color: cld_spec.color, labels })
}

pub(crate) fn render_cld_labels(scene: &mut ChartScene, graph: &GraphSheet, cld: &ResolvedCldLabels, bounds: &mut [RenderedSlotBound]) {
    let title_floor_y = if graph.spec.title.trim().is_empty() { 4.0 } else { 24.0 };
    for entry in &cld.labels {
        let Some(pos) =
            find_matched_index(bounds, entry.group_key, Some(entry.label.as_str()), |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))
        else {
            continue;
        };
        let bound = &mut bounds[pos];
        let rich = RichText::plain(entry.letters.clone());
        let metrics = measure_text_pt(&rich, cld.font_size_pt);
        let label_h = metrics.height_pt().max(cld.font_size_pt);
        let min_baseline_y = title_floor_y + label_h + 2.0;
        let max_baseline_y = (scene.plot_rect_pt.bottom() - 6.0).max(min_baseline_y);
        let desired_y = bound.top_y_pt - cld.offset_pt - 0.15 * cld.font_size_pt;
        let cld_y = crate::clamp_f64_safe(desired_y, min_baseline_y, max_baseline_y);

        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich,
            anchor_pt: PointPt::new(bound.x_pt, cld_y),
            anchor: TextAnchor::Middle,
            font_size_pt: cld.font_size_pt,
            rotation_deg: 0.0,
            color: cld.color,
        }));
        bound.top_y_pt = bound.top_y_pt.min(cld_y - label_h - 1.5);
    }
}

pub(super) fn render_pairwise_brackets(scene: &mut ChartScene, graph: &GraphSheet, doc: &ProjectDocument, bounds: &[RenderedSlotBound]) {
    if graph.spec.comparisons.is_empty() {
        return;
    }
    let mut resolved = resolve_brackets(graph, doc, bounds, &mut scene.warnings);
    if resolved.is_empty() {
        return;
    }

    // Sort shortest spans first so wider comparison brackets auto-stack cleanly above inner ones.
    resolved.sort_by(|a, b| {
        let span_a = a.slot_a_idx.abs_diff(a.slot_b_idx);
        let span_b = b.slot_a_idx.abs_diff(b.slot_b_idx);
        span_a.cmp(&span_b).then_with(|| a.x_lo.total_cmp(&b.x_lo))
    });

    let mut placed: Vec<(f64, f64, f64)> = Vec::with_capacity(resolved.len());
    let title_floor_y = if graph.spec.title.trim().is_empty() { 4.0 } else { 24.0 };

    for br in resolved {
        let data_top_y = bounds
            .iter()
            .filter(|b| b.x_pt >= br.x_lo - 1.0 && b.x_pt <= br.x_hi + 1.0)
            .map(|b| b.top_y_pt)
            .fold(f64::INFINITY, f64::min)
            .min(scene.plot_rect_pt.bottom());
        let metrics = measure_text_pt(&br.label, br.font_size_pt);
        let label_h = metrics.height_pt().max(br.font_size_pt);
        let min_step = br.step_pt.max(label_h + 7.0);
        let min_bar_y = title_floor_y + label_h + 4.0;
        let max_bar_y = (scene.plot_rect_pt.bottom() - 8.0).max(min_bar_y);

        let mut bar_y = (data_top_y - 8.0).min(scene.plot_rect_pt.bottom() - 12.0);
        for &(px0, px1, p_top) in &placed {
            if px1 >= br.x_lo - 2.0 && px0 <= br.x_hi + 2.0 {
                bar_y = bar_y.min(p_top - min_step);
            }
        }
        bar_y = crate::clamp_f64_safe(bar_y, min_bar_y, max_bar_y);

        emit_bracket_geometry(&mut scene.primitives, &br, bar_y);
        let label_y = bar_y - 3.5;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: br.label,
            anchor_pt: PointPt::new(0.5 * (br.x_lo + br.x_hi), label_y),
            anchor: TextAnchor::Middle,
            font_size_pt: br.font_size_pt,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        placed.push((br.x_lo, br.x_hi, label_y - label_h));
    }
}

pub(super) fn resolve_brackets(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    bounds: &[RenderedSlotBound],
    warnings: &mut Vec<ChartWarning>,
) -> Vec<ResolvedBracket> {
    let info_id = primary_info_sheet_id(doc, &graph.spec.primary_source_sheet_id);
    let mut out = Vec::with_capacity(graph.spec.comparisons.len());
    for spec in &graph.spec.comparisons {
        let res_opt = doc.result_sheet(&spec.result_sheet_id);
        if let Some(res) = res_opt
            && let ResultStatus::Failed { message, .. } = &res.status
        {
            let detail = if message.trim().is_empty() { "result sheet failed" } else { message.trim() };
            warnings.push(ChartWarning::new("failed_comparison_result", format!("comparison bracket skipped: {detail}")));
            continue;
        }
        let rec_opt = res_opt.and_then(|res| find_comparison_record(&res.pairwise_comparisons, spec.group_a, spec.group_b));

        if spec.hide_if_non_significant {
            let Some(rec) = rec_opt else {
                continue;
            };
            if !rec.significant {
                continue;
            }
        }

        let Some(label) = build_bracket_label(spec, res_opt, rec_opt, doc, info_id, warnings) else {
            continue;
        };
        let slot_a_opt = find_slot_for_group(bounds, spec.group_a, rec_opt.map(|r| r.label_a.as_str()));
        let slot_b_opt = find_slot_for_group(bounds, spec.group_b, rec_opt.map(|r| r.label_b.as_str()));
        let (Some((idx_a, slot_a)), Some((idx_b, slot_b))) = (slot_a_opt, slot_b_opt) else {
            warnings.push(ChartWarning::new(
                "missing_comparison_group",
                format!("comparison bracket {} vs {} skipped: target group not plotted", spec.group_a, spec.group_b),
            ));
            continue;
        };
        if (slot_a.x_pt - slot_b.x_pt).abs() < 1e-6 {
            if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked) {
                warnings.push(ChartWarning::new(
                    "stacked_bracket_guarded",
                    format!("skipped intra-stack comparison bracket ({}, {}) at X={:.1}", spec.group_a, spec.group_b, slot_a.x_pt),
                ));
            }
            continue;
        }
        out.push(ResolvedBracket {
            slot_a_idx: idx_a,
            slot_b_idx: idx_b,
            x_lo: slot_a.x_pt.min(slot_b.x_pt),
            x_hi: slot_a.x_pt.max(slot_b.x_pt),
            style: spec.style,
            label,
            stroke_width_pt: if spec.stroke_width_pt > 0.0 { spec.stroke_width_pt } else { 1.0 },
            font_size_pt: if spec.font_size_pt > 0.0 { spec.font_size_pt } else { 8.5 },
            step_pt: if spec.bracket_step_pt > 0.0 { spec.bracket_step_pt } else { 14.0 },
        });
    }
    out
}

fn build_bracket_label(
    spec: &PairwiseBracketSpec,
    res_opt: Option<&curvebook_doc::ResultSheet>,
    rec_opt: Option<&PairwiseComparisonRecord>,
    doc: &ProjectDocument,
    info_id: Option<&curvebook_doc::SheetId>,
    warnings: &mut Vec<ChartWarning>,
) -> Option<RichText> {
    match &spec.label_mode {
        BracketLabelMode::Custom(raw) => Some(expand_doc_text(raw, doc, info_id)),
        BracketLabelMode::SummaryStars => {
            let (Some(res), Some(rec)) = (res_opt, rec_opt) else {
                warnings.push(ChartWarning::new(
                    "missing_comparison_record",
                    format!("pairwise comparison ({}, {}) not found in '{}'", spec.group_a, spec.group_b, spec.result_sheet_id),
                ));
                return None;
            };
            Some(RichText::plain(res.analysis.p_value_format.stars(rec.raw_p_or_q)))
        }
        BracketLabelMode::NumericPOrQ => {
            let (Some(res), Some(rec)) = (res_opt, rec_opt) else {
                warnings.push(ChartWarning::new(
                    "missing_comparison_record",
                    format!("pairwise comparison ({}, {}) not found in '{}'", spec.group_a, spec.group_b, spec.result_sheet_id),
                ));
                return None;
            };
            let policy = LocaleNumberPolicy::default();
            let mut text = format_pvalue_label(rec.raw_p_or_q, res.analysis.p_value_format, true, false, &policy);
            if rec.is_fdr_q
                && let Some(rest) = text.strip_prefix('P')
            {
                text = format!("q{rest}");
            }
            Some(RichText::plain(text))
        }
    }
}

fn find_slot_for_group<'a>(bounds: &'a [RenderedSlotBound], group_idx: usize, label_hint: Option<&str>) -> Option<(usize, &'a RenderedSlotBound)> {
    let pos = find_matched_index(bounds, group_idx, label_hint, |b| (b.group_key, b.label.as_str(), b.alt_label.as_str()))?;
    bounds.get(pos).map(|b| (pos, b))
}

fn emit_bracket_geometry(primitives: &mut Vec<ScenePrimitive>, br: &ResolvedBracket, bar_y: f64) {
    let stroke = StrokePaint::solid(Rgba8::BLACK, br.stroke_width_pt);
    let path = match br.style {
        BracketStyle::FlatLine => Path2D::line(PointPt::new(br.x_lo, bar_y), PointPt::new(br.x_hi, bar_y)),
        BracketStyle::HalfTick => bracket_legs_path(br.x_lo, br.x_hi, bar_y, 2.5),
        BracketStyle::Square => bracket_legs_path(br.x_lo, br.x_hi, bar_y, 5.0),
        BracketStyle::CurlyBrace => curly_brace_path(br.x_lo, br.x_hi, bar_y, 4.5),
    };
    primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: None });
}

fn bracket_legs_path(x0: f64, x1: f64, y: f64, tick_len: f64) -> Path2D {
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0, y + tick_len)),
            PathCmd::LineTo(PointPt::new(x0, y)),
            PathCmd::LineTo(PointPt::new(x1, y)),
            PathCmd::LineTo(PointPt::new(x1, y + tick_len)),
        ],
    }
}

fn curly_brace_path(x0: f64, x1: f64, y: f64, amp: f64) -> Path2D {
    let mid = 0.5 * (x0 + x1);
    let bot = y + amp;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(PointPt::new(x0, bot)),
            PathCmd::CubicTo { c1: PointPt::new(x0, y), c2: PointPt::new(mid, bot), to: PointPt::new(mid, y - 1.5) },
            PathCmd::CubicTo { c1: PointPt::new(mid, bot), c2: PointPt::new(x1, y), to: PointPt::new(x1, bot) },
        ],
    }
}
