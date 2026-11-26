//! `Change Graph Type` modal body, graph-type option cards, thumbnail glyphs, and memoized live
//! dataset preview with Bezier subdivision and ear-clipping concave tessellation
//! (`gui-layout-spec.md` §8.11B).

use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, ScenePrimitive, compile_graph};
use curvebook_doc::{ColumnDesignHint, ErrorBarMetric, GraphKind, GraphSheet, SummaryMeasure, SurvivalDisplayMetric, TableKind};
use egui::{Align2, Color32, CornerRadius, FontId, Mesh, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{ChangeGraphTypeState, GRAPH_TYPE_OPTIONS, GraphTypeOption, ThumbnailGlyphKind};
use crate::egui_shell::UiIntent;
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::super::{FrameInspectEntryInput, Tokens};
use super::helpers::{ButtonEntry, GraphModalCtx, layout_elided, record_button, record_checkbox, record_label, rgba_to_color32};
use super::state::{GraphModalEguiState, GraphTypeFamily, GraphTypePreviewCache};

pub(super) fn render_change_graph_type_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.change_graph_type.clone();
    let mut dirty = false;

    // Family filter strip & Show All toggle.
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &fam in &GraphTypeFamily::ALL {
            let selected = fam == state.graph_type_family && !(fam == GraphTypeFamily::All && !state.graph_type_show_all);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.family.{}", fam.slug()),
                    label: fam.label(),
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.graph_type_family = fam;
                state.graph_type_show_all = fam == GraphTypeFamily::All;
            }
        }
        let mut show_all = state.graph_type_show_all;
        let show_all_lbl = tr(gctx.locale, "cgt.show_all");
        let chk = ui.checkbox(&mut show_all, show_all_lbl);
        record_checkbox(ui, &chk, gctx.meta, "graph_type.show_all", show_all_lbl, show_all);
        if chk.changed() {
            state.graph_type_show_all = show_all;
            state.graph_type_family = if show_all { GraphTypeFamily::All } else { GraphTypeFamily::Recommended };
        }
    });
    ui.add_space(6.0);

    // Build visible options cleanly from GRAPH_TYPE_OPTIONS.
    let base_options: Vec<&'static GraphTypeOption> = if state.graph_type_show_all || state.graph_type_family != GraphTypeFamily::Recommended {
        GRAPH_TYPE_OPTIONS.iter().filter(|opt| state.graph_type_family.matches(opt)).collect()
    } else {
        draft.recommended_options()
    };

    let cols = if gctx.body_w >= 540.0 { 2 } else { 1 };
    let card_w = ((gctx.body_w - 8.0 * ((cols - 1) as f32)) / (cols as f32)).max(180.0);

    for row_chunk in base_options.chunks(cols) {
        ui.horizontal(|ui| {
            for &opt in row_chunk {
                let selected = draft.selected_option_id == opt.id;
                let compatible = draft.source_table_kind.is_none_or(|k| opt.compatible_tables.contains(&k));
                let resp = render_graph_type_card(ui, gctx, opt, card_w, selected, compatible);
                if resp.clicked() {
                    draft.selected_option_id = opt.id.to_owned();
                    dirty = true;
                }
            }
        });
        ui.add_space(4.0);
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // Summary & Error Bar adaptive controls + compatibility hint.
    let active_opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == draft.selected_option_id).copied();
    let supports_summary = active_opt.is_some_and(|o| matches!(o.graph_kind, GraphKind::Column | GraphKind::Grouped | GraphKind::Xy));
    if supports_summary {
        render_cgt_summary_and_error_bars(ui, gctx, &mut draft, &mut dirty);
        ui.add_space(6.0);
    }

    let is_survival = draft.selected_option_id.starts_with("survival.") || draft.source_table_kind == Some(TableKind::Survival);
    if is_survival {
        render_cgt_survival_options(ui, gctx, &mut draft, &mut dirty);
        ui.add_space(6.0);
    }

    // Compatibility Hint Banner.
    let hint_text = build_compatibility_hint(draft.source_table_kind, draft.column_design_hint, active_opt.as_ref(), gctx.locale);
    egui::Frame::group(ui.style()).fill(gctx.tokens.panel_bg).stroke(Stroke::new(1.0, gctx.tokens.border)).corner_radius(CornerRadius::same(4)).show(
        ui,
        |ui| {
            ui.set_max_width((gctx.body_w - 14.0).max(160.0));
            let lbl_resp = ui.add(egui::Label::new(egui::RichText::new(&hint_text).small()).wrap());
            record_label(ui, &lbl_resp, gctx.meta, "graph_type.compatibility_hint", &hint_text);
        },
    );

    ui.add_space(6.0);
    render_live_dataset_preview(ui, state, gctx, &draft);

    if dirty {
        gctx.intents.push(UiIntent::UpdateChangeGraphTypeState(Box::new(draft)));
    }
}

