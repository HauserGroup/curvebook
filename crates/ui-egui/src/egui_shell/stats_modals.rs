//! Statistical, GLM, Cox, PCA/PCR, Clustering, Survival, and `dialogs_r9` parameter modals
//! plus `AnalyzeSheet`, `DuplicateAnalysisPrompt`, and `ChangeDataAnalyzed` in `egui 0.37`
//! (`R15.4`, `gui-layout-spec.md` §8.7–§8.8).

use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ContingencyMethod, CoxTieMethod, GlmFamily, KappaWeighting, MixedModelDfMethod,
    MultiplicityMethod, MultivariateSpec, OutlierSpec, PValueFormat, PValueStackMethod, ProjectDocument, RepeatedMeasuresDesign, ResidualCovariance,
    SheetId, SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, VariableRole,
};
use curvebook_text::{EquationSegment, SpanScript, SpanSlope, SpanWeight, parse_equation_segments};
use egui::{
    Color32, Context, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2,
    scroll_area::ScrollBarVisibility,
};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{
    ActiveDialog, AnalysisCategory, AnalyzeDialogState, EditAnalysisParametersState, ResultSheetViewState, VariableRoleFilter,
};
use crate::dialogs_r9::{RandomizationSchemeKind, SimDistKind};
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{
    ControlEnablement, evaluate_dialog_button, is_reml_mixed_effects_active, r_km_trend_test, r1_paired_or_matched_allowed,
    r1b_post_hoc_option_enabled, r1b_sphericity_correction, r2_assume_gaussian_allowed, r4_two_group_diagnostics, r4_two_group_graph_differences,
    r4_two_group_nonparametric_options, r4_two_group_welch_correction, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend,
    r7_correlation_heatmap_toggle, r8_subcolumn_aggregation, r9_pvalue_precision_spinner, r10_pcr_outcome_column, source_table_shape,
    survival_group_count,
};
use crate::views::analyze_picker::DuplicateChoice;

use super::inspect_bridge::{ModalCaptureReport, ModalCaptureSpec, finalize_egui_modal_response};
use super::stat_anova_modal::{
    ClippedTextSpec, StepperSpec, column_titles_for_draft, record_widget, render_clipped_text, render_modal_checkbox, render_modal_radio,
    render_modal_seg_btn, render_modal_textbox_row, render_section_title, render_stepper_row,
};
use super::widgets::paint_icon_button_named;
use super::{FrameInspectMeta, PendingUiIntents, Tokens};

/// Persistent transient UI state for `stats_modals` dialogs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsModalsEguiState {
    pub active_tab: String,
    pub analyze_query_draft: String,
    pub analyze_focus_requested: bool,
    pub mv_var_search: String,
    pub show_method_reference: bool,
    pub method_saved_as_default: bool,
    pub last_resolved_banner: String,
    pub open_session_sheet_id: Option<String>,
    pub lin_x_min_draft: String,
    pub lin_x_max_draft: String,
    pub lin_plot_min_draft: String,
    pub lin_plot_max_draft: String,
    pub lin_segments_draft: String,
}

/// Frame geometry and modal capture report emitted by `stats_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StatsModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub resolved_test_banner: String,
}

/// Shared modal chrome configuration for Wave 4 native `egui` dialogs (`R15.4`).
pub(super) struct Wave4ModalShell<'a> {
    pub(super) widget_id: &'a str,
    pub(super) title: &'a str,
    pub(super) subtitle_or_banner: Option<&'a str>,
    pub(super) banner_widget_id: &'a str,
    pub(super) max_width_pt: f32,
    pub(super) max_height_pt: f32,
    pub(super) use_body_scroll: bool,
    pub(super) footer_reserve_pt: f32,
}

impl From<ModalCaptureReport> for StatsModalFrameReport {
    fn from(c: ModalCaptureReport) -> Self {
        Self {
            modal_open: true,
            dialog_rect_pt: c.dialog_rect_pt,
            outside_click_captured: c.outside_click_captured,
            outside_wheel_captured: c.outside_wheel_captured,
            resolved_test_banner: String::new(),
        }
    }
}

/// Render a centered, responsive Wave 4 `egui::Modal` shell with pinned header/footer,
/// `AlwaysVisible` vertical scrollbar gutter (zero `Role::ScrollBar` overlap),
/// `ZLayer::ModalBackdrop` capture, and `FrameInspectMeta` registration.
#[allow(clippy::too_many_arguments)]
pub(super) fn show_wave4_modal<S, H, B, F>(
    ctx: &Context,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    state: &mut S,
    cfg: Wave4ModalShell<'_>,
    header_extra: H,
    body_fn: B,
    footer_fn: F,
) -> ModalCaptureReport
where
    H: FnOnce(&mut Ui, &mut S, &mut FrameInspectMeta, &mut PendingUiIntents),
    B: FnOnce(&mut Ui, &mut S, f32, f32, &mut FrameInspectMeta, &mut PendingUiIntents),
    F: FnOnce(&mut Ui, &mut S, &mut FrameInspectMeta, &mut PendingUiIntents),
{
    let screen = ctx.content_rect();
    let modal_w = (screen.width() - 24.0).clamp(280.0, cfg.max_width_pt);
    let modal_h = (screen.height() - 24.0).clamp(240.0, cfg.max_height_pt);
    let inner_w = (modal_w - 24.0).max(240.0);
    let inner_h = (modal_h - 24.0).max(180.0);
    let footer_h = cfg.footer_reserve_pt.max(36.0);

    let area = egui::Modal::default_area(Id::unique(("wave4_modal_area", cfg.widget_id))).constrain_to(screen);
    let frame = egui::Frame::window(&ctx.global_style())
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.5, tokens.accent_strong))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12));

    let modal_resp =
        egui::Modal::new(Id::unique(("wave4_modal", cfg.widget_id))).backdrop_color(tokens.scrim).area(area).frame(frame).show(ctx, |ui| {
            ui.set_min_size(Vec2::new(inner_w, inner_h));
            ui.set_max_size(Vec2::new(inner_w, inner_h));
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

            // Header strip
            ui.horizontal(|ui| {
                let title_w = (inner_w - 30.0).max(80.0);
                ui.allocate_ui(Vec2::new(title_w, 22.0), |ui| {
                    render_clipped_text(
                        ui,
                        meta,
                        ClippedTextSpec {
                            widget_id: "modal.title",
                            role: "label",
                            text: cfg.title,
                            font_size: 14.5,
                            color: tokens.text_strong,
                            z_layer: ZLayer::ModalDialog,
                            tooltip: None,
                            record_meta: true,
                        },
                    );
                });
                let (_, close_alloc) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                let close_lbl = tr(tokens.locale, "btn.cancel");
                let close_resp = paint_icon_button_named(ui, close_alloc, "close", close_lbl, close_lbl, false);
                record_widget(ui, meta, &close_resp, "modal.close", "button", close_lbl, None, None, true, None, ZLayer::ModalDialog, false, None);
                if close_resp.clicked() {
                    intents.push_dialog_button("cancel");
                }
            });

            if let Some(banner_text) = cfg.subtitle_or_banner
                && !banner_text.trim().is_empty()
            {
                ui.add_space(4.0);
                let banner_frame = egui::Frame::NONE
                    .fill(tokens.active_bg)
                    .stroke(Stroke::new(1.0, tokens.accent))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(8, 4));
                let bw = (inner_w - 16.0).max(120.0);
                banner_frame.show(ui, |ui| {
                    ui.set_width(bw);
                    ui.set_max_width(bw);
                    render_clipped_text(
                        ui,
                        meta,
                        ClippedTextSpec {
                            widget_id: cfg.banner_widget_id,
                            role: "status",
                            text: banner_text,
                            font_size: 11.5,
                            color: tokens.text_strong,
                            z_layer: ZLayer::ModalDialog,
                            tooltip: None,
                            record_meta: true,
                        },
                    );
                });
            }

            header_extra(ui, state, meta, intents);
            ui.add_space(4.0);

            let body_h = (ui.available_height() - footer_h - 6.0).max(72.0);
            if cfg.use_body_scroll {
                let scroll_w = (inner_w - 18.0).max(160.0);
                egui::ScrollArea::vertical()
                    .id_salt(("wave4_body_scroll", cfg.widget_id))
                    .max_height(body_h)
                    .min_scrolled_height(body_h)
                    .auto_shrink([false, false])
                    .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: true, scroll_bar: true, ..egui::scroll_area::ScrollSource::NONE })
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.set_width(scroll_w);
                        ui.set_max_width(scroll_w);
                        body_fn(ui, state, scroll_w, body_h, meta, intents);
                    });
            } else {
                ui.allocate_ui(Vec2::new(inner_w, body_h), |ui| {
                    ui.set_width(inner_w);
                    ui.set_max_width(inner_w);
                    body_fn(ui, state, inner_w, body_h, meta, intents);
                });
            }

            ui.add_space(4.0);
            ui.separator();
            ui.allocate_ui(Vec2::new(inner_w, (footer_h - 6.0).max(28.0)), |ui| {
                footer_fn(ui, state, meta, intents);
            });
        });

    let title_for_ak = cfg.title.to_owned();
    modal_resp.response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

    finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: cfg.widget_id, title: cfg.title, z_dialog: ZLayer::ModalDialog },
        meta,
    )
}

/// Shared modal button renderer supporting primary styling, truncation tooltips, and disabled reasons.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_modal_action_button(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    primary: bool,
    selected: bool,
    enablement: &ControlEnablement,
) -> bool {
    let enabled = enablement.enabled;
    let min_w: f32 = if primary { 92.0 } else { 66.0 };
    let max_w = (ui.available_width() - 4.0).max(48.0);
    let fg_text = if !enabled {
        tokens.text_disabled
    } else if primary {
        Color32::WHITE
    } else {
        tokens.text_strong
    };
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), fg_text));
    job.wrap = egui::text::TextWrapping { max_width: (max_w - 16.0).max(24.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let w = (galley.size().x + 18.0).clamp(min_w.min(max_w), max_w);
    let vis_clip = if meta.viewport_rect.is_positive() { ui.clip_rect().intersect(meta.viewport_rect) } else { ui.clip_rect() };
    let row_top = ui.cursor().top();
    let row_bot = row_top + 25.0;
    if row_top > vis_clip.bottom() || (!ui.layout().main_wrap() && row_bot < vis_clip.top()) {
        ui.allocate_space(Vec2::new(w, 25.0));
        return false;
    }
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 25.0), Sense::click());
    let resp = if enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };

    let (bg, fg, border) = if !enabled {
        (tokens.panel_bg, tokens.text_disabled, tokens.divider)
    } else if primary {
        (tokens.accent_strong, Color32::WHITE, tokens.accent_strong)
    } else if selected {
        (tokens.active_bg, tokens.text_strong, tokens.accent)
    } else if resp.hovered() {
        (tokens.hover_bg, tokens.text_strong, tokens.button_border)
    } else {
        (tokens.button_bg, tokens.text_strong, tokens.button_border)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let a11y_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, a11y_label.clone()));
    let tip = enablement.disabled_reason.as_deref().filter(|s| !s.is_empty()).or(ellipsized.then_some(label)).unwrap_or("");
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };

    record_widget(
        ui,
        meta,
        &resp,
        widget_id,
        "button",
        label,
        None,
        None,
        enabled,
        enablement.disabled_reason.as_deref(),
        ZLayer::ModalDialog,
        ellipsized,
        ellipsized.then_some(label),
    );
    enabled && resp.clicked()
}

/// Draw a highlighted live symbolic formula bar (`widget_id`: e.g. `"edit.formula_bar"`).
pub(super) fn render_formula_bar(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, widget_id: &str, title: &str, formula: &str) {
    let w = ui.available_width().max(120.0);
    let box_frame = egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Vec2::new(8.0, 5.0));
    box_frame.show(ui, |ui| {
        ui.set_width((w - 16.0).max(80.0));
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("{title}:")).small().strong().color(tokens.text_muted));
            render_clipped_text(
                ui,
                meta,
                ClippedTextSpec {
                    widget_id,
                    role: "status",
                    text: formula,
                    font_size: 11.5,
                    color: tokens.accent_strong,
                    z_layer: ZLayer::ModalDialog,
                    tooltip: Some(formula),
                    record_meta: true,
                },
            );
        });
    });
}

/// Render TeX math parsed by [`curvebook_text::parse_equation_segments`] into a compact visual card.
pub(super) fn render_tex_equation_preview(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, widget_id: &str, tex: &str, font_size: f32) {
    let segments = parse_equation_segments(tex);
    let avail_w = ui.available_width().max(120.0);
    let (eq_w, eq_h) = measure_equation_segments(ui, &segments, font_size);
    let card_h = (eq_h + 16.0).clamp(36.0, 92.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail_w, card_h), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), tokens.panel_bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    if segments.is_empty() {
        let mut job =
            egui::text::LayoutJob::single_section(tex.to_owned(), egui::TextFormat::simple(FontId::proportional(font_size), tokens.text_strong));
        job.wrap = egui::text::TextWrapping {
            max_width: (rect.width() - 16.0).max(40.0),
            max_rows: 2,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let g = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(rect.shrink(6.0)).galley(Pos2::new(rect.left() + 8.0, rect.center().y - g.size().y * 0.5), g, tokens.text_strong);
    } else {
        let origin = Pos2::new(rect.left() + ((rect.width() - eq_w.min(rect.width() - 16.0)) * 0.5).max(8.0), rect.center().y - eq_h * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink(4.0));
        paint_equation_segments(ui, &painter, origin, &segments, font_size, tokens.text_strong);
    }

    let a11y = tex.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, a11y.clone()));
    let resp = resp.on_hover_text(tex);
    record_widget(ui, meta, &resp, widget_id, "label", tex, None, Some(json!({ "tex": tex })), true, None, ZLayer::ModalDialog, false, None);
}

