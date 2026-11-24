//! Shared modal chrome, action button, live formula bar, TeX equation preview, role badge, and picker helpers.

use curvebook_doc::{
    AnalysisKind, ContingencyMethod, DataSheet, MixedModelDfMethod, MultiplicityMethod, ProjectDocument, ResidualCovariance, SheetId, VariableRole,
};
use curvebook_text::{EquationSegment, SpanScript, SpanSlope, SpanWeight, parse_equation_segments};
use egui::{Color32, Context, CornerRadius, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, scroll_area::ScrollBarVisibility};
use serde_json::json;

use crate::i18n::tr;
use crate::inspect::ZLayer;
use crate::modal_predicates::ControlEnablement;

use crate::egui_shell::inspect_bridge::{ModalCaptureReport, ModalCaptureSpec, finalize_egui_modal_response};
use crate::egui_shell::stat_anova_modal::{
    ClippedTextSpec, column_titles_for_draft, record_widget, render_clipped_text, render_modal_checkbox, render_modal_seg_btn, render_section_title,
};
use crate::egui_shell::widgets::paint_icon_button_named;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

/// Shared modal chrome configuration for Wave 4 native `egui` dialogs (`R15.4`).
pub(in crate::egui_shell) struct Wave4ModalShell<'a> {
    pub(in crate::egui_shell) widget_id: &'a str,
    pub(in crate::egui_shell) title: &'a str,
    pub(in crate::egui_shell) subtitle_or_banner: Option<&'a str>,
    pub(in crate::egui_shell) banner_widget_id: &'a str,
    pub(in crate::egui_shell) max_width_pt: f32,
    pub(in crate::egui_shell) max_height_pt: f32,
    pub(in crate::egui_shell) use_body_scroll: bool,
    pub(in crate::egui_shell) footer_reserve_pt: f32,
}

/// Render a centered, responsive Wave 4 `egui::Modal` shell with pinned header/footer,
/// `AlwaysVisible` vertical scrollbar gutter (zero `Role::ScrollBar` overlap),
/// `ZLayer::ModalBackdrop` capture, and `FrameInspectMeta` registration.
#[allow(clippy::too_many_arguments)]
pub(in crate::egui_shell) fn show_wave4_modal<S, H, B, F>(
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
        .stroke(Stroke::new(1.5, tokens.accent))
        .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
        .inner_margin(egui::Margin::same(12));

    let modal_resp =
        egui::Modal::new(Id::unique(("wave4_modal", cfg.widget_id))).backdrop_color(tokens.scrim).area(area).frame(frame).show(ctx, |ui| {
            ui.set_min_size(Vec2::new(inner_w, inner_h));
            ui.set_max_size(Vec2::new(inner_w, inner_h));
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

            // Header strip
            ui.horizontal(|ui| {
                let close_side = tokens.metrics.min_hit_target_pt;
                let title_w = (inner_w - close_side - tokens.metrics.space_md).max(80.0);
                ui.allocate_ui(Vec2::new(title_w, close_side), |ui| {
                    render_clipped_text(
                        ui,
                        meta,
                        ClippedTextSpec {
                            widget_id: "modal.title",
                            role: "label",
                            text: cfg.title,
                            font_size: tokens.metrics.font_ui_title_pt,
                            color: tokens.text_strong,
                            z_layer: ZLayer::ModalDialog,
                            tooltip: None,
                            record_meta: true,
                        },
                    );
                });
                let (_, close_alloc) = ui.allocate_exact_size(Vec2::splat(close_side), Sense::click());
                let close_lbl = tr(tokens.locale, "btn.cancel");
                let close_resp = paint_icon_button_named(ui, close_alloc, "close", close_lbl, close_lbl, false);
                record_widget(ui, meta, &close_resp, "modal.close", "button", close_lbl, None, None, true, None, ZLayer::ModalDialog, false, None);
                if close_resp.clicked() {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
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
pub(in crate::egui_shell) fn render_modal_action_button(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    primary: bool,
    selected: bool,
    enablement: &ControlEnablement,
) -> bool {
    let min_w: f32 = if primary { 92.0 } else { 66.0 };
    let max_w = (ui.available_width() - 4.0).max(48.0);
    let (_, clicked) = crate::egui_shell::ui_kit::styled_button(
        ui,
        tokens,
        meta,
        crate::egui_shell::ui_kit::StyledButtonSpec {
            widget_id,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: false,
            label,
            tooltip: None,
            command_id: None,
            width: crate::egui_shell::ui_kit::ButtonWidth::FitText { min_w, max_w, pad_x: 18.0 },
            height: 25.0,
            font_size: 11.5,
            radius: tokens.metrics.radius_md,
            text_align: crate::egui_shell::ui_kit::ButtonTextAlign::Center,
            break_anywhere: false,
            primary,
            selected,
            enabled: enablement.enabled,
            accent_text_when_selected: false,
            border_override: None,
            value: None,
            disabled_reason: enablement.disabled_reason.as_deref(),
            z_layer: ZLayer::ModalDialog,
            skip_offscreen: true,
            clip_interact_to_viewport: false,
            tooltip_mode: crate::egui_shell::ui_kit::TooltipMode::Standard,
        },
    );
    clicked
}

/// Draw a highlighted live symbolic formula bar (`widget_id`: e.g. `"edit.formula_bar"`).
pub(in crate::egui_shell) fn render_formula_bar(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    title: &str,
    formula: &str,
) {
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
                    color: tokens.accent,
                    z_layer: ZLayer::ModalDialog,
                    tooltip: Some(formula),
                    record_meta: true,
                },
            );
        });
    });
}