fn render_cgt_summary_and_error_bars(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut ChangeGraphTypeState, dirty: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.summary")).strong());
        for (slug, lbl, val) in [
            ("mean", "Mean", SummaryMeasure::Mean),
            ("median", "Median", SummaryMeasure::Median),
            ("geomean", "Geom. Mean", SummaryMeasure::GeometricMean),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.sum.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.summary_measure == val,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.summary_measure = val;
                *dirty = true;
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.error_bars")).strong());
        for (slug, lbl, metric) in [
            ("sd", "SD", ErrorBarMetric::Sd),
            ("sem", "SEM", ErrorBarMetric::Sem),
            ("ci95", "95% CI", ErrorBarMetric::ConfidenceInterval95),
            ("iqr", "IQR", ErrorBarMetric::InterquartileRange),
            ("range", "Range", ErrorBarMetric::Range),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.eb.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.error_bar_metric == metric,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.error_bar_metric = metric;
                *dirty = true;
            }
        }
    });
}

fn render_cgt_survival_options(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut ChangeGraphTypeState, dirty: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new("Survival Y:").strong());
        for (slug, lbl, metric) in [
            ("pct_survival", "% Survival", SurvivalDisplayMetric::PercentSurvival),
            ("pct_event", "% Event", SurvivalDisplayMetric::PercentCumulativeEvent),
            ("fraction", "Fraction", SurvivalDisplayMetric::FractionSurvival),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.survival.metric.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.survival_metric == metric,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.survival_metric = metric;
                *dirty = true;
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new("Curve:").strong());
        for (slug, lbl, stair) in [("staircase", "Staircase", true), ("point_to_point", "Point-to-point", false)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.survival.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.survival_staircase == stair,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.survival_staircase = stair;
                *dirty = true;
            }
        }
        ui.label("Error:");
        for (slug, lbl, err_opt) in
            [("none", "None", None), ("sem", "SEM", Some(ErrorBarMetric::Sem)), ("ci95", "95% CI", Some(ErrorBarMetric::ConfidenceInterval95))]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.survival.err.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.survival_error_display == err_opt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.survival_error_display = err_opt;
                *dirty = true;
            }
        }
    });
}

fn build_compatibility_hint(
    src_kind: Option<TableKind>,
    design_hint: Option<ColumnDesignHint>,
    opt: Option<&GraphTypeOption>,
    locale: &str,
) -> String {
    if let Some(o) = opt
        && let Some(kind) = src_kind
        && !o.compatible_tables.contains(&kind)
    {
        return format!("{} ({:?} -> {})", tr(locale, "cgt.hint.cross_table"), kind, o.label);
    }
    if design_hint == Some(ColumnDesignHint::PairedRepeatedMeasures) {
        return tr(locale, "cgt.hint.paired").to_owned();
    }
    tr(locale, "cgt.hint.compatible").to_owned()
}