fn measure_equation_segments(ui: &Ui, segments: &[EquationSegment], font_size: f32) -> (f32, f32) {
    let mut total_w = 0.0_f32;
    let mut max_h = font_size * 1.15;
    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => {
                for span in &rich.spans {
                    let scale = match span.script {
                        SpanScript::Baseline => 1.0,
                        SpanScript::Superscript | SpanScript::Subscript => 0.72,
                    };
                    let font = FontId::proportional((font_size * scale).max(7.0));
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(span.text.clone(), font, Color32::BLACK));
                    total_w += galley.size().x;
                    max_h = max_h.max(galley.size().y);
                }
            }
            EquationSegment::Fraction { numerator, denominator } => {
                let (nw, nh) = measure_equation_segments(ui, numerator, font_size * 0.85);
                let (dw, dh) = measure_equation_segments(ui, denominator, font_size * 0.85);
                total_w += nw.max(dw) + 8.0;
                max_h = max_h.max(nh + dh + 6.0);
            }
        }
    }
    (total_w, max_h)
}

fn paint_equation_segments(ui: &Ui, painter: &egui::Painter, origin: Pos2, segments: &[EquationSegment], font_size: f32, color: Color32) -> Rect {
    let (total_w, total_h) = measure_equation_segments(ui, segments, font_size);
    let cy = origin.y + total_h * 0.5;
    let mut cx = origin.x;

    for seg in segments {
        match seg {
            EquationSegment::Inline(rich) => {
                for span in &rich.spans {
                    let (scale, dy) = match span.script {
                        SpanScript::Baseline => (1.0, 0.0),
                        SpanScript::Superscript => (0.72, -font_size * 0.28),
                        SpanScript::Subscript => (0.72, font_size * 0.22),
                    };
                    let mut job = egui::text::LayoutJob::single_section(
                        span.text.clone(),
                        egui::text::TextFormat {
                            font_id: FontId::proportional((font_size * scale).max(7.0)),
                            color,
                            italics: span.slope == SpanSlope::Italic,
                            extra_letter_spacing: if span.weight == SpanWeight::Bold { 0.2 } else { 0.0 },
                            ..Default::default()
                        },
                    );
                    job.wrap = egui::text::TextWrapping::no_max_width();
                    let galley = ui.fonts_mut(|f| f.layout_job(job));
                    let w = galley.size().x;
                    let h = galley.size().y;
                    painter.galley(Pos2::new(cx, cy - h * 0.5 + dy), galley, color);
                    cx += w;
                }
            }
            EquationSegment::Fraction { numerator, denominator } => {
                let sub_size = (font_size * 0.85).max(7.0);
                let (nw, nh) = measure_equation_segments(ui, numerator, sub_size);
                let (dw, _dh) = measure_equation_segments(ui, denominator, sub_size);
                let frac_w = nw.max(dw) + 6.0;
                paint_equation_segments(ui, painter, Pos2::new(cx + (frac_w - nw) * 0.5, cy - nh - 2.0), numerator, sub_size, color);
                painter.line_segment([Pos2::new(cx + 1.0, cy), Pos2::new(cx + frac_w - 1.0, cy)], Stroke::new(1.0, color));
                paint_equation_segments(ui, painter, Pos2::new(cx + (frac_w - dw) * 0.5, cy + 2.0), denominator, sub_size, color);
                cx += frac_w + 2.0;
            }
        }
    }
    Rect::from_min_size(origin, Vec2::new(total_w.max(1.0), total_h.max(1.0)))
}

/// Render an Okabe-Ito-derived variable role tag (`NUM`, `CAT`, `LBL`, `FX=`).
pub(super) fn render_mv_role_badge(ui: &mut Ui, tokens: &Tokens, role: VariableRole, inferred: bool) {
    let (tag, col) = match role {
        VariableRole::Continuous => ("NUM", if tokens.dark { Color32::from_rgb(86, 180, 233) } else { Color32::from_rgb(0, 114, 178) }),
        VariableRole::Categorical => ("CAT", Color32::from_rgb(213, 94, 0)),
        VariableRole::Label => ("LBL", tokens.text_muted),
        VariableRole::Calculated => ("FX=", Color32::from_rgb(0, 158, 115)),
    };
    let text = if inferred { format!("{tag}*") } else { String::from(tag) };
    egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, col))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(Vec2::new(4.0, 1.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(10.0).strong().color(col));
        });
}

// ============================================================================
// 1. Analyze... Entry Picker, DuplicatePrompt, ChangeDataAnalyzed (§8.7)
// ============================================================================

/// Render native `egui` `Analyze Data...` modal (`ActiveDialog::AnalyzeSheet`).
pub fn render_analyze_sheet_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::AnalyzeSheet { .. } = &app.canvas_dialogs.active_dialog else {
        ctx.data_mut(|d| d.remove::<bool>(Id::unique("analyze_modal.init_focus")));
        return StatsModalFrameReport::default();
    };
    let az = &app.canvas_dialogs.analyze;
    let doc = app.document();
    let entries = az.compatible_entries();
    let src_name = az
        .source_sheet_id
        .as_ref()
        .and_then(|id| doc.data_sheet(id).map(|ds| ds.title.as_str()).or_else(|| doc.result_sheet(id).map(|rs| rs.title())))
        .unwrap_or("Active Sheet");
    let subtitle = format!("{src_name}  |  {} compatible analyses", entries.len());

    let focus_key = Id::unique("analyze_modal.init_focus");
    if !ctx.data(|d| d.get_temp::<bool>(focus_key)).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(focus_key, true));
        state.analyze_focus_requested = true;
        state.analyze_query_draft = az.search_query.clone();
    } else if state.analyze_query_draft != az.search_query && !ctx.egui_wants_keyboard_input() {
        state.analyze_query_draft = az.search_query.clone();
    }

    let ok_en = evaluate_dialog_button(&app.canvas_dialogs, doc, "ok").unwrap_or_else(ControlEnablement::enabled);
    let locale = normalize_locale(&app.locale);

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.analyze_sheet",
            title: tr(locale, "w6.view.modal.analyze_data"),
            subtitle_or_banner: Some(&subtitle),
            banner_widget_id: "analyze.status_banner",
            max_width_pt: 780.0,
            max_height_pt: 560.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let clear_w = 56.0;
                let edit_w = (ui.available_width() - clear_w - 8.0).max(120.0);
                let search_id = Id::unique("analyze.search.input");
                let search_resp = ui.add_sized(
                    Vec2::new(edit_w, 24.0),
                    TextEdit::singleline(&mut state.analyze_query_draft)
                        .id(search_id)
                        .hint_text("Search analyses (e.g., t-test, ANOVA, nonlinear, GLM, PCA)…"),
                );
                if state.analyze_focus_requested {
                    search_resp.request_focus();
                    if !ctx.will_discard() {
                        state.analyze_focus_requested = false;
                    }
                }
                if search_resp.changed() {
                    intents.push_set_analyze_query(state.analyze_query_draft.clone());
                }
                record_widget(
                    ui,
                    meta,
                    &search_resp,
                    "analyze.search",
                    "textbox",
                    "Search analyses",
                    None,
                    Some(json!(state.analyze_query_draft)),
                    true,
                    None,
                    ZLayer::ModalDialog,
                    false,
                    None,
                );

                if render_modal_seg_btn(ui, tokens, meta, "analyze.search.clear", "Clear", false, &ControlEnablement::enabled()) {
                    state.analyze_query_draft.clear();
                    state.analyze_focus_requested = true;
                    intents.push_custom_view_action("analyze_picker", "clear_search");
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if render_modal_seg_btn(ui, tokens, meta, "analyze.cat.all", "All", az.category_filter.is_none(), &ControlEnablement::enabled()) {
                    intents.push_custom_view_action("analyze_picker", "toggle_category:all");
                }
                for (slug, cat) in [
                    ("column", AnalysisCategory::Column),
                    ("grouped", AnalysisCategory::Grouped),
                    ("xy", AnalysisCategory::XyAndRegression),
                    ("contingency", AnalysisCategory::Contingency),
                    ("survival", AnalysisCategory::Survival),
                    ("multivariate", AnalysisCategory::Multivariate),
                    ("transform", AnalysisCategory::Transform),
                ] {
                    let active = az.category_filter == Some(cat);
                    if render_modal_seg_btn(ui, tokens, meta, &format!("analyze.cat.{slug}"), cat.label(), active, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("analyze_picker", format!("toggle_category:{slug}"));
                    }
                }
            });
        },
        |ui, _state, avail_w, _avail_h, meta, intents| {
            let narrow = avail_w < 540.0;
            if narrow {
                render_analyze_entries_pane(ui, app, &entries, tokens, meta, intents);
                ui.add_space(8.0);
                render_analyze_datasets_pane(ui, app, az, tokens, meta, intents);
            } else {
                let left_w = (avail_w * 0.54).max(240.0);
                let right_w = (avail_w - left_w - 10.0).max(180.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_w);
                        render_analyze_entries_pane(ui, app, &entries, tokens, meta, intents);
                    });
                    ui.add_space(10.0);
                    ui.allocate_ui_with_layout(Vec2::new(right_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(right_w);
                        render_analyze_datasets_pane(ui, app, az, tokens, meta, intents);
                    });
                });
            }
        },
        |ui, _state, meta, intents| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ok_en) {
                        intents.push_dialog_button("ok");
                    }
                    if render_modal_action_button(
                        ui,
                        tokens,
                        meta,
                        "modal.cancel",
                        tr(locale, "btn.cancel"),
                        false,
                        false,
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_dialog_button("cancel");
                    }
                });
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) && ok_en.enabled {
        intents.push_dialog_button("ok");
    }
    StatsModalFrameReport::from(capture)
}

fn render_analyze_entries_pane(
    ui: &mut Ui,
    app: &UiApp,
    entries: &[&'static crate::dialogs_canvas::AnalysisCatalogItem],
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Compatible Analyses");
    if entries.is_empty() {
        let msg = app.canvas_dialogs.analyze.empty_feedback().unwrap_or_else(|| String::from("No analyses match."));
        render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "analyze.empty_feedback",
                role: "status",
                text: &msg,
                font_size: 11.5,
                color: tokens.text_muted,
                z_layer: ZLayer::ModalDialog,
                tooltip: Some(&msg),
                record_meta: true,
            },
        );
        return;
    }
    for item in entries {
        let selected = app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some(item.id);
        let en = crate::modal_predicates::evaluate_dialog_widget(&app.canvas_dialogs, app.document(), &format!("analyze.entry.{}", item.id));
        if render_modal_radio(ui, tokens, meta, &format!("analyze.entry.{}", item.id), item.label, selected, &en) {
            intents.push_select_analyze_entry(item.id, false);
        }
    }

    // Matching existing duplicate notice if present
    let matching = app.canvas_dialogs.analyze.matching_existing_analyses(app.document());
    if !matching.is_empty() {
        ui.add_space(6.0);
        let msg = format!("Note: {} matching analysis sheet(s) already exist for this source.", matching.len());
        render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "analyze.duplicate_hint",
                role: "status",
                text: &msg,
                font_size: 11.0,
                color: tokens.accent_strong,
                z_layer: ZLayer::ModalDialog,
                tooltip: Some(&msg),
                record_meta: true,
            },
        );
    }
}

fn render_analyze_datasets_pane(
    ui: &mut Ui,
    app: &UiApp,
    az: &AnalyzeDialogState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if matches!(az.selected_entry_id.as_deref(), Some("stats.pca" | "stats.pcr" | "pca" | "pcr")) && !az.variable_candidates.is_empty() {
        render_section_title(ui, tokens, "PCA Predictors / PCR Outcome");
        let is_pcr = matches!(az.selected_entry_id.as_deref(), Some("stats.pcr" | "pcr"));
        for var in &az.variable_candidates {
            let pred_on = az.pca_predictors.contains(&var.col_index);
            ui.horizontal_wrapped(|ui| {
                let pred_lbl = if is_pcr { format!("Pred: {}", var.title) } else { format!("Predictor: {}", var.title) };
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.pca.pred.{}", var.col_index),
                    &pred_lbl,
                    pred_on,
                    &ControlEnablement::enabled(),
                ) {
                    intents.push_custom_view_action("analyze_picker", format!("toggle_pca_pred:{}", var.col_index));
                }
                if is_pcr {
                    let pcr_en = r10_pcr_outcome_column(var.col_index, &az.pca_predictors);
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("analyze.pcr.outcome.{}", var.col_index),
                        "Outcome (Y)",
                        az.pcr_outcome_column == Some(var.col_index),
                        &pcr_en,
                    ) {
                        intents.push_custom_view_action("analyze_picker", format!("set_pcr_outcome:{}", var.col_index));
                    }
                }
            });
        }
        ui.add_space(6.0);
    }

    if !az.variable_candidates.is_empty() {
        render_section_title(ui, tokens, "Variables & Role Filter");
        ui.horizontal_wrapped(|ui| {
            for (slug, lbl, target) in [
                ("all", "All", VariableRoleFilter::All),
                ("continuous", "Continuous", VariableRoleFilter::ContinuousOnly),
                ("categorical", "Categorical", VariableRoleFilter::CategoricalOnly),
                ("label", "Label", VariableRoleFilter::LabelOnly),
                ("calculated", "Calculated", VariableRoleFilter::CalculatedOnly),
            ] {
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.mv.filter.{slug}"),
                    lbl,
                    az.role_filter == target,
                    &ControlEnablement::enabled(),
                ) {
                    intents.push_custom_view_action("analyze_picker", format!("set_mv_role_filter:{slug}"));
                }
            }
        });
        ui.add_space(4.0);
        for var in az.filtered_variables() {
            let checked = az.selected_data_sets.contains(&var.col_index);
            ui.horizontal(|ui| {
                render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.mv.var.{}", var.col_index),
                    &var.title,
                    checked,
                    &ControlEnablement::enabled(),
                ) {
                    intents.push_custom_view_action("analyze_picker", format!("toggle_mv_var:{}", var.col_index));
                }
            });
        }
    } else if let Some(src_id) = az.source_sheet_id.as_ref() {
        render_section_title(ui, tokens, "Data Sets / Columns");
        let titles = column_titles_for_draft(app.document(), Some(src_id));
        let all_selected = az.selected_data_sets.is_empty();
        if render_modal_checkbox(ui, tokens, meta, "analyze.dataset.all", "All Data Sets", all_selected, &ControlEnablement::enabled()) {
            intents.push_set_analyze_selected_datasets(Vec::new());
        }
        for (idx, col_title) in titles.iter().enumerate() {
            let checked = az.selected_data_sets.is_empty() || az.selected_data_sets.contains(&idx);
            if render_modal_checkbox(ui, tokens, meta, &format!("analyze.dataset.{idx}"), col_title, checked, &ControlEnablement::enabled()) {
                intents.push_toggle_analyze_dataset(idx);
            }
        }
    }

    if matches!(az.selected_entry_id.as_deref(), Some("stats.outliers" | "outliers")) {
        ui.add_space(6.0);
        render_section_title(ui, tokens, "Outlier Identification Mode");
        let is_grubbs = az.outlier_method == "grubbs";
        if render_modal_radio(
            ui,
            tokens,
            meta,
            "analyze.outlier.rout",
            "ROUT (Robust regression + FDR Q%)",
            !is_grubbs,
            &ControlEnablement::enabled(),
        ) {
            intents.push_custom_view_action("analyze_picker", "outlier_mode:rout");
        }
        if render_modal_radio(ui, tokens, meta, "analyze.outlier.grubbs", "Grubbs' test (Alpha threshold)", is_grubbs, &ControlEnablement::enabled())
        {
            intents.push_custom_view_action("analyze_picker", "outlier_mode:grubbs");
        }
        let step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "analyze.outlier.param",
                label: if is_grubbs { "Significance Alpha" } else { "Max Desired FDR Q (%)" },
                value_text: &if is_grubbs { format!("{:.3}", az.outlier_alpha) } else { format!("{:.1}%", az.outlier_fdr_q_percent) },
                enablement: &ControlEnablement::enabled(),
            },
        );
        if step < 0 {
            intents.push_custom_view_action("analyze_picker", if is_grubbs { "outlier_param_step:-0.01" } else { "outlier_param_step:-1" });
        } else if step > 0 {
            intents.push_custom_view_action("analyze_picker", if is_grubbs { "outlier_param_step:0.01" } else { "outlier_param_step:1" });
        }
        if is_grubbs
            && render_modal_checkbox(
                ui,
                tokens,
                meta,
                "analyze.outlier.iterative",
                "Iterative Grubbs (until no outlier)",
                az.outlier_iterative,
                &ControlEnablement::enabled(),
            )
        {
            intents.push_custom_view_action("analyze_picker", "outlier_toggle_iterative");
        }
    }
}