/// Render TeX math parsed by [`curvebook_text::parse_equation_segments`] into a compact visual card.
pub(in crate::egui_shell) fn render_tex_equation_preview(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    tex: &str,
    font_size: f32,
) {
    let segments = parse_equation_segments(tex);
    let avail_w = ui.available_width().max(120.0);
    let (eq_w, eq_h) = measure_equation_segments(ui, &segments, font_size);
    let card_h = (eq_h + 16.0).clamp(36.0, 92.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail_w, card_h), Sense::hover());
    let r_md = CornerRadius::same(tokens.metrics.radius_md);
    ui.painter().rect_filled(rect, r_md, tokens.panel_bg);
    ui.painter().rect_stroke(rect, r_md, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

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
pub(in crate::egui_shell) fn render_mv_role_badge(ui: &mut Ui, tokens: &Tokens, role: VariableRole, inferred: bool) {
    let (tag, col) = match role {
        VariableRole::Continuous => ("NUM", tokens.role_num),
        VariableRole::Categorical => ("CAT", tokens.role_cat),
        VariableRole::Label => ("LBL", tokens.role_lbl),
        VariableRole::Calculated => ("FX=", tokens.role_fx),
    };
    let text = if inferred { format!("{tag}*") } else { String::from(tag) };
    egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, col))
        .corner_radius(CornerRadius::same(tokens.metrics.radius_sm))
        .inner_margin(Vec2::new(4.0, 1.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(tokens.metrics.font_ui_small_pt).strong().color(col));
        });
}

pub(super) fn render_df_method_picker(
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
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
}

pub(super) fn render_cov_structure_picker(
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
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
}

pub(super) fn render_dataset_checklist(
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
            intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::SelectAllDatasets));
        }
        for (idx, col_title) in titles.iter().enumerate() {
            let on = sel.is_empty() || sel.contains(&idx);
            if render_modal_checkbox(ui, tokens, meta, &format!("edit.dataset.{idx}"), col_title, on, &ControlEnablement::enabled()) {
                intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::ToggleDataset { idx }));
            }
        }
    });
}

/// Resolve the source [`DataSheet`] for an analysis, falling back to `ResultSheet::analyzable_data`
/// when an analysis chains onto an upstream `Transform` / `Normalize` / `PruneRows` result sheet.
pub(super) fn resolve_source_data_sheet<'a>(doc: &'a ProjectDocument, id: &SheetId) -> Option<&'a DataSheet> {
    doc.data_sheet(id).or_else(|| doc.result_sheet(id).and_then(|rs| rs.analyzable_data.as_ref()))
}

pub(super) fn df_method_label(m: MixedModelDfMethod) -> &'static str {
    match m {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger df",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite df",
    }
}

pub(super) fn multiplicity_short_label(m: MultiplicityMethod) -> &'static str {
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

pub(super) fn contingency_method_label(m: ContingencyMethod) -> &'static str {
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

pub(super) fn method_reference_text(spec: &curvebook_doc::AnalysisSpec) -> &'static str {
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