fn render_graph_type_card(
    ui: &mut Ui,
    gctx: &mut GraphModalCtx<'_>,
    opt: &GraphTypeOption,
    card_w: f32,
    selected: bool,
    compatible: bool,
) -> Response {
    let card_h = 56.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
    let stroke_color = if selected { gctx.tokens.accent } else { gctx.tokens.border };
    let fill_color = if selected {
        gctx.tokens.active_bg
    } else if resp.hovered() {
        gctx.tokens.panel_bg
    } else {
        gctx.tokens.surface_bg
    };
    ui.painter().rect(rect, CornerRadius::same(6), fill_color, Stroke::new(if selected { 1.5 } else { 1.0 }, stroke_color), StrokeKind::Inside);

    let thumb_rect = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(40.0, 40.0));
    paint_thumbnail_glyph(ui.painter(), thumb_rect, opt.thumbnail, gctx.tokens);

    let text_x = thumb_rect.right() + 8.0;
    let text_w = (rect.right() - text_x - 8.0).max(40.0);
    let (title_galley, elided) = layout_elided(ui.painter(), opt.label, text_w, gctx.tokens.text_primary, 12.0);
    ui.painter().galley(Pos2::new(text_x, rect.top() + 8.0), title_galley, gctx.tokens.text_primary);

    let tag = if compatible { opt.subtitle } else { "Cross-table format hint" };
    let (sub_galley, _) = layout_elided(ui.painter(), tag, text_w, gctx.tokens.text_muted, 10.5);
    ui.painter().galley(Pos2::new(text_x, rect.top() + 28.0), sub_galley, gctx.tokens.text_muted);

    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, opt.label));
    if elided || resp.hovered() {
        resp.clone().on_hover_text(format!("{}\n{}", opt.label, opt.subtitle));
    }
    gctx.meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &format!("graph_type.card.{}", opt.id),
            role: "option",
            label: opt.label,
            command_id: None,
            value: Some(json!({ "selected": selected, "compatible": compatible })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: elided,
            full_label: elided.then_some(opt.label),
        },
    );
    resp
}