/// Render `DuplicateAnalysisPrompt` disambiguation modal.
pub fn render_duplicate_analysis_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let _ = &state.active_tab;
    let locale = normalize_locale(&app.locale);
    let shard = &app.view_shards.analyze_picker;
    let doc = app.document();

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.duplicate_analysis",
            title: tr(locale, "w6.view.modal.analysis_already_exists"),
            subtitle_or_banner: Some("An analysis of this type already exists on this source sheet."),
            banner_widget_id: "analyze_picker.dup.banner",
            max_width_pt: 520.0,
            max_height_pt: 380.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _state, _w, _h, meta, intents| {
            let is_create = shard.duplicate_choice == DuplicateChoice::CreateNew;
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "analyze_picker.dup.create_new",
                "Create a new ResultSheet",
                is_create,
                &ControlEnablement::enabled(),
            ) {
                intents.push_custom_view_action("analyze_picker", "dup_choice:create_new");
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "analyze_picker.dup.modify_existing",
                "Modify existing analysis parameters & results",
                !is_create,
                &ControlEnablement::enabled(),
            ) {
                intents.push_custom_view_action("analyze_picker", "dup_choice:modify_existing");
            }

            if !is_create && !existing_result_ids.is_empty() {
                ui.add_space(6.0);
                render_section_title(ui, tokens, "Select Existing ResultSheet");
                let selected_id = shard.duplicate_target_id.as_deref().or_else(|| existing_result_ids.first().map(SheetId::as_str)).unwrap_or("");
                for (idx, rid) in existing_result_ids.iter().enumerate() {
                    let lbl = doc.result_sheet(rid).map_or_else(|| rid.as_str().to_owned(), |rs| format!("{} ({})", rs.title(), rid.as_str()));
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("analyze_picker.dup.target.{idx}"),
                        &lbl,
                        selected_id == rid.as_str(),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_custom_view_action("analyze_picker", format!("dup_target:{}", rid.as_str()));
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }
    StatsModalFrameReport::from(capture)
}

/// Render `Change Data Analyzed...` retargeting modal.
pub fn render_change_data_analyzed_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ChangeDataAnalyzed { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let _ = &state.active_tab;
    let cd = &app.canvas_dialogs.change_data;
    let doc = app.document();
    let rs_title = doc.result_sheet(result_sheet_id).map_or("Result", |rs| rs.title());
    let subtitle = format!("Retargeting `{rs_title}` ({} candidate tables)", cd.candidates.len());
    let ok_en = cd
        .selected_target_id
        .as_ref()
        .map(|_| ControlEnablement::enabled())
        .unwrap_or_else(|| ControlEnablement::disabled("Select a compatible source DataSheet."));
    let locale = normalize_locale(&app.locale);

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.change_data_analyzed",
            title: tr(locale, "btn.change_data_analyzed"),
            subtitle_or_banner: Some(&subtitle),
            banner_widget_id: "change_data.status_banner",
            max_width_pt: 620.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _state, _w, _h, meta, intents| {
            render_section_title(ui, tokens, "Compatible Source DataSheets");
            if cd.candidates.is_empty() {
                render_clipped_text(
                    ui,
                    meta,
                    ClippedTextSpec {
                        widget_id: "change_data.empty",
                        role: "status",
                        text: "No DataSheets available in document.",
                        font_size: 11.5,
                        color: tokens.text_muted,
                        z_layer: ZLayer::ModalDialog,
                        tooltip: None,
                        record_meta: true,
                    },
                );
            } else {
                for (idx, cand) in cd.candidates.iter().enumerate() {
                    let selected = cd.selected_target_id.as_ref() == Some(&cand.sheet_id);
                    let en = if cand.compatible {
                        ControlEnablement::enabled()
                    } else {
                        ControlEnablement::disabled(cand.incompatible_reason.as_deref().unwrap_or("Incompatible table archetype"))
                    };
                    let (rows, cols, _) = source_table_shape(doc, Some(&cand.sheet_id));
                    let lbl = format!("{} [{} — {}×{}]", cand.title, cand.table_kind.label(), rows, cols);
                    if render_modal_radio(ui, tokens, meta, &format!("change_data.candidate.{idx}"), &lbl, selected, &en) {
                        intents.push_select_retarget_source(cand.sheet_id.clone());
                    }
                }
            }

            if let Some(target_id) = &cd.selected_target_id {
                let titles = column_titles_for_draft(doc, Some(target_id));
                if !titles.is_empty() {
                    ui.add_space(8.0);
                    render_section_title(ui, tokens, "Columns Included After Retargeting");
                    if render_modal_checkbox(
                        ui,
                        tokens,
                        meta,
                        "change_data.col.all",
                        "All Columns",
                        cd.selected_data_sets.is_empty(),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut fields = Map::new();
                        fields.insert(String::from("selected_data_sets"), Value::Array(Vec::new()));
                        intents.push_patch_active_dialog(fields);
                    }
                    for (ci, col_name) in titles.iter().enumerate() {
                        let on = cd.selected_data_sets.is_empty() || cd.selected_data_sets.contains(&ci);
                        if render_modal_checkbox(ui, tokens, meta, &format!("change_data.col.{ci}"), col_name, on, &ControlEnablement::enabled()) {
                            let mut next = if cd.selected_data_sets.is_empty() { (0..titles.len()).collect() } else { cd.selected_data_sets.clone() };
                            if let Some(pos) = next.iter().position(|&c| c == ci) {
                                next.remove(pos);
                            } else {
                                next.push(ci);
                                next.sort_unstable();
                            }
                            let mut fields = Map::new();
                            fields.insert(String::from("selected_data_sets"), Value::Array(next.into_iter().map(|i| json!(i)).collect()));
                            intents.push_patch_active_dialog(fields);
                        }
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ok_en) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }
    StatsModalFrameReport::from(capture)
}

// ============================================================================
// 2. Generalized Statistical / GLM / Cox / PCA / PCR / Clustering / Survival Modals (§8.8)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatTabDef {
    slug: &'static str,
    label: &'static str,
}

fn tabs_for_spec(app: &UiApp, spec: &curvebook_doc::AnalysisSpec) -> Vec<StatTabDef> {
    let is_mv = app.document().data_sheet(&spec.source_sheet_id).is_some_and(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_)));
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(_) => vec![
            StatTabDef { slug: "experimental_design", label: "Design & Test" },
            StatTabDef { slug: "repeated_measures", label: "Repeated Measures" },
            StatTabDef { slug: "options", label: "Options" },
            StatTabDef { slug: "diagnostics", label: "Diagnostics" },
        ],
        AnalysisKind::FactorialAnova(_) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: "Data" });
            }
            v.push(StatTabDef { slug: "experimental_design", label: "Design & Model" });
            v.push(StatTabDef { slug: "repeated_measures", label: "Repeated Measures" });
            v.push(StatTabDef { slug: "multiple_comparisons", label: "Comparisons" });
            v.push(StatTabDef { slug: "options", label: "Options" });
            v
        }
        AnalysisKind::NestedComparison(_) | AnalysisKind::MultipleRowTTests(_) => vec![
            StatTabDef { slug: "experimental_design", label: "Design & Data" },
            StatTabDef { slug: "multiple_comparisons", label: "Multiple Comparisons" },
            StatTabDef { slug: "options", label: "Options" },
        ],
        AnalysisKind::GeneralizedLinearModel(_)
        | AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. })
        | AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { .. }) => vec![
            StatTabDef { slug: "data", label: "Data" },
            StatTabDef { slug: "experimental_design", label: "Model & Formula" },
            StatTabDef { slug: "options", label: "Options" },
            StatTabDef { slug: "diagnostics", label: "Diagnostics" },
        ],
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: "Data" });
            }
            v.push(StatTabDef { slug: "experimental_design", label: "Survival Tests" });
            v.push(StatTabDef { slug: "multiple_comparisons", label: "Pairwise Log-Rank" });
            v.push(StatTabDef { slug: "options", label: "Confidence & Options" });
            v
        }
        AnalysisKind::LinearRegression(_) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: "Data" });
            }
            v.push(StatTabDef { slug: "experimental_design", label: "Parameters" });
            v.push(StatTabDef { slug: "range", label: "Range & Curve" });
            v.push(StatTabDef { slug: "options", label: "Options" });
            v.push(StatTabDef { slug: "diagnostics", label: "Diagnostics" });
            v
        }
        _ => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: "Data" });
            }
            v.push(StatTabDef { slug: "experimental_design", label: "Parameters" });
            v.push(StatTabDef { slug: "options", label: "Options" });
            v.push(StatTabDef { slug: "diagnostics", label: "Diagnostics" });
            v
        }
    }
}

fn resolve_survival_banner(surv: &SurvivalAnalysisSpec) -> (String, String) {
    match surv {
        SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, gehan_breslow_wilcoxon, trend_test, .. } => {
            let mut tests = Vec::new();
            if *log_rank_mantel_cox {
                tests.push("Mantel-Cox");
            }
            if *gehan_breslow_wilcoxon {
                tests.push("Gehan-Wilcoxon");
            }
            if *trend_test {
                tests.push("Log-Rank Trend");
            }
            let summary = if tests.is_empty() { String::from("KM curves only") } else { tests.join(" + ") };
            (String::from("Parameters: Kaplan-Meier Survival"), format!("Product-limit KM ({summary})"))
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, predictor_columns, .. } => {
            (String::from("Parameters: Cox Proportional Hazards"), format!("Cox PH ({} covariates, {:?} ties)", predictor_columns.len(), tie_method))
        }
    }
}

fn resolve_multivariate_banner(mv: &MultivariateSpec, alpha: f64) -> (String, String) {
    match mv {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => (
            String::from("Parameters: Principal Component Analysis (PCA)"),
            format!(
                "{}{}",
                if *standardize { "Correlation matrix (standardized)" } else { "Covariance matrix (centered)" },
                if parallel_analysis_seed.is_some() { " + Horn parallel analysis" } else { "" }
            ),
        ),
        MultivariateSpec::CorrelationMatrix { spearman_rank } => (
            String::from("Parameters: Correlation Matrix"),
            format!("{} pairwise matrix (α={alpha:.3})", if *spearman_rank { "Spearman rank r_s" } else { "Pearson product-moment r" }),
        ),
        MultivariateSpec::Clustering { k_means_k, distance, linkage, .. } => (
            String::from("Parameters: Clustering & Dendrogram"),
            format!("{:?} / {:?}{}", distance, linkage, k_means_k.map_or_else(String::new, |k| format!(" + k-means (k={k})"))),
        ),
        MultivariateSpec::MultipleImputation { imputations_m, response_column, .. } => (
            String::from("Parameters: Multiple Imputation (Rubin MICE)"),
            format!("m={imputations_m} imputations{}", if response_column.is_some() { " + pooled GLM" } else { "" }),
        ),
    }
}

/// Resolve the live dynamic header title and test banner for any `AnalysisSpec`.
#[must_use]
pub fn resolve_stat_params_banner(app: &UiApp, spec: &curvebook_doc::AnalysisSpec) -> (String, String) {
    let conf = spec.confidence_level.percent();
    let alpha = (1.0 - conf / 100.0).max(0.0001);
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            let tail = if t.two_tailed { "two-tailed" } else { "one-tailed" };
            let m = match t.method {
                TwoGroupMethod::UnpairedT => "Unpaired Student t-test",
                TwoGroupMethod::WelchT => "Welch's unpaired t-test",
                TwoGroupMethod::PairedT => "Paired Student t-test",
                TwoGroupMethod::RatioPairedT => "Ratio paired t-test",
                TwoGroupMethod::LognormalUnpairedT => "Lognormal unpaired t-test",
                TwoGroupMethod::MannWhitneyU => "Mann-Whitney U rank test",
                TwoGroupMethod::WilcoxonSignedRank => "Wilcoxon signed-rank test",
                TwoGroupMethod::KolmogorovSmirnov => "Kolmogorov-Smirnov test",
                TwoGroupMethod::OneSampleT { .. } => "One-sample t-test",
                TwoGroupMethod::OneSampleWilcoxon { .. } => "One-sample Wilcoxon test",
                TwoGroupMethod::PairedSignTest => "Paired binomial sign test",
            };
            (String::from("Parameters: Two-Group / One-Sample Test"), format!("{m} ({tail}, α={alpha:.3})"))
        }
        AnalysisKind::FactorialAnova(f) => {
            let kind = if f.three_way {
                "Three-way"
            } else if f.multifactor_columns.len() >= 2 {
                "Multifactor N-way"
            } else {
                "Two-way"
            };
            let ss = match f.ss_type {
                SumOfSquaresType::TypeI => "Type I SS",
                SumOfSquaresType::TypeIII => "Type III SS",
            };
            let rm = if f.mixed_effects_reml || is_reml_mixed_effects_active(&app.canvas_dialogs.edit_params, app.document()) {
                "REML mixed-effects"
            } else {
                match f.repeated_measures {
                    RepeatedMeasuresDesign::None => "Between-subjects",
                    RepeatedMeasuresDesign::RowFactorMatched => "Row-matched RM",
                    RepeatedMeasuresDesign::ColumnFactorMatched => "Col-matched RM",
                    RepeatedMeasuresDesign::BothFactorsMatched => "Doubly RM",
                }
            };
            (String::from("Parameters: Factorial / Multifactor ANOVA"), format!("{kind} ANOVA ({rm}, {ss})"))
        }
        AnalysisKind::NestedComparison(n) => (
            String::from("Parameters: Nested Comparison"),
            format!("{} ({})", if n.two_group_t_test { "Nested t-test" } else { "Nested 1-way ANOVA" }, df_method_label(n.df_method)),
        ),
        AnalysisKind::MultipleRowTTests(m) => {
            let test = if m.nonparametric {
                "Nonparametric rank"
            } else if m.paired {
                "Paired t"
            } else if m.assume_equal_sd {
                "Student t"
            } else {
                "Welch t"
            };
            (String::from("Parameters: Multiple Row t-Tests"), format!("{test} row tests + {}", multiplicity_short_label(m.multiplicity.method)))
        }
        AnalysisKind::Contingency(c) => (
            String::from("Parameters: Contingency / Categorical"),
            format!("{}{}", contingency_method_label(c.method), if c.compute_odds_and_risk_ratios { " + OR/RR" } else { "" }),
        ),
        AnalysisKind::RocCurve(r) => {
            let ci = match r.ci_method {
                curvebook_doc::RocCiMethod::DeLong => "DeLong (1988)",
                curvebook_doc::RocCiMethod::HanleyMcNeil => "Hanley-McNeil",
            };
            (String::from("Parameters: ROC Curve Analysis"), format!("AUC + {ci} CI ({conf:.0}%)"))
        }
        AnalysisKind::Survival(surv) => resolve_survival_banner(surv),
        AnalysisKind::GeneralizedLinearModel(glm) => {
            let fam = match glm.family {
                GlmFamily::Linear => "Gaussian Linear GLM (Identity)",
                GlmFamily::Logistic => "Binomial Logistic GLM (Logit)",
                GlmFamily::Poisson => "Poisson Count GLM (Log)",
            };
            (String::from("Parameters: Generalized Linear Model"), format!("{fam}, {} predictors", glm.predictor_columns.len()))
        }
        AnalysisKind::Multivariate(mv) => resolve_multivariate_banner(mv, alpha),
        AnalysisKind::DescriptiveStats(d) => (
            String::from("Parameters: Descriptive Statistics"),
            format!("{:?}{}", d.aggregation, if d.run_normality_battery { " + Normality battery" } else { "" }),
        ),
        AnalysisKind::OutlierDetection(o) => {
            let desc = match o {
                OutlierSpec::Rout { fdr_q_percent } => format!("ROUT (Q = {fdr_q_percent:.2}%)"),
                OutlierSpec::Grubbs { alpha, iterative } => format!("Grubbs (α={alpha:.3}{})", if *iterative { ", iterative" } else { "" }),
            };
            (String::from("Parameters: Identify Outliers"), desc)
        }
        AnalysisKind::BlandAltman(ba) => (
            String::from("Parameters: Bland-Altman Method Comparison"),
            format!("{} ({conf:.0}% LoA)", if ba.percent_difference { "% Difference vs Average" } else { "Difference (A-B) vs Average" }),
        ),
        AnalysisKind::PValueStack(ps) => {
            (String::from("Parameters: Analyze Stack of P Values"), format!("{} (threshold {:.3})", ps.method.label(), ps.threshold))
        }
        AnalysisKind::LinearRegression(lin) => {
            let kind = if lin.deming_model_ii { "Deming Model II orthogonal regression" } else { "Ordinary Least Squares (Model I)" };
            let origin = if lin.force_through_origin && !lin.deming_model_ii { " (through origin)" } else { "" };
            (String::from("Parameters: Simple Linear / Deming Regression"), format!("{kind}{origin}"))
        }
        AnalysisKind::PassingBablok(_) => {
            (String::from("Parameters: Passing-Bablok Regression"), format!("Nonparametric median-slope + Cusum ({conf:.0}% CI)"))
        }
        _ => (String::from("Parameters: Analysis"), format!("{} (α={alpha:.3})", spec.effective_title())),
    }
}

/// Render general Statistical / GLM / Cox / PCA / PCR / Clustering / Survival parameter modals.
pub fn render_stat_params_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        state.open_session_sheet_id = None;
        return StatsModalFrameReport::default();
    };
    let doc = app.document();
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref().or_else(|| doc.result_sheet(result_sheet_id).map(|rs| &rs.analysis)) else {
        state.open_session_sheet_id = None;
        return StatsModalFrameReport::default();
    };

    let (header_title, banner) = resolve_stat_params_banner(app, spec);
    state.last_resolved_banner.clone_from(&banner);
    let tabs = tabs_for_spec(app, spec);
    let ep = &app.canvas_dialogs.edit_params;

    // Sync active tab and transient input drafts cleanly when opening or switching sheets
    if state.open_session_sheet_id.as_deref() != Some(result_sheet_id.as_str()) {
        state.open_session_sheet_id = Some(result_sheet_id.as_str().to_owned());
        state.active_tab = tabs.iter().find(|t| t.slug == ep.active_tab).map_or(tabs[0].slug, |t| t.slug).to_owned();
        state.show_method_reference = false;
        state.method_saved_as_default = false;
        if let AnalysisKind::LinearRegression(lin) = &spec.kind {
            let (lo, hi) = lin.x_range_filter.unwrap_or_default();
            state.lin_x_min_draft = lo.map_or_else(String::new, |v| format!("{v}"));
            state.lin_x_max_draft = hi.map_or_else(String::new, |v| format!("{v}"));
            state.lin_plot_min_draft = lin.plot_x_min.map_or_else(String::new, |v| format!("{v}"));
            state.lin_plot_max_draft = lin.plot_x_max.map_or_else(String::new, |v| format!("{v}"));
            state.lin_segments_draft = lin.plot_segments.map_or_else(String::new, |v| format!("{v}"));
        } else {
            state.lin_x_min_draft.clear();
            state.lin_x_max_draft.clear();
            state.lin_plot_min_draft.clear();
            state.lin_plot_max_draft.clear();
            state.lin_segments_draft.clear();
        }
    } else if !tabs.iter().any(|t| t.slug == state.active_tab) {
        state.active_tab = tabs[0].slug.to_owned();
    }

    let is_create_mode = doc.result_sheet(result_sheet_id).is_none();
    let locale = normalize_locale(&app.locale);
    let ok_lbl = if is_create_mode { "Run Analysis" } else { tr(locale, "btn.ok") };

    let mut report = StatsModalFrameReport::from(show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.edit_analysis_parameters",
            title: &header_title,
            subtitle_or_banner: Some(&banner),
            banner_widget_id: "edit.resolved_test_banner",
            max_width_pt: 780.0,
            max_height_pt: 570.0,
            use_body_scroll: true,
            footer_reserve_pt: 42.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for tab in &tabs {
                    let en = crate::modal_predicates::evaluate_edit_params_tab(ep, tab.slug);
                    let selected = state.active_tab == tab.slug;
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.tab.{}", tab.slug), tab.label, selected, &en) {
                        state.active_tab = tab.slug.to_owned();
                        intents.push_set_edit_params_active_tab(tab.slug);
                    }
                }
            });
            if let Some(formula) = build_live_symbolic_formula(spec, doc) {
                ui.add_space(4.0);
                render_formula_bar(ui, tokens, meta, "edit.formula_bar", "Model Formula", &formula);
            }
        },
        |ui, state, _w, _h, meta, intents| {
            if state.show_method_reference {
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.border))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Vec2::new(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Method Reference & Citations").strong().color(tokens.accent_strong));
                        ui.label(egui::RichText::new(method_reference_text(spec)).small().color(tokens.text_primary));
                    });
                ui.add_space(6.0);
            }
            let tab_slug = state.active_tab.clone();
            render_stat_params_tab_body(ui, state, StatTabBodyCtx { app, ep, spec, tab_slug: &tab_slug }, tokens, meta, intents);
        },
        |ui, state, meta, intents| {
            ui.horizontal(|ui| {
                if render_modal_action_button(
                    ui,
                    tokens,
                    meta,
                    "dialog.button.method_ref",
                    "Method Reference",
                    false,
                    state.show_method_reference,
                    &ControlEnablement::enabled(),
                ) {
                    state.show_method_reference = !state.show_method_reference;
                }
                if render_modal_action_button(
                    ui,
                    tokens,
                    meta,
                    "dialog.button.save_default",
                    "Save as Default",
                    false,
                    state.method_saved_as_default,
                    &ControlEnablement::enabled(),
                ) {
                    state.method_saved_as_default = true;
                    intents.push_save_edit_params_as_default();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", ok_lbl, true, false, &ControlEnablement::enabled()) {
                        intents.push_dialog_button("ok");
                    }
                    if !is_create_mode
                        && doc.result_sheet(result_sheet_id).is_some()
                        && render_modal_action_button(ui, tokens, meta, "modal.apply", "Update Analysis", false, false, &ControlEnablement::enabled())
                    {
                        intents.push_dialog_button("apply");
                    }
                    if render_modal_action_button(
                        ui,
                        tokens,
                        meta,
                        "modal.cancel",
                        tr(locale, "btn.cancel"),
                        false,
                        false,
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_dialog_button("cancel");
                    }
                });
            });
        },
    ));

    // Keyboard navigation: Ctrl+Tab/Ctrl+PageDown/Up between enabled tabs, Escape/Enter
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if state.show_method_reference {
            state.show_method_reference = false;
        } else {
            intents.push_dialog_button("cancel");
        }
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push_dialog_button("ok");
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageDown)) {
        step_stat_tab(ep, &tabs, state, 1, intents);
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageUp)) {
        step_stat_tab(ep, &tabs, state, -1, intents);
    }

    report.resolved_test_banner = banner;
    report
}

fn step_stat_tab(
    ep: &EditAnalysisParametersState,
    tabs: &[StatTabDef],
    state: &mut StatsModalsEguiState,
    dir: isize,
    intents: &mut PendingUiIntents,
) {
    let len = tabs.len() as isize;
    if len == 0 {
        return;
    }
    let cur = tabs.iter().position(|t| t.slug == state.active_tab).unwrap_or(0) as isize;
    for offset in 1..=len {
        let idx = (cur + dir * offset).rem_euclid(len) as usize;
        let cand = tabs[idx].slug;
        if crate::modal_predicates::evaluate_edit_params_tab(ep, cand).enabled {
            state.active_tab = cand.to_owned();
            intents.push_set_edit_params_active_tab(cand);
            return;
        }
    }
}

fn build_live_symbolic_formula(spec: &curvebook_doc::AnalysisSpec, doc: &ProjectDocument) -> Option<String> {
    let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
    let name = |col: usize| titles.get(col).cloned().unwrap_or_else(|| format!("X{}", col + 1));
    match &spec.kind {
        AnalysisKind::FactorialAnova(f) if !f.multifactor_columns.is_empty() => {
            let y = f.response_column.map_or_else(|| String::from("Y"), name);
            let terms: Vec<String> = f.multifactor_columns.iter().map(|&c| name(c)).collect();
            let mut rhs = terms.join(" + ");
            if f.max_interaction_order >= 2 && terms.len() >= 2 {
                rhs.push_str(&format!(" + {}:{}", terms[0], terms[1]));
            }
            if f.max_interaction_order >= 3 && terms.len() >= 3 {
                rhs.push_str(&format!(" + {}:{}:{}", terms[0], terms[1], terms[2]));
            }
            Some(format!("{y} ~ {rhs}"))
        }
        AnalysisKind::GeneralizedLinearModel(glm) => {
            let y = name(glm.response_column);
            let lhs = match glm.family {
                GlmFamily::Linear => y,
                GlmFamily::Logistic => format!("logit(P({y}=1))"),
                GlmFamily::Poisson => format!("log(E[{y}])"),
            };
            let preds = if glm.predictor_columns.is_empty() {
                String::from("1")
            } else {
                glm.predictor_columns.iter().enumerate().map(|(i, &c)| format!("β{}·{}", i + 1, name(c))).collect::<Vec<_>>().join(" + ")
            };
            Some(format!("{lhs} ~ β₀ + {preds}"))
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }) => {
            let preds = if predictor_columns.is_empty() {
                String::from("0")
            } else {
                predictor_columns.iter().enumerate().map(|(i, &c)| format!("β{}·{}", i + 1, name(c))).collect::<Vec<_>>().join(" + ")
            };
            Some(format!("h(t | X) = h₀(t) · exp({preds})"))
        }
        _ => None,
    }
}

struct StatTabBodyCtx<'a> {
    app: &'a UiApp,
    ep: &'a EditAnalysisParametersState,
    spec: &'a curvebook_doc::AnalysisSpec,
    tab_slug: &'a str,
}