fn paint_thumbnail_glyph(painter: &egui::Painter, rect: Rect, kind: ThumbnailGlyphKind, tokens: &Tokens) {
    painter.rect(rect, CornerRadius::same(4), tokens.panel_bg, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let c0 = tokens.accent;
    let c1 = tokens.text_primary;
    let st = Stroke::new(1.5, c0);
    let axis_st = Stroke::new(1.0, tokens.text_muted);
    painter.line_segment([Pos2::new(inner.left(), inner.top()), Pos2::new(inner.left(), inner.bottom())], axis_st);
    painter.line_segment([Pos2::new(inner.left(), inner.bottom()), Pos2::new(inner.right(), inner.bottom())], axis_st);

    match kind {
        ThumbnailGlyphKind::Scatter | ThumbnailGlyphKind::Volcano | ThumbnailGlyphKind::BlandAltman | ThumbnailGlyphKind::ResidualPlot => {
            for (fx, fy) in [(0.25, 0.72), (0.45, 0.45), (0.65, 0.58), (0.82, 0.25)] {
                painter.circle_filled(lerp_rect(inner, fx, fy), 2.3, c0);
            }
        }
        ThumbnailGlyphKind::ScatterLine | ThumbnailGlyphKind::RocCurve | ThumbnailGlyphKind::NormalQqPlot => {
            let pts = [lerp_rect(inner, 0.15, 0.8), lerp_rect(inner, 0.4, 0.55), lerp_rect(inner, 0.65, 0.35), lerp_rect(inner, 0.9, 0.18)];
            painter.add(egui::Shape::line(pts.to_vec(), st));
            for p in pts {
                painter.circle_filled(p, 2.0, c1);
            }
        }
        ThumbnailGlyphKind::BarReplicates | ThumbnailGlyphKind::GroupedInterleaved | ThumbnailGlyphKind::StackedBar => {
            for (idx, h) in [0.45, 0.75, 0.6].iter().enumerate() {
                let x = inner.left() + 4.0 + (idx as f32) * (inner.width() / 3.2);
                let top = inner.bottom() - inner.height() * *h;
                painter.rect_filled(Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + 6.0, inner.bottom())), CornerRadius::ZERO, c0);
            }
        }
        ThumbnailGlyphKind::BeforeAfter => {
            for (y0, y1) in [(0.7, 0.3), (0.5, 0.2), (0.8, 0.55)] {
                let p0 = lerp_rect(inner, 0.25, y0);
                let p1 = lerp_rect(inner, 0.75, y1);
                painter.line_segment([p0, p1], st);
                painter.circle_filled(p0, 2.0, c1);
                painter.circle_filled(p1, 2.0, c0);
            }
        }
        ThumbnailGlyphKind::BoxWhisker | ThumbnailGlyphKind::ForestPlot => {
            for (fx, top, bot) in [(0.35, 0.35, 0.65), (0.7, 0.2, 0.55)] {
                let cx = inner.left() + inner.width() * fx;
                painter.line_segment([Pos2::new(cx, inner.top() + 3.0), Pos2::new(cx, inner.bottom() - 3.0)], Stroke::new(1.0, c1));
                painter.rect(
                    Rect::from_min_max(
                        Pos2::new(cx - 4.0, inner.top() + inner.height() * top),
                        Pos2::new(cx + 4.0, inner.top() + inner.height() * bot),
                    ),
                    CornerRadius::ZERO,
                    tokens.active_bg,
                    st,
                    StrokeKind::Inside,
                );
            }
        }
        ThumbnailGlyphKind::Violin | ThumbnailGlyphKind::AlignedDot => {
            let pts = [lerp_rect(inner, 0.5, 0.15), lerp_rect(inner, 0.75, 0.45), lerp_rect(inner, 0.5, 0.85), lerp_rect(inner, 0.25, 0.45)];
            painter.add(egui::Shape::convex_polygon(pts.to_vec(), tokens.active_bg, st));
        }
        ThumbnailGlyphKind::KaplanMeier => {
            let pts = [
                lerp_rect(inner, 0.1, 0.2),
                lerp_rect(inner, 0.4, 0.2),
                lerp_rect(inner, 0.4, 0.5),
                lerp_rect(inner, 0.75, 0.5),
                lerp_rect(inner, 0.75, 0.8),
                lerp_rect(inner, 0.95, 0.8),
            ];
            painter.add(egui::Shape::line(pts.to_vec(), st));
        }
        ThumbnailGlyphKind::Donut => {
            let c = inner.center();
            let r = inner.width().min(inner.height()) * 0.42;
            painter.circle_stroke(c, r, Stroke::new(4.0, c0));
        }
        ThumbnailGlyphKind::BubbleScatter => {
            painter.circle_filled(lerp_rect(inner, 0.3, 0.6), 4.0, c0);
            painter.circle_filled(lerp_rect(inner, 0.65, 0.35), 6.0, tokens.active_bg);
            painter.circle_stroke(lerp_rect(inner, 0.65, 0.35), 6.0, st);
        }
        ThumbnailGlyphKind::Heatmap => {
            let cw = inner.width() / 3.0;
            let ch = inner.height() / 3.0;
            for r in 0..3 {
                for c in 0..3 {
                    let shade = ((r * 3 + c) * 24) as u8;
                    let cell_col = Color32::from_rgb(40 + shade / 2, 110, 210 - shade / 3);
                    let cell =
                        Rect::from_min_size(Pos2::new(inner.left() + (c as f32) * cw, inner.top() + (r as f32) * ch), Vec2::new(cw - 1.0, ch - 1.0));
                    painter.rect_filled(cell, CornerRadius::ZERO, cell_col);
                }
            }
        }
    }
}

fn lerp_rect(rect: Rect, fx: f32, fy: f32) -> Pos2 {
    Pos2::new(rect.left() + rect.width() * fx, rect.top() + rect.height() * fy)
}