fn render_stat_params_tab_body(
    ui: &mut Ui,
    state: &mut StatsModalsEguiState,
    tab_ctx: StatTabBodyCtx<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let StatTabBodyCtx { app, ep, spec, tab_slug } = tab_ctx;
    let doc = app.document();
    match tab_slug {
        "data" => render_stat_mv_data_tab(ui, doc, spec, tokens, meta, intents),
        "range" => render_linear_range_tab(ui, state, spec, tokens, meta, intents),
        "repeated_measures" => render_stat_rm_tab(ui, app, ep, spec, tokens, meta, intents),
        "multiple_comparisons" => render_stat_multiplicity_tab(ui, app, ep, spec, tokens, meta, intents),
        "options" => render_stat_options_tab(ui, spec, tokens, meta, intents),
        "diagnostics" => render_stat_diagnostics_tab(ui, spec, tokens, meta, intents),
        _ => render_stat_design_tab(ui, app, ep, spec, tokens, meta, intents),
    }
}

fn render_linear_range_tab(
    ui: &mut Ui,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !matches!(spec.kind, AnalysisKind::LinearRegression(_)) {
        return;
    }
    render_section_title(ui, tokens, "Fit X Range Filter & Curve Plot Window");
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.x_min",
        "Restrict fit X >= (optional)",
        "auto min",
        &mut state.lin_x_min_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.x_max",
        "Restrict fit X <= (optional)",
        "auto max",
        &mut state.lin_x_max_draft,
        110.0,
    );
    ui.add_space(4.0);
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.plot_min",
        "Plot curve start X (optional)",
        "data min",
        &mut state.lin_plot_min_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.plot_max",
        "Plot curve end X (optional)",
        "data max",
        &mut state.lin_plot_max_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.segments",
        "Curve segments (blank = auto)",
        "auto",
        &mut state.lin_segments_draft,
        110.0,
    );
}

fn render_stat_mv_data_tab(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Some(ds) = doc.data_sheet(&spec.source_sheet_id) else {
        return;
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        return;
    };

    render_section_title(ui, tokens, "Multivariate Variable Role Binding");
    match &spec.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => {
            render_section_title(ui, tokens, "Response (Y) Variable");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.glm.resp.{idx}"),
                        &var.title,
                        glm.response_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                            g.response_column = idx;
                            g.predictor_columns.retain(|&c| c != idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
            ui.add_space(6.0);
            render_section_title(ui, tokens, "Predictor Covariates (X)");
            for (idx, var) in mv.variables.iter().enumerate() {
                let is_resp = glm.response_column == idx;
                let en =
                    if is_resp { ControlEnablement::disabled("Outcome variable cannot also be a predictor.") } else { ControlEnablement::enabled() };
                if render_modal_checkbox(ui, tokens, meta, &format!("edit.glm.pred.{idx}"), &var.title, glm.predictor_columns.contains(&idx), &en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                        if let Some(p) = g.predictor_columns.iter().position(|&c| c == idx) {
                            g.predictor_columns.remove(p);
                        } else {
                            g.predictor_columns.push(idx);
                            g.predictor_columns.sort_unstable();
                        }
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. }) => {
            render_section_title(ui, tokens, "Time-to-Event & Status Indicator");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.time.{idx}"),
                        "Time",
                        *time_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column: tc, .. }) = &mut next.kind {
                            *tc = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.status.{idx}"),
                        "Event (0/1)",
                        *status_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { status_column: sc, .. }) = &mut next.kind {
                            *sc = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    let pred_en = if *time_column == idx || *status_column == idx {
                        ControlEnablement::disabled("Time/status column cannot be a covariate.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.pred.{idx}"),
                        "Covariate",
                        predictor_columns.contains(&idx),
                        &pred_en,
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns: pc, .. }) = &mut next.kind {
                            if let Some(p) = pc.iter().position(|&c| c == idx) {
                                pc.remove(p);
                            } else {
                                pc.push(idx);
                                pc.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::FactorialAnova(f) => {
            render_section_title(ui, tokens, "Outcome (Y) & Categorical Factors");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.fac.resp.{idx}"),
                        "Response (Y)",
                        f.response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            fa.response_column = Some(idx);
                            fa.multifactor_columns.retain(|&c| c != idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    let fac_en = if f.response_column == Some(idx) {
                        ControlEnablement::disabled("Outcome column cannot also be a factor.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.fac.col.{idx}"), "Factor", f.multifactor_columns.contains(&idx), &fac_en)
                    {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            if let Some(p) = fa.multifactor_columns.iter().position(|&c| c == idx) {
                                fa.multifactor_columns.remove(p);
                            } else {
                                fa.multifactor_columns.push(idx);
                                fa.multifactor_columns.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) => {
            render_section_title(ui, tokens, "Kaplan-Meier MV Column Roles");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.time.{idx}"),
                        "Time",
                        *mv_time_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column: c, .. }) = &mut next.kind {
                            *c = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.status.{idx}"),
                        "Status",
                        *mv_status_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_status_column: c, .. }) = &mut next.kind {
                            *c = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.group.{idx}"),
                        "Cohort",
                        *mv_group_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_group_column: c, .. }) = &mut next.kind {
                            *c = if *c == Some(idx) { None } else { Some(idx) };
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. }) => {
            render_section_title(ui, tokens, "Pooled Regression Variables (Optional)");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.mi.resp.{idx}"),
                        "Outcome",
                        *response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                            response_column: rc, predictor_columns: pc, ..
                        }) = &mut next.kind
                        {
                            *rc = if *rc == Some(idx) { None } else { Some(idx) };
                            if *rc == Some(idx) {
                                pc.retain(|&c| c != idx);
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.mi.pred.{idx}"),
                        "Predictor",
                        predictor_columns.contains(&idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { predictor_columns: pc, .. }) = &mut next.kind {
                            if let Some(p) = pc.iter().position(|&c| c == idx) {
                                pc.remove(p);
                            } else {
                                pc.push(idx);
                                pc.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
    }
}

fn render_two_group_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let AnalysisKind::TwoGroupComparison(t) = &spec.kind else {
        return;
    };
    let assume_en = r2_assume_gaussian_allowed(doc, Some(&spec.source_sheet_id));
    let paired_en = r1_paired_or_matched_allowed(doc, Some(&spec.source_sheet_id));
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.assume_gaussian", "Assume Gaussian distribution", ep.assume_gaussian, &assume_en) {
            intents.push_dialog_widget_click("edit.assume_gaussian");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.paired_or_matched", "Paired / Matched design", ep.paired_or_matched, &paired_en) {
            intents.push_dialog_widget_click("edit.paired_or_matched");
        }
    });
    ui.add_space(4.0);
    render_section_title(ui, tokens, "Test Method");
    let welch_en = r4_two_group_welch_correction(t.method);
    let unpaired_nonparam_en = assume_en.clone();
    let paired_nonparam_en = if !assume_en.enabled { assume_en.clone() } else { paired_en.clone() };
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, m, en) in [
            ("edit.tg.unpaired_t", "Unpaired Student t", TwoGroupMethod::UnpairedT, ControlEnablement::enabled()),
            ("edit.tg.welch_t", "Welch's t", TwoGroupMethod::WelchT, welch_en.clone()),
            ("edit.tg.paired_t", "Paired Student t", TwoGroupMethod::PairedT, paired_en.clone()),
            ("edit.tg.ratio_paired", "Ratio paired t", TwoGroupMethod::RatioPairedT, paired_en.clone()),
            ("edit.tg.lognormal", "Lognormal t", TwoGroupMethod::LognormalUnpairedT, ControlEnablement::enabled()),
            ("edit.tg.mann_whitney", "Mann-Whitney U", TwoGroupMethod::MannWhitneyU, unpaired_nonparam_en.clone()),
            ("edit.tg.wilcoxon", "Wilcoxon signed-rank", TwoGroupMethod::WilcoxonSignedRank, paired_nonparam_en.clone()),
            ("edit.tg.ks", "Kolmogorov-Smirnov", TwoGroupMethod::KolmogorovSmirnov, unpaired_nonparam_en),
            ("edit.tg.sign", "Paired binomial sign", TwoGroupMethod::PairedSignTest, paired_nonparam_en),
        ] {
            let active = std::mem::discriminant(&t.method) == std::mem::discriminant(&m);
            if render_modal_radio(ui, tokens, meta, wid, lbl, active, &en) {
                let mut next = spec.clone();
                if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                    tg.method = m;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
    ui.add_space(4.0);
    let diff_en = r4_two_group_graph_differences(t.method);
    let np_en = r4_two_group_nonparametric_options(t.method);
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.tg.two_tailed", "Two-tailed P value", t.two_tailed, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                tg.two_tailed = !t.two_tailed;
            }
            intents.push_set_edit_params_draft(next);
        }
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.compute_effect_sizes",
            "Effect sizes (Cohen's d)",
            t.compute_effect_sizes,
            &ControlEnablement::enabled(),
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                tg.compute_effect_sizes = !t.compute_effect_sizes;
            }
            intents.push_set_edit_params_draft(next);
        }
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.two_group.welch",
            "Welch's correction",
            ep.welch_correction || t.method == TwoGroupMethod::WelchT,
            &welch_en,
        ) {
            intents.push_dialog_widget_click("edit.two_group.welch");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_differences", "Graph differences", ep.graph_differences, &diff_en) {
            intents.push_dialog_widget_click("edit.two_group.graph_differences");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_ranks", "Graph ranks", ep.graph_ranks, &np_en) {
            intents.push_dialog_widget_click("edit.two_group.graph_ranks");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.hodges_lehmann", "Hodges-Lehmann CI", ep.hodges_lehmann_ci, &np_en) {
            intents.push_dialog_widget_click("edit.two_group.hodges_lehmann");
        }
    });
    if let Some(ds) = doc.data_sheet(&spec.source_sheet_id)
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "Multiple Variables Grouping & Outcome Roles");
        egui::Grid::new("edit_tg_mv_grid").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.two_group.group.{idx}"),
                        "Grp",
                        t.mv_group_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                            tg.mv_group_column = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.two_group.resp.{idx}"),
                        "Y",
                        t.mv_response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                            tg.mv_response_column = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
                if idx % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    }
    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_factorial_anova_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let AnalysisKind::FactorialAnova(f) = &spec.kind else {
        return;
    };
    let paired_en = r1_paired_or_matched_allowed(doc, Some(&spec.source_sheet_id));
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.paired_or_matched",
        tr(tokens.locale, "stat_params.label.paired_matched"),
        ep.paired_or_matched,
        &paired_en,
    ) {
        intents.push_dialog_widget_click("edit.paired_or_matched");
    }
    ui.add_space(4.0);
    render_section_title(ui, tokens, "Factorial & Sum of Squares Structure");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, target) in [
            ("edit.fac.ss_type_iii", "Type III SS (Marginal)", SumOfSquaresType::TypeIII),
            ("edit.fac.ss_type_i", "Type I SS (Sequential)", SumOfSquaresType::TypeI),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, f.ss_type == target, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.ss_type = target;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (ord, lbl) in [(1usize, "Main effects only"), (2, "Up to 2-way interactions"), (3, "Up to 3-way interactions")] {
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                &format!("edit.fac.order.{ord}"),
                lbl,
                f.max_interaction_order == ord,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.max_interaction_order = ord;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.compute_effect_sizes",
        "Report partial eta-squared & omega-squared",
        f.compute_effect_sizes,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
            fa.compute_effect_sizes = !f.compute_effect_sizes;
        }
        intents.push_set_edit_params_draft(next);
    }
    if let Some(ds) = doc.data_sheet(&spec.source_sheet_id)
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "Outcome (Y) & Categorical Factors");
        egui::Grid::new("edit_fac_mv_grid").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.fac.resp.{idx}"),
                        "Y",
                        f.response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            fa.response_column = Some(idx);
                            fa.multifactor_columns.retain(|&c| c != idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    let fac_en = if f.response_column == Some(idx) {
                        ControlEnablement::disabled("Outcome column cannot also be a factor.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.fac.col.{idx}"), "Fac", f.multifactor_columns.contains(&idx), &fac_en) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            if let Some(p) = fa.multifactor_columns.iter().position(|&c| c == idx) {
                                fa.multifactor_columns.remove(p);
                            } else {
                                fa.multifactor_columns.push(idx);
                                fa.multifactor_columns.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
                if idx % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    }
    ui.add_space(6.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_survival_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    surv: &SurvivalAnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    match surv {
        SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. } => {
            render_section_title(ui, tokens, "Tied Failure Likelihood Handling");
            for (wid, lbl, tm) in [
                ("edit.cox.efron", "Efron approximation (recommended default)", CoxTieMethod::Efron),
                ("edit.cox.breslow", "Breslow partial likelihood", CoxTieMethod::Breslow),
                ("edit.cox.exact", "Exact discrete conditional likelihood", CoxTieMethod::Exact),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, *tie_method == tm, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: tm_mut, .. }) = &mut next.kind {
                        *tm_mut = tm;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            asymmetrical_greenwood_ci,
            mv_group_column,
            ..
        } => {
            render_section_title(ui, tokens, "Survival Curve Comparison Tests");
            let grp_count = survival_group_count(doc, Some(&spec.source_sheet_id), *mv_group_column);
            let trend_en = r_km_trend_test(grp_count);
            for (wid, lbl, checked, en, idx) in [
                ("edit.km.mantel_cox", "Log-rank (Mantel-Cox) test", *log_rank_mantel_cox, ControlEnablement::enabled(), 0u8),
                ("edit.km.gehan", "Gehan-Breslow-Wilcoxon test", *gehan_breslow_wilcoxon, ControlEnablement::enabled(), 1),
                ("edit.km.trend", "Log-rank test for trend (>= 3 cohorts)", *trend_test, trend_en, 2),
                ("edit.km.asym_ci", "Asymmetrical ln(-ln S(t)) Greenwood CI bounds", *asymmetrical_greenwood_ci, ControlEnablement::enabled(), 3),
            ] {
                if render_modal_checkbox(ui, tokens, meta, wid, lbl, checked, &en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                        log_rank_mantel_cox: mc,
                        gehan_breslow_wilcoxon: gb,
                        trend_test: tt,
                        asymmetrical_greenwood_ci: ac,
                        ..
                    }) = &mut next.kind
                    {
                        match idx {
                            0 => *mc = !*mc,
                            1 => *gb = !*gb,
                            2 => *tt = !*tt,
                            _ => *ac = !*ac,
                        }
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            ui.add_space(6.0);
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
    }
}

fn render_contingency_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    c: &curvebook_doc::ContingencySpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let (rows, cols, _) = source_table_shape(doc, Some(&spec.source_sheet_id));
    let two_by_two_en = r5_contingency_2x2_effect_sizes(rows, cols);
    let trend_en = r5_contingency_cochran_armitage_trend(rows, cols);
    render_section_title(ui, tokens, "Categorical Test & Effect Sizes");
    for (wid, lbl, m, en) in [
        ("edit.cont.fisher", "Fisher's exact test", ContingencyMethod::FisherExact, ControlEnablement::enabled()),
        ("edit.cont.chisq", "Pearson Chi-square test", ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ControlEnablement::enabled()),
        ("edit.cont.yates", "Chi-square + Yates continuity", ContingencyMethod::PearsonChiSquare { yates_continuity: true }, two_by_two_en.clone()),
        ("edit.cont.trend", "Cochran-Armitage linear trend", ContingencyMethod::CochranArmitageTrend, trend_en),
        ("edit.cont.mcnemar", "McNemar's paired test", ContingencyMethod::McNemarPaired, two_by_two_en.clone()),
        (
            "edit.cont.kappa",
            "Cohen's Kappa inter-rater agreement",
            ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
            ControlEnablement::enabled(),
        ),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, c.method == m, &en) {
            let mut next = spec.clone();
            if let AnalysisKind::Contingency(cg) = &mut next.kind {
                cg.method = m;
            }
            intents.push_set_edit_params_draft(next);
        }
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.cont.or_rr",
        "Odds Ratio / Relative Risk + Attributable Risk",
        c.compute_odds_and_risk_ratios,
        &two_by_two_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Contingency(cg) = &mut next.kind {
            cg.compute_odds_and_risk_ratios = !c.compute_odds_and_risk_ratios;
        }
        intents.push_set_edit_params_draft(next);
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.cont.sens_spec",
        "Diagnostic Sensitivity / Specificity / PPV / NPV",
        c.compute_sensitivity_specificity,
        &two_by_two_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Contingency(cg) = &mut next.kind {
            cg.compute_sensitivity_specificity = !c.compute_sensitivity_specificity;
        }
        intents.push_set_edit_params_draft(next);
    }
}