/// Resolve or compile the memoized live preview [`ChartScene`] on `state`, returning `(scene_opt, recompiled)`
/// so repeated hover/scroll repaints avoid per-frame `compile_graph` (`AUD-10-54`).
pub(super) fn get_or_compile_preview<'a>(
    state: &'a mut GraphModalEguiState,
    app: &UiApp,
    cgt: &ChangeGraphTypeState,
) -> (Option<&'a Result<ChartScene, String>>, bool) {
    let Some(gid) = cgt.graph_sheet_id.as_ref() else {
        state.graph_type_preview_cache = None;
        return (None, false);
    };
    let doc = app.document();
    let Some(gs) = doc.graph_sheet(gid) else {
        state.graph_type_preview_cache = None;
        return (None, false);
    };
    let undo_depth = app.session().undo_depth();
    let redo_depth = app.session().redo_depth();
    let sheet_counts = (doc.data_sheets.len(), doc.result_sheets.len(), doc.graph_sheets.len());

    let hit = state.graph_type_preview_cache.as_ref().is_some_and(|c| {
        c.graph_sheet_id == *gid
            && c.selected_option_id == cgt.selected_option_id
            && c.summary_measure == cgt.summary_measure
            && c.error_bar_metric == cgt.error_bar_metric
            && c.survival_metric == cgt.survival_metric
            && c.survival_staircase == cgt.survival_staircase
            && c.survival_error_display == cgt.survival_error_display
            && c.undo_depth == undo_depth
            && c.redo_depth == redo_depth
            && c.sheet_counts == sheet_counts
            && c.source_spec == gs.spec
    });

    if !hit {
        let mut temp: GraphSheet = gs.clone();
        let _ = cgt.apply_to_spec(&mut temp.spec);
        let compiled = compile_graph(&temp, doc).map_err(|e| e.to_string());
        state.graph_type_preview_cache = Some(GraphTypePreviewCache {
            graph_sheet_id: gid.clone(),
            selected_option_id: cgt.selected_option_id.clone(),
            summary_measure: cgt.summary_measure,
            error_bar_metric: cgt.error_bar_metric,
            survival_metric: cgt.survival_metric,
            survival_staircase: cgt.survival_staircase,
            survival_error_display: cgt.survival_error_display,
            source_spec: gs.spec.clone(),
            undo_depth,
            redo_depth,
            sheet_counts,
            compiled,
        });
    }
    (state.graph_type_preview_cache.as_ref().map(|c| &c.compiled), !hit)
}

fn render_live_dataset_preview(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>, draft: &ChangeGraphTypeState) {
    ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.preview")).strong());
    ui.add_space(3.0);
    let preview_w = gctx.body_w;
    let preview_h = 112.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(preview_w, preview_h), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(6), Color32::WHITE, Stroke::new(1.0, gctx.tokens.border), StrokeKind::Inside);

    let (compiled, _) = get_or_compile_preview(state, gctx.app, draft);
    match compiled {
        Some(Ok(scene)) if !scene.primitives.is_empty() => {
            paint_chart_scene_compact(ui.painter(), scene, rect.shrink(6.0));
        }
        Some(Err(err)) => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                err,
                FontId::proportional(gctx.tokens.metrics.font_ui_small_pt),
                gctx.tokens.text_muted,
            );
        }
        _ => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr(gctx.locale, "cgt.preview.empty"),
                FontId::proportional(gctx.tokens.metrics.font_ui_small_pt),
                gctx.tokens.text_muted,
            );
        }
    }

    let preview_lbl = tr(gctx.locale, "w5.graph_type.preview").to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Image, true, preview_lbl.clone()));
    gctx.meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: "graph_type.preview_pane",
            role: "img",
            label: tr(gctx.locale, "w5.graph_type.preview"),
            command_id: None,
            value: Some(json!({ "selected_option": draft.selected_option_id })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

const QUAD_BEZIER_STEPS: usize = 8;
const CUBIC_BEZIER_STEPS: usize = 10;

/// Flatten a [`Path2D`] into screen-space polylines, subdividing quadratic and cubic Beziers
/// cleanly instead of jumping straight to endpoints (`AUD-10-54`).
pub(super) fn flatten_path_subpaths(path: &Path2D, map_pt: impl Fn(PointPt) -> Pos2) -> Vec<(Vec<Pos2>, bool)> {
    let mut subpaths: Vec<(Vec<Pos2>, bool)> = Vec::new();
    let mut pts: Vec<Pos2> = Vec::new();
    let mut closed = false;
    let mut cur: Option<PointPt> = None;

    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(p) => {
                if pts.len() >= 2 {
                    subpaths.push((std::mem::take(&mut pts), closed));
                } else {
                    pts.clear();
                }
                closed = false;
                pts.push(map_pt(p));
                cur = Some(p);
            }
            PathCmd::LineTo(p) => {
                pts.push(map_pt(p));
                cur = Some(p);
            }
            PathCmd::QuadTo { control, to } => {
                if let Some(p0) = cur {
                    for step in 1..=QUAD_BEZIER_STEPS {
                        let t = (step as f64) / (QUAD_BEZIER_STEPS as f64);
                        let u = 1.0 - t;
                        let x = u * u * p0.x + 2.0 * u * t * control.x + t * t * to.x;
                        let y = u * u * p0.y + 2.0 * u * t * control.y + t * t * to.y;
                        pts.push(map_pt(PointPt::new(x, y)));
                    }
                } else {
                    pts.push(map_pt(to));
                }
                cur = Some(to);
            }
            PathCmd::CubicTo { c1, c2, to } => {
                if let Some(p0) = cur {
                    for step in 1..=CUBIC_BEZIER_STEPS {
                        let t = (step as f64) / (CUBIC_BEZIER_STEPS as f64);
                        let u = 1.0 - t;
                        let u2 = u * u;
                        let t2 = t * t;
                        let x = u2 * u * p0.x + 3.0 * u2 * t * c1.x + 3.0 * u * t2 * c2.x + t2 * t * to.x;
                        let y = u2 * u * p0.y + 3.0 * u2 * t * c1.y + 3.0 * u * t2 * c2.y + t2 * t * to.y;
                        pts.push(map_pt(PointPt::new(x, y)));
                    }
                } else {
                    pts.push(map_pt(to));
                }
                cur = Some(to);
            }
            PathCmd::Close => closed = true,
        }
    }
    if pts.len() >= 2 {
        subpaths.push((pts, closed));
    }
    subpaths
}