fn render_misc_stats_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    match &spec.kind {
        AnalysisKind::NestedComparison(n) => {
            render_section_title(ui, tokens, "Nested Model Structure");
            for (wid, lbl, val) in
                [("edit.nested.t", "Nested t-test (2 groups)", true), ("edit.nested.anova", "Nested One-Way REML ANOVA (k >= 2)", false)]
            {
                if render_modal_radio(ui, tokens, meta, wid, lbl, n.two_group_t_test == val, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::NestedComparison(ns) = &mut next.kind {
                        ns.two_group_t_test = val;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            render_df_method_picker(ui, spec, n.df_method, tokens, meta, intents);
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::MultipleRowTTests(m) => {
            render_section_title(ui, tokens, "Row-Wise Comparison Test");
            for (wid, lbl, checked, idx) in [
                ("edit.row_t.paired", "Paired comparisons per row", m.paired, 0u8),
                ("edit.row_t.nonparam", "Nonparametric rank test (Mann-Whitney / Wilcoxon)", m.nonparametric, 1),
            ] {
                if render_modal_checkbox(ui, tokens, meta, wid, lbl, checked, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::MultipleRowTTests(mr) = &mut next.kind {
                        if idx == 0 {
                            mr.paired = !m.paired;
                        } else {
                            mr.nonparametric = !m.nonparametric;
                        }
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if !m.nonparametric
                && !m.paired
                && render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    "edit.row_t.equal_sd",
                    "Assume equal SD (pooled Student t)",
                    m.assume_equal_sd,
                    &ControlEnablement::enabled(),
                )
            {
                let mut next = spec.clone();
                if let AnalysisKind::MultipleRowTTests(mr) = &mut next.kind {
                    mr.assume_equal_sd = !m.assume_equal_sd;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        AnalysisKind::DescriptiveStats(d) => {
            let (_, _, max_sub) = source_table_shape(doc, Some(&spec.source_sheet_id));
            let sub_en = r8_subcolumn_aggregation(max_sub);
            render_section_title(ui, tokens, "Replicate Aggregation & Distribution Tests");
            for (wid, lbl, agg) in [
                ("edit.desc.row_means", "Collapse subcolumns by row mean", SubcolumnAggregation::RowMeans),
                ("edit.desc.per_subcol", "Separate summary per subcolumn", SubcolumnAggregation::PerSubcolumn),
                ("edit.desc.pooled", "Pool all replicate cells", SubcolumnAggregation::PooledGrandSample),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, d.aggregation == agg, &sub_en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                        ds.aggregation = agg;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.desc.normality",
                "Run Shapiro-Wilk, D'Agostino-Pearson, AD & KS normality tests",
                d.run_normality_battery,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                    ds.run_normality_battery = !d.run_normality_battery;
                }
                intents.push_set_edit_params_draft(next);
            }
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::OutlierDetection(out) => {
            render_outlier_design(ui, doc, spec, out, tokens, meta, intents);
        }
        AnalysisKind::BlandAltman(ba) => {
            render_section_title(ui, tokens, "Bland-Altman Comparison");
            for (wid, lbl, pct) in
                [("edit.ba.diff", "Difference (A - B) vs Average", false), ("edit.ba.pct", "Percentage difference 100*(A-B)/Mean", true)]
            {
                if render_modal_radio(ui, tokens, meta, wid, lbl, ba.percent_difference == pct, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::BlandAltman(b) = &mut next.kind {
                        b.percent_difference = pct;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        AnalysisKind::PValueStack(ps) => {
            render_section_title(ui, tokens, "Multiplicity / FDR Correction Procedure");
            for m in PValueStackMethod::ALL {
                if render_modal_radio(ui, tokens, meta, &format!("edit.pstack.{m:?}"), m.label(), ps.method == m, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::PValueStack(p) = &mut next.kind {
                        p.method = m;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.pstack.threshold",
                    label: if ps.method.is_fdr() { "Desired FDR Q" } else { "FWER Alpha" },
                    value_text: &format!("{:.3}", ps.threshold),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::PValueStack(p) = &mut next.kind {
                    p.threshold = (p.threshold + if step > 0 { 0.01 } else { -0.01 }).clamp(0.001, 0.50);
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        AnalysisKind::LinearRegression(lin) => {
            render_linear_regression_design(ui, doc, spec, lin, tokens, meta, intents);
        }
        AnalysisKind::PassingBablok(pb) => {
            render_section_title(ui, tokens, "Method Comparison Columns (X Reference vs Y Candidate)");
            let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
            for (idx, t) in titles.iter().enumerate() {
                ui.horizontal(|ui| {
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.pb.x.{idx}"),
                        &format!("Ref X: {t}"),
                        pb.x_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::PassingBablok(p) = &mut next.kind {
                            p.x_column = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.pb.y.{idx}"),
                        &format!("Method Y: {t}"),
                        pb.y_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::PassingBablok(p) = &mut next.kind {
                            p.y_column = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::RocCurve(r) => {
            render_section_title(ui, tokens, "ROC AUC Confidence Interval Estimator");
            for (wid, lbl, method) in [
                ("edit.roc.delong", "DeLong et al. (1988) structural components", curvebook_doc::RocCiMethod::DeLong),
                ("edit.roc.hanley", "Hanley & McNeil (1982) approximation", curvebook_doc::RocCiMethod::HanleyMcNeil),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, r.ci_method == method, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::RocCurve(rc) = &mut next.kind {
                        rc.ci_method = method;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
    }
}

fn render_outlier_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    out: &OutlierSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Outlier Algorithm");
    let is_rout = matches!(out, OutlierSpec::Rout { .. });
    if render_modal_radio(ui, tokens, meta, "edit.outlier.rout", "ROUT (Robust regression + FDR Q)", is_rout, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 });
        intents.push_set_edit_params_draft(next);
    }
    if render_modal_radio(ui, tokens, meta, "edit.outlier.grubbs", "Grubbs' ESD test (Alpha)", !is_rout, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false });
        intents.push_set_edit_params_draft(next);
    }
    match out {
        OutlierSpec::Rout { fdr_q_percent } => {
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.outlier.q",
                    label: "Max Desired FDR Q (%)",
                    value_text: &format!("{fdr_q_percent:.2}%"),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: q }) = &mut next.kind {
                    *q = (*q + if step > 0 { 0.5 } else { -0.5 }).clamp(0.1, 10.0);
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        OutlierSpec::Grubbs { alpha, iterative } => {
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.outlier.alpha",
                    label: "Significance Alpha",
                    value_text: &format!("{alpha:.3}"),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: a, .. }) = &mut next.kind {
                    *a = (*a + if step > 0 { 0.01 } else { -0.01 }).clamp(0.001, 0.25);
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.outlier.iterative",
                "Iterative outlier removal",
                *iterative,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { iterative: it, .. }) = &mut next.kind {
                    *it = !*it;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_linear_regression_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    lin: &curvebook_doc::LinearRegressionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Regression Model");
    for (wid, lbl, deming) in
        [("edit.lin.ols", "Ordinary Least Squares (Model I)", false), ("edit.lin.deming", "Deming Model II (Error in both X and Y)", true)]
    {
        if render_modal_radio(ui, tokens, meta, wid, lbl, lin.deming_model_ii == deming, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                l.deming_model_ii = deming;
            }
            intents.push_set_edit_params_draft(next);
        }
    }
    let origin_en = if lin.deming_model_ii {
        ControlEnablement::disabled("Force through origin is unavailable in Deming Model II.")
    } else {
        ControlEnablement::enabled()
    };
    if render_modal_checkbox(ui, tokens, meta, "edit.lin.origin", "Force line through origin (X=0, Y=0)", lin.force_through_origin, &origin_en) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.force_through_origin = !lin.force_through_origin;
        }
        intents.push_set_edit_params_draft(next);
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.lin.interp",
        "Interpolate unknowns from standard curve",
        lin.interpolate_unknowns,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.interpolate_unknowns = !lin.interpolate_unknowns;
        }
        intents.push_set_edit_params_draft(next);
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_stat_design_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(_) => render_two_group_design(ui, doc, ep, spec, tokens, meta, intents),
        AnalysisKind::FactorialAnova(_) => render_factorial_anova_design(ui, doc, ep, spec, tokens, meta, intents),
        AnalysisKind::GeneralizedLinearModel(glm) => {
            render_section_title(ui, tokens, "Exponential Family & Link Function");
            for (wid, lbl, fam) in [
                ("edit.glm.linear", "Gaussian (Identity link: Y = Xβ)", GlmFamily::Linear),
                ("edit.glm.logistic", "Binomial Logistic (Logit link: odds ratios)", GlmFamily::Logistic),
                ("edit.glm.poisson", "Poisson Count (Log link: rate ratios)", GlmFamily::Poisson),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, glm.family == fam, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                        g.family = fam;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.glm.compute_vif",
                "Compute Variance Inflation Factors (VIF)",
                glm.compute_vif,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                    g.compute_vif = !glm.compute_vif;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        AnalysisKind::Survival(surv) => render_survival_design(ui, doc, spec, surv, tokens, meta, intents),
        AnalysisKind::Contingency(c) => render_contingency_design(ui, doc, spec, c, tokens, meta, intents),
        AnalysisKind::Multivariate(mv) => render_multivariate_design_controls(ui, doc, spec, mv, tokens, meta, intents),
        _ => render_misc_stats_design(ui, doc, spec, tokens, meta, intents),
    }
}

fn render_multivariate_design_controls(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    mv: &MultivariateSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    match mv {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            render_section_title(ui, tokens, "PCA Matrix & Component Retention");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.pca.corr",
                "Standardize variables (Correlation matrix)",
                *standardize,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: s, .. }) = &mut next.kind {
                    *s = true;
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_radio(ui, tokens, meta, "edit.pca.cov", "Center only (Covariance matrix)", !*standardize, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: s, .. }) = &mut next.kind {
                    *s = false;
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.pca.parallel",
                "Horn's Monte Carlo parallel analysis",
                parallel_analysis_seed.is_some(),
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { parallel_analysis_seed: seed, .. }) = &mut next.kind {
                    *seed = if seed.is_some() { None } else { Some(42) };
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            render_section_title(ui, tokens, "Correlation Coefficient");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.corr.pearson",
                "Pearson r (linear correlation)",
                !*spearman_rank,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: sr }) = &mut next.kind {
                    *sr = false;
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.corr.spearman",
                "Spearman r_s (monotonic rank)",
                *spearman_rank,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: sr }) = &mut next.kind {
                    *sr = true;
                }
                intents.push_set_edit_params_draft(next);
            }
            let heat_en = r7_correlation_heatmap_toggle(true);
            let _ = render_modal_checkbox(ui, tokens, meta, "edit.corr.heatmap", "Generate correlation heatmap graph", true, &heat_en);
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            render_section_title(ui, tokens, "Distance Metric & Linkage");
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, d) in [
                    ("edit.cluster.dist.euclidean", "Euclidean", ClusterDistance::Euclidean),
                    ("edit.cluster.dist.manhattan", "Manhattan", ClusterDistance::Manhattan),
                    ("edit.cluster.dist.pearson", "Pearson (1-r)", ClusterDistance::Pearson),
                    ("edit.cluster.dist.cosine", "Cosine", ClusterDistance::Cosine),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, *distance == d, &ControlEnablement::enabled()) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance: dm, .. }) = &mut next.kind {
                            *dm = d;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, l) in [
                    ("edit.cluster.link.ward", "Ward", ClusterLinkage::Ward),
                    ("edit.cluster.link.complete", "Complete", ClusterLinkage::Complete),
                    ("edit.cluster.link.single", "Single", ClusterLinkage::Single),
                    ("edit.cluster.link.upgma", "UPGMA", ClusterLinkage::Upgma),
                    ("edit.cluster.link.wpgma", "WPGMA", ClusterLinkage::Wpgma),
                    ("edit.cluster.link.centroid", "Centroid", ClusterLinkage::Centroid),
                    ("edit.cluster.link.median", "Median", ClusterLinkage::Median),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, *linkage == l, &ControlEnablement::enabled()) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: lm, .. }) = &mut next.kind {
                            *lm = l;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                }
            });
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.cluster.k_means_auto",
                "Hierarchical only (no fixed K-means)",
                k_means_k.is_none(),
                &ControlEnablement::enabled(),
            ) {
                intents.push_dialog_widget_click("edit.cluster.k_means_auto");
            }
            let k_val = k_means_k.unwrap_or(0);
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.cluster.k_means_k",
                    label: "K-Means Clusters (0 = hierarchical only)",
                    value_text: &k_val.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: km, .. }) = &mut next.kind {
                    let next_k = if step > 0 {
                        k_val.saturating_add(1).clamp(2, 12)
                    } else if k_val <= 2 {
                        0
                    } else {
                        k_val - 1
                    };
                    *km = if next_k == 0 { None } else { Some(next_k) };
                }
                intents.push_set_edit_params_draft(next);
            }
            let sweep_val = k_sweep_max.unwrap_or(6);
            let sweep_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.cluster.k_sweep_max",
                    label: "Silhouette/Elbow Sweep Max K",
                    value_text: &sweep_val.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if sweep_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max: ks, .. }) = &mut next.kind {
                    let next_sweep = if sweep_step > 0 { sweep_val.saturating_add(1).clamp(2, 12) } else { sweep_val.saturating_sub(1).max(2) };
                    *ks = Some(next_sweep);
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, .. } => {
            render_section_title(ui, tokens, "Chained Equations (MICE) Settings");
            let step_m = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.mi.m",
                    label: "Imputed Datasets (m)",
                    value_text: &imputations_m.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_m != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m: m, .. }) = &mut next.kind {
                    *m = (*m as isize + step_m as isize).clamp(2, 50) as usize;
                }
                intents.push_set_edit_params_draft(next);
            }
            let step_it = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.mi.iters",
                    label: "Gibbs Sweep Iterations",
                    value_text: &max_iters.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_it != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters: it, .. }) = &mut next.kind {
                    *it = (*it as isize + step_it as isize * 5).clamp(5, 200) as usize;
                }
                intents.push_set_edit_params_draft(next);
            }
            ui.label(egui::RichText::new(format!("Deterministic PRNG seed: {seed}")).small().color(tokens.text_muted));
        }
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_stat_rm_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    match &spec.kind {
        AnalysisKind::FactorialAnova(f) => {
            render_section_title(ui, tokens, "Repeated-Measures Matching Axis");
            for (wid, lbl, rm) in [
                ("edit.fac.rm.none", "No matching (independent cells)", RepeatedMeasuresDesign::None),
                ("edit.fac.rm.row", "Matched along Row Factor", RepeatedMeasuresDesign::RowFactorMatched),
                ("edit.fac.rm.col", "Matched along Column Factor", RepeatedMeasuresDesign::ColumnFactorMatched),
                ("edit.fac.rm.both", "Repeated measures in both factors", RepeatedMeasuresDesign::BothFactorsMatched),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, f.repeated_measures == rm, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                        fa.repeated_measures = rm;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.rm.mixed_effects_reml",
                "Fit Mixed-Effects REML (handles missing cells)",
                f.mixed_effects_reml || reml_active,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.mixed_effects_reml = !f.mixed_effects_reml;
                }
                intents.push_set_edit_params_draft(next);
            }
            render_df_method_picker(ui, spec, f.df_method, tokens, meta, intents);
            render_cov_structure_picker(ui, spec, f.residual_covariance, tokens, meta, intents);
        }
        AnalysisKind::TwoGroupComparison(_) => {
            let paired_en = r1_paired_or_matched_allowed(app.document(), Some(&spec.source_sheet_id));
            render_section_title(ui, tokens, "Within-Subject Matching");
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.paired_or_matched",
                "Each row is a matched subject pair",
                ep.paired_or_matched,
                &paired_en,
            ) {
                intents.push_dialog_widget_click("edit.paired_or_matched");
            }
        }
        _ => {}
    }
}

fn render_stat_multiplicity_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let cur = match &spec.kind {
        AnalysisKind::FactorialAnova(f) => f.multiplicity.method,
        AnalysisKind::NestedComparison(n) => n.multiplicity.method,
        AnalysisKind::MultipleRowTTests(m) => m.multiplicity.method,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method,
        _ => MultiplicityMethod::None,
    };
    render_section_title(ui, tokens, "Multiplicity Adjustment / Post-Hoc Method");
    for (slug, lbl, m) in [
        ("tukey", "Tukey HSD (all-pairs)", MultiplicityMethod::TukeyHsd),
        ("bonferroni", "Bonferroni (FWER)", MultiplicityMethod::Bonferroni),
        ("sidak", "Sidak (FWER)", MultiplicityMethod::Sidak),
        ("holm_sidak", "Holm-Sidak (step-down FWER)", MultiplicityMethod::HolmSidak),
        ("dunnett", "Dunnett (compare vs. control)", MultiplicityMethod::Dunnett { control_column: ep.control_column }),
        ("bh", "Benjamini-Hochberg (FDR)", MultiplicityMethod::BenjaminiHochbergFdr),
        ("bky", "Two-Stage BKY (adaptive FDR)", MultiplicityMethod::TwoStageBkyFdr),
        ("none", "None (uncorrected)", MultiplicityMethod::None),
    ] {
        let en = r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, m);
        if render_modal_radio(ui, tokens, meta, &format!("edit.post_hoc.{slug}"), lbl, cur == m, &en) {
            let mut next = spec.clone();
            match &mut next.kind {
                AnalysisKind::FactorialAnova(f) => f.multiplicity.method = m,
                AnalysisKind::NestedComparison(n) => n.multiplicity.method = m,
                AnalysisKind::MultipleRowTTests(mr) => mr.multiplicity.method = m,
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = m,
                _ => {}
            }
            intents.push_set_edit_params_draft(next);
        }
    }
}

fn render_stat_options_tab(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Confidence Level & Alpha");
    let conf_pct = spec.confidence_level.percent();
    ui.horizontal_wrapped(|ui| {
        for (wid, pct, lbl) in [("edit.conf.90", 90.0_f64, "90% CI"), ("edit.conf.95", 95.0, "95% CI"), ("edit.conf.99", 99.0, "99% CI")] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, (conf_pct - pct).abs() < 0.05, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("set_confidence:{pct:.1}"));
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, "P-Value Display Precision");
    ui.horizontal_wrapped(|ui| {
        for (wid, slug, lbl, active) in [
            ("edit.pval.four_decimal", "four_decimal", "0.0001", matches!(spec.p_value_format, PValueFormat::FourDecimal)),
            ("edit.pval.apa", "apa", "APA", matches!(spec.p_value_format, PValueFormat::Apa)),
            ("edit.pval.nejm", "nejm", "NEJM", matches!(spec.p_value_format, PValueFormat::Nejm)),
            ("edit.pval.custom", "custom", "Custom", matches!(spec.p_value_format, PValueFormat::Custom { .. })),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("set_pvalue_format:{slug}"));
            }
        }
    });

    let digits = match spec.p_value_format {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => decimals,
    };
    let r9_en = r9_pvalue_precision_spinner(&spec.p_value_format);
    let step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec { id_prefix: "edit.pvalue.digits", label: "Custom P-Value Decimal Digits", value_text: &digits.to_string(), enablement: &r9_en },
    );
    if step != 0 {
        intents.push_custom_view_action("stat_params", format!("spinner:edit.pvalue.digits:{step}"));
    }

    if let AnalysisKind::OneWayAnova(_) = &spec.kind {
        let sph = r1b_sphericity_correction(false, false);
        let _ = render_modal_checkbox(ui, tokens, meta, "edit.sphericity.geisser_greenhouse", "Geisser-Greenhouse", true, &sph);
    }
}

fn render_stat_diagnostics_tab(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Diagnostic Plots & Residual Checks");
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            let diag_en = r4_two_group_diagnostics(t.method);
            let _ = render_modal_checkbox(ui, tokens, meta, "edit.diag.qq", "Quantile-Quantile (QQ) residual plot", diag_en.enabled, &diag_en);
            let _ = render_modal_checkbox(ui, tokens, meta, "edit.diag.shapiro", "Shapiro-Wilk normality check", diag_en.enabled, &diag_en);
        }
        AnalysisKind::LinearRegression(lin) => {
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.lin.conf_bands",
                "Plot 95% confidence bands",
                lin.confidence_bands,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.confidence_bands = !lin.confidence_bands;
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.lin.pred_bands",
                "Plot prediction bands",
                lin.prediction_bands,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.prediction_bands = !lin.prediction_bands;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        _ => {
            ui.label(
                egui::RichText::new("Residual QQ plots, assumption checklist, and diagnostic exports are enabled automatically.")
                    .small()
                    .color(tokens.text_muted),
            );
        }
    }
}

fn render_df_method_picker(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    current: MixedModelDfMethod,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.add_space(4.0);
    render_section_title(ui, tokens, "REML Denominator Degrees of Freedom");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, m) in [
            ("edit.rm.df.kr", "Kenward-Roger", MixedModelDfMethod::KenwardRoger),
            ("edit.rm.df.sat", "Satterthwaite", MixedModelDfMethod::Satterthwaite),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, current == m, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                match &mut next.kind {
                    AnalysisKind::FactorialAnova(f) => f.df_method = m,
                    AnalysisKind::NestedComparison(n) => n.df_method = m,
                    _ => {}
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
}

fn render_cov_structure_picker(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    current: ResidualCovariance,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.add_space(4.0);
    render_section_title(ui, tokens, "REML Residual Covariance Structure");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, cov) in [
            ("edit.reml.cs", "Compound Symmetry", ResidualCovariance::CompoundSymmetry),
            ("edit.reml.un", "Unstructured", ResidualCovariance::Unstructured),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, current == cov, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(f) = &mut next.kind {
                    f.residual_covariance = cov;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
}

fn render_dataset_checklist(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
    if titles.is_empty() {
        return;
    }
    render_section_title(ui, tokens, "Included Data Sets / Columns");
    let sel = &spec.selected_data_sets;
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.dataset.all", "All Data Sets / Columns", sel.is_empty(), &ControlEnablement::enabled()) {
            intents.push_custom_view_action("stat_params", "select_all_datasets");
        }
        for (idx, col_title) in titles.iter().enumerate() {
            let on = sel.is_empty() || sel.contains(&idx);
            if render_modal_checkbox(ui, tokens, meta, &format!("edit.dataset.{idx}"), col_title, on, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("toggle_dataset:{idx}"));
            }
        }
    });
}

// ============================================================================
// 3. R9 Simulation/Sampling/Randomize & ResultViewer Modals
// ============================================================================

/// Render `dialogs_r9` generation modals (`RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`).
pub fn render_dialogs_r9_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let _ = &state.active_tab;
    let locale = normalize_locale(&app.locale);
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::RandomizeGroups => render_randomize_groups_modal(ctx, app, locale, tokens, meta, intents),
        ActiveDialog::RandomSampleRows { .. } => render_random_sample_modal(ctx, app, locale, tokens, meta, intents),
        ActiveDialog::SimulateDistributions => render_simulate_dist_modal(ctx, app, locale, tokens, meta, intents),
        _ => StatsModalFrameReport::default(),
    }
}

fn render_randomize_groups_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let rg = &app.canvas_dialogs.randomize_groups;
    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.randomize_groups",
            title: tr(locale, "w6.view.modal.randomize_groups"),
            subtitle_or_banner: Some("Generate balanced or permuted-block treatment group assignments."),
            banner_widget_id: "rand_groups.banner",
            max_width_pt: 580.0,
            max_height_pt: 460.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, _w, _h, meta, intents| {
            render_section_title(ui, tokens, "Treatment Arms Presets");
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, payload) in [
                    ("rand_groups.p2", "2 Arms (Ctrl / Tx)", "rand_groups:arms:two"),
                    ("rand_groups.p3", "3 Arms (Ctrl / Low / High)", "rand_groups:arms:three"),
                    ("rand_groups.pw", "Weighted 1:2", "rand_groups:arms:weighted"),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, false, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("dialogs_graph_misc", payload);
                    }
                }
            });
            let step_n = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "rand_groups.n",
                    label: "Total Subjects (N)",
                    value_text: &rg.subject_count.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_n != 0 {
                intents.push_custom_view_action("dialogs_graph_misc", if step_n > 0 { "rand_groups:n_inc" } else { "rand_groups:n_dec" });
            }
            ui.add_space(4.0);
            render_section_title(ui, tokens, "Randomization Scheme");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "rand_groups.sch.balanced",
                "Complete balanced allocation",
                rg.scheme == RandomizationSchemeKind::CompleteBalanced,
                &ControlEnablement::enabled(),
            ) {
                intents.push_custom_view_action("dialogs_graph_misc", "rand_groups:scheme:balanced");
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "rand_groups.sch.block",
                "Permuted block randomization",
                rg.scheme == RandomizationSchemeKind::PermutedBlock,
                &ControlEnablement::enabled(),
            ) {
                intents.push_custom_view_action("dialogs_graph_misc", "rand_groups:scheme:block");
            }
            if rg.scheme == RandomizationSchemeKind::PermutedBlock {
                let blk = rg.block_sizes.first().copied().unwrap_or(4);
                let step_b = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_groups.block",
                        label: "Block Size",
                        value_text: &blk.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_b != 0 {
                    intents.push_custom_view_action("dialogs_graph_misc", if step_b > 0 { "rand_groups:block_inc" } else { "rand_groups:block_dec" });
                }
            }
            let step_seed = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "rand_groups.seed",
                    label: "PRNG Seed",
                    value_text: &rg.seed.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_seed != 0 {
                intents.push_custom_view_action("dialogs_graph_misc", if step_seed > 0 { "rand_groups:seed_inc" } else { "rand_groups:seed_dec" });
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    StatsModalFrameReport::from(capture)
}