fn cross2(a: Pos2, b: Pos2, c: Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn near_pos(a: Pos2, b: Pos2) -> bool {
    (a - b).length_sq() < 1e-6
}

fn point_in_oriented_triangle(p: Pos2, a: Pos2, b: Pos2, c: Pos2, orient: f32) -> bool {
    if near_pos(p, a) || near_pos(p, b) || near_pos(p, c) {
        return false;
    }
    let eps = -1e-4_f32;
    orient * cross2(a, b, p) >= eps && orient * cross2(b, c, p) >= eps && orient * cross2(c, a, p) >= eps
}

/// Triangulate a simple (possibly concave) 2D polygon using panic-free ear-clipping (`AUD-10-54`).
/// Returns vertex-index triples into `verts_out` (the cleaned deduplicated vertex list).
pub(super) fn triangulate_polygon_ear_clip(pts: &[Pos2]) -> (Vec<Pos2>, Vec<[u32; 3]>) {
    let mut verts: Vec<Pos2> = Vec::with_capacity(pts.len());
    for &p in pts {
        if !p.x.is_finite() || !p.y.is_finite() {
            continue;
        }
        if verts.last().is_none_or(|&prev| !near_pos(prev, p)) {
            verts.push(p);
        }
    }
    while verts.len() >= 2 && verts.first().zip(verts.last()).is_some_and(|(&a, &b)| near_pos(a, b)) {
        verts.pop();
    }
    if verts.len() < 3 || u32::try_from(verts.len()).is_err() {
        return (verts, Vec::new());
    }

    let mut signed_area2 = 0.0_f32;
    for i in 0..verts.len() {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        signed_area2 += a.x * b.y - b.x * a.y;
    }
    if signed_area2.abs() < 1e-6 {
        return (verts, Vec::new());
    }
    let orient = if signed_area2 > 0.0 { 1.0_f32 } else { -1.0_f32 };

    let mut remaining: Vec<usize> = (0..verts.len()).collect();
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(verts.len().saturating_sub(2));
    let max_iters = verts.len().saturating_mul(verts.len()).saturating_add(16);

    for _ in 0..max_iters {
        if remaining.len() <= 3 {
            break;
        }
        let n = remaining.len();
        let mut clipped_pos: Option<usize> = None;
        for i in 0..n {
            let prev = remaining[(i + n - 1) % n];
            let curr = remaining[i];
            let next = remaining[(i + 1) % n];
            let (a, b, c) = (verts[prev], verts[curr], verts[next]);
            if orient * cross2(a, b, c) <= 1e-6 {
                continue;
            }
            let contains_other = remaining
                .iter()
                .copied()
                .any(|idx| idx != prev && idx != curr && idx != next && point_in_oriented_triangle(verts[idx], a, b, c, orient));
            if !contains_other {
                if let (Ok(i0), Ok(i1), Ok(i2)) = (u32::try_from(prev), u32::try_from(curr), u32::try_from(next)) {
                    tris.push([i0, i1, i2]);
                }
                clipped_pos = Some(i);
                break;
            }
        }
        if let Some(pos) = clipped_pos {
            remaining.remove(pos);
            continue;
        }
        // Fallback for collinear or near-degenerate edges: prune a collinear vertex safely.
        let collinear = (0..n).find(|&i| {
            let prev = remaining[(i + n - 1) % n];
            let curr = remaining[i];
            let next = remaining[(i + 1) % n];
            cross2(verts[prev], verts[curr], verts[next]).abs() <= 1e-5
        });
        if let Some(pos) = collinear {
            remaining.remove(pos);
        } else {
            break;
        }
    }

    if remaining.len() == 3
        && let (Ok(i0), Ok(i1), Ok(i2)) = (u32::try_from(remaining[0]), u32::try_from(remaining[1]), u32::try_from(remaining[2]))
    {
        tris.push([i0, i1, i2]);
    }
    (verts, tris)
}

fn paint_chart_scene_compact(painter: &egui::Painter, scene: &ChartScene, paper: Rect) {
    let sx = paper.width() / (scene.width_pt.max(1.0) as f32);
    let sy = paper.height() / (scene.height_pt.max(1.0) as f32);
    let map_pt = |pt: PointPt| Pos2::new(paper.left() + (pt.x as f32) * sx, paper.top() + (pt.y as f32) * sy);
    let clipped = painter.with_clip_rect(paper);

    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill, stroke, .. } = prim {
            for (pts, closed) in flatten_path_subpaths(path, map_pt) {
                flush_preview_poly(&clipped, &pts, closed, fill.as_ref(), stroke.as_ref(), sx);
            }
        }
    }
}

fn flush_preview_poly(
    painter: &egui::Painter,
    pts: &[Pos2],
    closed: bool,
    fill: Option<&curvebook_chart::FillPaint>,
    stroke: Option<&curvebook_chart::StrokePaint>,
    scale: f32,
) {
    if pts.len() < 2 {
        return;
    }
    if closed && let Some(f) = fill {
        let color = rgba_to_color32(f.color);
        let (verts, tris) = triangulate_polygon_ear_clip(pts);
        if !tris.is_empty() {
            let mut mesh = Mesh::default();
            mesh.vertices.reserve(verts.len());
            for pos in verts {
                mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::epaint::WHITE_UV, color });
            }
            mesh.indices.reserve(tris.len().saturating_mul(3));
            for [i0, i1, i2] in tris {
                mesh.indices.extend_from_slice(&[i0, i1, i2]);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
    if let Some(s) = stroke {
        let st = Stroke::new(((s.width_pt as f32) * scale).max(0.75), rgba_to_color32(s.color));
        if closed {
            painter.add(egui::Shape::closed_line(pts.to_vec(), st));
        } else {
            painter.add(egui::Shape::line(pts.to_vec(), st));
        }
    }
}