fn render_random_sample_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let rs = &app.canvas_dialogs.random_sample_rows;
    let doc = app.document();
    let prop_en = if rs.stratify_column.is_none() {
        ControlEnablement::disabled("Proportional allocation requires selecting a stratification column.")
    } else {
        crate::modal_predicates::evaluate_dialog_widget(&app.canvas_dialogs, doc, "sample.proportional_allocation")
    };

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.random_sample_rows",
            title: tr(locale, "cmd.table.randomSampleRows"),
            subtitle_or_banner: Some("Draw a reproducible simple or stratified random sample of rows."),
            banner_widget_id: "rand_sample.banner",
            max_width_pt: 660.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, avail_w, _h, meta, intents| {
            let titles = column_titles_for_draft(doc, rs.source_sheet_id.as_ref());
            let render_left = |ui: &mut Ui, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents| {
                render_section_title(ui, tokens, "Source DataSheet");
                for (idx, ds) in doc.data_sheets.iter().enumerate() {
                    let selected = rs.source_sheet_id.as_ref() == Some(&ds.id);
                    if render_modal_radio(ui, tokens, meta, &format!("rand_sample.src.{idx}"), &ds.title, selected, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("dialogs_graph_misc", format!("rand_sample:src:{idx}"));
                    }
                }
                ui.add_space(4.0);
                let step_n = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_sample.n",
                        label: "Sample Size (rows)",
                        value_text: &rs.sample_size.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_n != 0 {
                    intents.push_custom_view_action("dialogs_graph_misc", if step_n > 0 { "rand_sample:n_inc" } else { "rand_sample:n_dec" });
                }
                let step_s = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        id_prefix: "rand_sample.seed",
                        label: "PRNG Seed",
                        value_text: &rs.seed.to_string(),
                        enablement: &ControlEnablement::enabled(),
                    },
                );
                if step_s != 0 {
                    intents.push_custom_view_action("dialogs_graph_misc", if step_s > 0 { "rand_sample:seed_inc" } else { "rand_sample:seed_dec" });
                }
            };
            let render_right = |ui: &mut Ui, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents| {
                render_section_title(ui, tokens, "Stratification");
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    "sample.proportional_allocation",
                    "Proportional allocation across strata",
                    rs.proportional_allocation,
                    &prop_en,
                ) {
                    intents.push_custom_view_action("dialogs_graph_misc", "rand_sample:proportional");
                }
                if render_modal_radio(
                    ui,
                    tokens,
                    meta,
                    "rand_sample.strat.none",
                    "Unstratified simple random sample",
                    rs.stratify_column.is_none(),
                    &ControlEnablement::enabled(),
                ) {
                    intents.push_custom_view_action("dialogs_graph_misc", "rand_sample:stratify:none");
                }
                for (ci, t) in titles.iter().enumerate() {
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("rand_sample.strat.{ci}"),
                        &format!("Stratify by: {t}"),
                        rs.stratify_column == Some(ci),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_custom_view_action("dialogs_graph_misc", format!("rand_sample:stratify:{ci}"));
                    }
                }
            };
            if avail_w >= 480.0 {
                let left_w = (avail_w * 0.48).max(200.0);
                let right_w = (avail_w - left_w - 10.0).max(200.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_w);
                        render_left(ui, meta, intents);
                    });
                    ui.add_space(10.0);
                    ui.allocate_ui_with_layout(Vec2::new(right_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(right_w);
                        render_right(ui, meta, intents);
                    });
                });
            } else {
                render_left(ui, meta, intents);
                ui.add_space(4.0);
                render_right(ui, meta, intents);
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    StatsModalFrameReport::from(capture)
}

fn render_simulate_dist_modal(
    ctx: &Context,
    app: &UiApp,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let sd = &app.canvas_dialogs.simulate_distributions;
    let col_idx = app.view_shards.dialogs_graph_misc.sim_selected_col_idx.min(sd.columns.len().saturating_sub(1));

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.simulate_distributions",
            title: tr(locale, "cmd.table.simulateDistributions"),
            subtitle_or_banner: Some("Simulate synthetic probability columns for power/Monte Carlo benchmarks."),
            banner_widget_id: "sim_dist.banner",
            max_width_pt: 600.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _, _w, _h, meta, intents| {
            let step_rows = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "sim_dist.rows",
                    label: "Number of Rows",
                    value_text: &sd.n_rows.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_rows != 0 {
                intents.push_custom_view_action("dialogs_graph_misc", if step_rows > 0 { "sim_dist:rows_inc" } else { "sim_dist:rows_dec" });
            }
            let step_seed = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "sim_dist.seed",
                    label: "PRNG Seed",
                    value_text: &sd.seed.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_seed != 0 {
                intents.push_custom_view_action("dialogs_graph_misc", if step_seed > 0 { "sim_dist:seed_inc" } else { "sim_dist:seed_dec" });
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "simulate.use_multiple_variables",
                "Create MultipleVariables table",
                sd.use_multiple_variables,
                &ControlEnablement::enabled(),
            ) {
                intents.push_custom_view_action("dialogs_graph_misc", "sim_dist:multi_var");
            }
            ui.add_space(4.0);
            render_section_title(ui, tokens, "Simulated Columns");
            ui.horizontal_wrapped(|ui| {
                for (i, c) in sd.columns.iter().enumerate() {
                    if render_modal_seg_btn(ui, tokens, meta, &format!("sim_dist.col.{i}"), &c.name, i == col_idx, &ControlEnablement::enabled()) {
                        intents.push_custom_view_action("dialogs_graph_misc", format!("sim_dist:col:{i}"));
                    }
                }
                if render_modal_seg_btn(ui, tokens, meta, "sim_dist.add_col", "+ Add Column", false, &ControlEnablement::enabled()) {
                    intents.push_custom_view_action("dialogs_graph_misc", "sim_dist:add_col");
                }
            });
            if let Some(col) = sd.columns.get(col_idx) {
                ui.add_space(4.0);
                render_section_title(ui, tokens, "Distribution Family");
                ui.horizontal_wrapped(|ui| {
                    for (wid, slug, lbl, kind) in [
                        ("sim_dist.d.uni", "uniform", "Uniform", SimDistKind::Uniform),
                        ("sim_dist.d.gauss", "gaussian", "Gaussian", SimDistKind::Gaussian),
                        ("sim_dist.d.logn", "lognormal", "LogNormal", SimDistKind::LogNormal),
                        ("sim_dist.d.bin", "binomial", "Binomial", SimDistKind::Binomial),
                        ("sim_dist.d.pois", "poisson", "Poisson", SimDistKind::Poisson),
                        ("sim_dist.d.exp", "exponential", "Exponential", SimDistKind::Exponential),
                    ] {
                        if render_modal_seg_btn(ui, tokens, meta, wid, lbl, col.distribution == kind, &ControlEnablement::enabled()) {
                            intents.push_custom_view_action("dialogs_graph_misc", format!("sim_dist:dist:{slug}"));
                        }
                    }
                });
                let (p1_lbl, p1_val, p2_opt) = match col.distribution {
                    SimDistKind::Uniform => ("Minimum", format!("{:.2}", col.min), Some(("Maximum", format!("{:.2}", col.max)))),
                    SimDistKind::Gaussian => ("Mean (μ)", format!("{:.2}", col.mean), Some(("SD (σ)", format!("{:.2}", col.sd)))),
                    SimDistKind::LogNormal => ("Log Mean", format!("{:.2}", col.mean_log), Some(("Log SD", format!("{:.2}", col.sd_log)))),
                    SimDistKind::Binomial => ("Trials (n)", col.trials.to_string(), Some(("Prob (p)", format!("{:.2}", col.p)))),
                    SimDistKind::Poisson => ("Lambda (λ)", format!("{:.2}", col.lambda), None),
                    SimDistKind::Exponential => ("Rate (λ)", format!("{:.2}", col.rate), None),
                };
                let s1 = render_stepper_row(
                    ui,
                    tokens,
                    meta,
                    StepperSpec { id_prefix: "sim_dist.p1", label: p1_lbl, value_text: &p1_val, enablement: &ControlEnablement::enabled() },
                );
                if s1 != 0 {
                    intents.push_custom_view_action("dialogs_graph_misc", if s1 > 0 { "sim_dist:param:p1_inc" } else { "sim_dist:param:p1_dec" });
                }
                if let Some((p2_lbl, p2_val)) = p2_opt {
                    let s2 = render_stepper_row(
                        ui,
                        tokens,
                        meta,
                        StepperSpec { id_prefix: "sim_dist.p2", label: p2_lbl, value_text: &p2_val, enablement: &ControlEnablement::enabled() },
                    );
                    if s2 != 0 {
                        intents.push_custom_view_action("dialogs_graph_misc", if s2 > 0 { "sim_dist:param:p2_inc" } else { "sim_dist:param:p2_dec" });
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    StatsModalFrameReport::from(capture)
}

/// Render `ActiveDialog::ResultViewer` when explicitly requested.
pub fn render_result_viewer_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ResultViewer { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let doc = app.document();
    let rs = doc.result_sheet(result_sheet_id);
    let title = rs.map_or_else(|| tr(tokens.locale, "w6.view.modal.result_viewer").to_owned(), |r| r.title().to_owned());
    let tab_idx = app.canvas_dialogs.result_view.active_tab_index;
    let locale = normalize_locale(&app.locale);

    let mut dummy = ();
    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        &mut dummy,
        Wave4ModalShell {
            widget_id: "dialog.result_viewer",
            title: &title,
            subtitle_or_banner: rs.map(|r| r.analysis.effective_title()),
            banner_widget_id: "result_viewer.banner",
            max_width_pt: 780.0,
            max_height_pt: 560.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |ui, _, meta, intents| {
            if let Some(res) = rs
                && !res.tabs.is_empty()
            {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (idx, tab) in res.tabs.iter().enumerate() {
                        if render_modal_seg_btn(
                            ui,
                            tokens,
                            meta,
                            &format!("result.tab.{idx}"),
                            &tab.title,
                            idx == tab_idx.min(res.tabs.len() - 1),
                            &ControlEnablement::enabled(),
                        ) {
                            intents.push_select_result_tab(idx);
                        }
                    }
                });
            }
        },
        |ui, _, _w, _h, _meta, _intents| {
            if let Some(res) = rs
                && let Some(tab) = res.tabs.get(tab_idx).or_else(|| res.tabs.first())
            {
                egui::Grid::new("result_viewer_table").striped(true).num_columns(tab.column_headers.len().max(1)).show(ui, |ui| {
                    for hdr in &tab.column_headers {
                        ui.label(egui::RichText::new(hdr).strong().small().color(tokens.text_strong));
                    }
                    ui.end_row();
                    for row in tab.rows.iter().take(120) {
                        for cell in row {
                            let s = ResultSheetViewState::format_result_cell(cell, res.analysis.p_value_format);
                            ui.label(egui::RichText::new(s).small());
                        }
                        ui.end_row();
                    }
                });
            } else {
                ui.label("No tabular results available.");
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push_dialog_button("ok");
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );
    StatsModalFrameReport::from(capture)
}

fn df_method_label(m: MixedModelDfMethod) -> &'static str {
    match m {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger df",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite df",
    }
}

fn multiplicity_short_label(m: MultiplicityMethod) -> &'static str {
    match m {
        MultiplicityMethod::None => "None",
        MultiplicityMethod::TukeyHsd => "Tukey HSD",
        MultiplicityMethod::Dunnett { .. } => "Dunnett",
        MultiplicityMethod::Sidak => "Sidak",
        MultiplicityMethod::Bonferroni => "Bonferroni",
        MultiplicityMethod::HolmSidak => "Holm-Sidak",
        MultiplicityMethod::DunnettT3 => "Dunnett T3",
        MultiplicityMethod::GamesHowell => "Games-Howell",
        MultiplicityMethod::TamhaneT2 => "Tamhane T2",
        MultiplicityMethod::DunnRankSum => "Dunn",
        MultiplicityMethod::LinearTrend => "Linear Trend",
        MultiplicityMethod::BenjaminiHochbergFdr => "BH FDR",
        MultiplicityMethod::BenjaminiYekutieliFdr => "BY FDR",
        MultiplicityMethod::TwoStageBkyFdr => "BKY FDR",
        MultiplicityMethod::NewmanKeuls => "Newman-Keuls",
        MultiplicityMethod::FisherLsd => "Fisher LSD",
    }
}

fn contingency_method_label(m: ContingencyMethod) -> &'static str {
    match m {
        ContingencyMethod::PearsonChiSquare { yates_continuity: true } => "Chi-square + Yates",
        ContingencyMethod::PearsonChiSquare { yates_continuity: false } => "Pearson Chi-square",
        ContingencyMethod::LikelihoodRatioG2 => "Likelihood Ratio G^2",
        ContingencyMethod::FisherExact => "Fisher's Exact",
        ContingencyMethod::BarnardExact => "Barnard's Exact",
        ContingencyMethod::CochranArmitageTrend => "Cochran-Armitage Trend",
        ContingencyMethod::McNemarPaired => "McNemar Paired",
        ContingencyMethod::CohenKappa { .. } => "Cohen's Kappa",
        ContingencyMethod::FleissKappa => "Fleiss' Kappa",
        ContingencyMethod::ExactBinomial => "Exact Binomial",
    }
}

fn method_reference_text(spec: &curvebook_doc::AnalysisSpec) -> &'static str {
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(_) => {
            "Student (1908), Welch-Satterthwaite (1947), Mann-Whitney (1947), Wilcoxon (1945) & Hodges-Lehmann (1963) shift CIs."
        }
        AnalysisKind::FactorialAnova(_) => {
            "Type III orthogonal contrasts (Maxwell, Delaney & Kelley 2017) with Kenward-Roger (1997) / Satterthwaite REML mixed-effects."
        }
        AnalysisKind::GeneralizedLinearModel(_) => {
            "McCullagh & Nelder (1989) IRLS maximum likelihood with canonical identity/logit/log links and VIF collinearity checks."
        }
        AnalysisKind::Survival(_) => {
            "Kaplan & Meier (1958), Mantel-Cox log-rank, Greenwood CIs (Kalbfleisch & Prentice 2002), and Cox (1972) PH with Efron/Breslow/Exact ties."
        }
        AnalysisKind::Multivariate(_) => {
            "Jolliffe (2002) SVD PCA, Horn (1965) parallel analysis, Ward/UPGMA hierarchical + k-means++ clustering, and Rubin (1987) MICE pooling."
        }
        _ => "Exact peer-reviewed CurveBook statistical reference equations and confidence limits evaluated at the active alpha level.",
    }
}
