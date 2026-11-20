//! Native `egui` multi-tab reactive `ResultSheet` viewer (`docs/planning/gui-layout-spec.md` §8.10, `R15.3`).

use std::collections::BTreeSet;

use curvebook_doc::{AnalysisKind, CiBound, PValueFormat, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId};
use egui::{CornerRadius, FontId, Key, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ResultSheetViewState;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use crate::i18n::tr_or_fallback;
use crate::inspect::ZLayer;
use crate::tr;

/// Semantic classification of a [`ResultTab`] per `gui-layout-spec.md` §8.10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResultTabCategory {
    InferentialReport,
    PlotReadyMatrix,
}

impl ResultTabCategory {
    /// Classify `kind` into an inferential narrative/summary report vs a plot-ready derived matrix.
    #[must_use]
    pub const fn for_kind(kind: ResultTabKind) -> Self {
        match kind {
            ResultTabKind::Residuals
            | ResultTabKind::CurveCoordinates
            | ResultTabKind::InterpolatedUnknowns
            | ResultTabKind::NumberAtRisk
            | ResultTabKind::CovarianceMatrix
            | ResultTabKind::NormalQq
            | ResultTabKind::BaselineSurvival
            | ResultTabKind::LogMinusLog => Self::PlotReadyMatrix,
            ResultTabKind::MainSummary
            | ResultTabKind::AnovaTable
            | ResultTabKind::MultipleComparisons
            | ResultTabKind::ParametersAndConfidenceIntervals
            | ResultTabKind::ModelComparison
            | ResultTabKind::DescriptiveSummary
            | ResultTabKind::Diagnostics => Self::InferentialReport,
        }
    }

    /// Concise badge label surfaced on the tab sub-header.
    #[must_use]
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::InferentialReport => "Inferential Report",
            Self::PlotReadyMatrix => "Plot-Ready Matrix",
        }
    }
}

/// Persistent `egui_shell` state for the multi-tab `ResultSheet` viewer (`R15.3`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsViewEguiState {
    pub scroll_offset_y: f32,
    pub hovered_row: Option<usize>,
    pub selected_cell: Option<(SheetId, usize, usize, usize)>,
    pub collapsed_sections: BTreeSet<String>,
    pub tab_strip_scroll_x: f32,
}

/// Per-frame region & active-tab report returned by [`render_results_view`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsViewFrameReport {
    pub header_rect_pt: Option<Rect>,
    pub body_rect_pt: Option<Rect>,
    pub scrollbar_rect_pt: Option<Rect>,
    pub active_tab_index: usize,
    pub active_tab_category: Option<ResultTabCategory>,
    pub has_ambiguity_callout: bool,
}

#[derive(Debug, Clone)]
struct FormattedCellDisplay {
    text: String,
    is_ambiguous: bool,
    is_significant: bool,
}

fn format_result_cell_semantic(cell: &ResultCell, p_format: PValueFormat, is_survival_summary: bool) -> FormattedCellDisplay {
    match cell {
        ResultCell::Empty => FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false },
        ResultCell::Number { value, prefix, units } => {
            if !value.is_finite() {
                // Unreached medians or undefined scalars must remain explicitly blank, never "NaN" or "0".
                return FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false };
            }
            let base = ResultSheetViewState::format_result_cell(cell, p_format);
            let is_amb = prefix.as_deref() == Some("~");
            let text = if is_amb && !base.contains("???") {
                let unit_sfx = units.as_deref().filter(|u| !u.is_empty()).map_or(String::new(), |u| format!(" {u}"));
                let _ = unit_sfx;
                base
            } else {
                base
            };
            FormattedCellDisplay { text, is_ambiguous: is_amb, is_significant: false }
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            let has_open = matches!(lower, CiBound::Open) || matches!(upper, CiBound::Open);
            if is_survival_summary {
                let lo = format_survival_ci_bound(*lower);
                let hi = format_survival_ci_bound(*upper);
                let text = if lo.is_empty() && hi.is_empty() { String::new() } else { format!("[{lo}, {hi}]") };
                FormattedCellDisplay { text, is_ambiguous: false, is_significant: false }
            } else if has_open {
                let lo = format_ci_bound_semantic(*lower);
                let hi = format_ci_bound_semantic(*upper);
                let text = if matches!(lower, CiBound::Open) && matches!(upper, CiBound::Open) {
                    String::from("??? (ambiguous)")
                } else {
                    format!("[{lo}, {hi}] (???)")
                };
                FormattedCellDisplay { text, is_ambiguous: true, is_significant: false }
            } else {
                FormattedCellDisplay { text: ResultSheetViewState::format_result_cell(cell, p_format), is_ambiguous: false, is_significant: false }
            }
        }
        ResultCell::PValue { stars, .. } => {
            let text = ResultSheetViewState::format_result_cell(cell, p_format);
            let is_sig = !stars.is_empty() && stars != "ns";
            FormattedCellDisplay { text, is_ambiguous: false, is_significant: is_sig }
        }
        ResultCell::Text(s) => {
            let lower = s.to_ascii_lowercase();
            let is_amb = s.contains("???") || lower.contains("ambiguous");
            let text = if is_amb && !s.contains("???") { format!("??? {s}") } else { s.clone() };
            FormattedCellDisplay { text, is_ambiguous: is_amb, is_significant: false }
        }
        ResultCell::Boolean(b) => {
            FormattedCellDisplay { text: String::from(if *b { "Yes" } else { "No" }), is_ambiguous: false, is_significant: false }
        }
    }
}

fn format_survival_ci_bound(bound: CiBound) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_owned(),
        CiBound::Finite(_) | CiBound::Open => String::from("—"),
    }
}

fn format_ci_bound_semantic(bound: CiBound) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_owned(),
        CiBound::Finite(_) | CiBound::Open => String::from("???"),
    }
}

/// Render the active [`curvebook_doc::ResultSheet`] with `32 pt` top summary bar, scrollable
/// sub-tab strip (`< >` + `Ctrl+PageUp/PageDown`), ambiguity callouts, and semantic matrix/report body.
pub fn render_results_view(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut ResultsViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ResultsViewFrameReport {
    let active_id = app.document().active_sheet_id.clone();
    render_results_view_for_sheet(ui, app, active_id.as_ref(), state, tokens, enabled, meta, intents)
}

/// Render a specific [`curvebook_doc::ResultSheet`] inside `ui`.
#[allow(clippy::too_many_arguments)]
pub fn render_results_view_for_sheet(
    ui: &mut egui::Ui,
    app: &UiApp,
    sheet_id: Option<&SheetId>,
    state: &mut ResultsViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ResultsViewFrameReport {
    let mut report = ResultsViewFrameReport::default();
    let doc = app.document();
    let Some(target_id) = sheet_id else {
        return report;
    };
    let Some(rs) = doc.result_sheet(target_id) else {
        return report;
    };

    ensure_solid_scrollbar_style(ui, tokens);

    let tab_count = rs.tabs.len();
    let mut active_idx = if tab_count == 0 { 0 } else { app.canvas_dialogs.result_view.active_tab_index.min(tab_count - 1) };

    // Ctrl+PageUp / Ctrl+PageDown keyboard switching across sub-tabs (§8.10)
    if enabled && tab_count > 1 && !ui.ctx().egui_wants_keyboard_input() {
        let prev_pressed = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::PageUp) || i.consume_key(Modifiers::COMMAND, Key::PageUp));
        let next_pressed = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::PageDown) || i.consume_key(Modifiers::COMMAND, Key::PageDown));
        if prev_pressed {
            active_idx = if active_idx == 0 { tab_count - 1 } else { active_idx - 1 };
            intents.push_select_result_tab(active_idx);
        } else if next_pressed {
            active_idx = (active_idx + 1) % tab_count;
            intents.push_select_result_tab(active_idx);
        }
    }
    report.active_tab_index = active_idx;

    let avail = ui.available_rect_before_wrap();
    if avail.width() < 40.0 || avail.height() < 40.0 {
        return report;
    }

    // 32 pt Top Summary Bar: [Edit Parameters (Cmd+T)] | [<] [Sub-tabs...] [>]
    let hdr_h = 32.0_f32.min(avail.height());
    let hdr_rect = Rect::from_min_size(avail.min, vec2(avail.width(), hdr_h));
    report.header_rect_pt = Some(hdr_rect);

    ui.painter().rect_filled(hdr_rect, CornerRadius::ZERO, tokens.panel_bg);
    ui.painter()
        .line_segment([pos2(hdr_rect.left(), hdr_rect.bottom()), pos2(hdr_rect.right(), hdr_rect.bottom())], Stroke::new(1.0, tokens.divider));

    ui.scope_builder(egui::UiBuilder::new().max_rect(hdr_rect.shrink2(vec2(6.0, 3.0))), |ui| {
        ui.set_clip_rect(hdr_rect);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let edit_lbl = if hdr_rect.width() >= 520.0 {
                tr_or_fallback(&app.locale, "results.parameters", "Edit Parameters (Cmd+T)")
            } else {
                tr(&app.locale, "toolbar.parameters")
            };
            let edit_btn = egui::Button::new(RichText::new(edit_lbl).color(tokens.text_strong)).fill(tokens.active_bg);
            let edit_resp = ui.add_enabled(enabled, edit_btn);
            meta.record_response(
                ui,
                &edit_resp,
                FrameInspectEntryInput {
                    widget_id: "result.edit_params",
                    role: "button",
                    label: edit_lbl,
                    command_id: Some("ui.editParams.open"),
                    value: None,
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if edit_resp.clicked() && enabled {
                intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
            }

            ui.separator();

            // < > stepper buttons
            let can_prev = enabled && tab_count > 1 && active_idx > 0;
            let prev_resp = ui.add_enabled(can_prev, egui::Button::new("<").min_size(Vec2::new(24.0, 24.0)));
            meta.record_response(
                ui,
                &prev_resp,
                FrameInspectEntryInput {
                    widget_id: "result.tabs.prev",
                    role: "button",
                    label: tr_or_fallback(&app.locale, "results.prev_tabs", "Previous Tab"),
                    command_id: None,
                    value: None,
                    enabled: can_prev,
                    disabled_reason: (!can_prev).then_some("At first result tab"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if prev_resp.clicked() && can_prev {
                let next_i = active_idx.saturating_sub(1);
                intents.push_select_result_tab(next_i);
                state.tab_strip_scroll_x = (state.tab_strip_scroll_x - 100.0).max(0.0);
            }

            let next_btn_w = 26.0;
            let strip_w = (ui.available_width() - next_btn_w - 6.0).max(0.0);
            let max_tab_w = (strip_w - 4.0).clamp(36.0, 136.0);
            let gap = 4.0_f32;
            let tab_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
            let tab_text_budget = (max_tab_w - 14.0).max(8.0);
            let mut tab_layouts = Vec::with_capacity(tab_count);
            let mut cur_x = 0.0_f32;
            for (idx, tab) in rs.tabs.iter().enumerate() {
                let mut job = egui::text::LayoutJob::single_section(
                    tab.title.clone(),
                    egui::text::TextFormat { font_id: tab_font.clone(), color: tokens.text_primary, ..Default::default() },
                );
                job.wrap =
                    egui::text::TextWrapping { max_width: tab_text_budget, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let g = ui.fonts_mut(|f| f.layout_job(job));
                let w = (g.size().x + 14.0).clamp(36.0_f32.min(max_tab_w), max_tab_w);
                tab_layouts.push((idx, tab, cur_x, w));
                cur_x += w + gap;
            }
            let total_tabs_w = (cur_x - gap).max(0.0);
            let max_scroll_x = (total_tabs_w - strip_w).max(0.0);
            state.tab_strip_scroll_x = state.tab_strip_scroll_x.clamp(0.0, max_scroll_x);
            if let Some((_, _, ax0, aw)) = tab_layouts.get(active_idx) {
                let ax1 = *ax0 + *aw;
                if *ax0 < state.tab_strip_scroll_x {
                    state.tab_strip_scroll_x = (*ax0).clamp(0.0, max_scroll_x);
                } else if ax1 > state.tab_strip_scroll_x + strip_w {
                    state.tab_strip_scroll_x = (ax1 - strip_w).clamp(0.0, max_scroll_x);
                }
            }

            let strip_rect = ui.allocate_space(vec2(strip_w, 24.0)).1;
            let strip_clip = strip_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if strip_w >= 28.0 && strip_clip.is_positive() {
                ui.scope_builder(egui::UiBuilder::new().max_rect(strip_rect), |ui| {
                    ui.set_clip_rect(strip_clip);
                    for (idx, tab, x0, w) in &tab_layouts {
                        let selected = *idx == active_idx;
                        let tab_rect =
                            Rect::from_min_size(pos2(strip_rect.left() - state.tab_strip_scroll_x + *x0, strip_rect.top() + 1.0), vec2(*w, 22.0));
                        let vis = tab_rect.intersect(strip_clip);
                        if !vis.is_positive() || vis.width() < 28.0 || vis.height() < 8.0 {
                            continue;
                        }
                        let resp = ui.interact(
                            vis,
                            ui.make_persistent_id(("egui_shell.result_tab", rs.id.as_str(), *idx)),
                            if enabled { Sense::click() } else { Sense::hover() },
                        );
                        let bg = if !enabled {
                            tokens.panel_bg
                        } else if selected {
                            tokens.active_bg
                        } else if resp.hovered() {
                            tokens.hover_bg
                        } else {
                            tokens.surface_bg
                        };
                        let fg = if !enabled {
                            tokens.text_disabled
                        } else if selected {
                            tokens.text_strong
                        } else {
                            tokens.text_primary
                        };
                        let border = if !enabled {
                            tokens.divider
                        } else if selected {
                            tokens.accent_strong
                        } else {
                            tokens.border
                        };
                        let painter = ui.painter().with_clip_rect(vis);
                        painter.rect_filled(vis, CornerRadius::same(4), bg);
                        painter.rect_stroke(vis, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);

                        let pad_x = 6.0_f32;
                        let avail_text_w = (vis.width() - pad_x * 2.0).max(4.0);
                        let mut job = egui::text::LayoutJob::single_section(
                            tab.title.clone(),
                            egui::text::TextFormat { font_id: tab_font.clone(), color: fg, ..Default::default() },
                        );
                        job.wrap =
                            egui::text::TextWrapping { max_width: avail_text_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                        let ellipsized = galley.elided;
                        if galley.size().x <= vis.width() - pad_x || ellipsized {
                            let tx = if galley.size().x + pad_x * 2.0 <= vis.width() {
                                (vis.center().x - galley.size().x * 0.5).max(vis.left() + pad_x)
                            } else {
                                vis.left() + pad_x
                            };
                            let ty = vis.center().y - galley.size().y * 0.5;
                            painter.galley(pos2(tx, ty), galley, fg);
                        }

                        let ak_title = tab.title.clone();
                        resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, enabled, selected, ak_title.clone()));
                        let resp = if !enabled {
                            resp.on_hover_text("modal dialog is active")
                        } else if ellipsized {
                            resp.on_hover_text(&tab.title)
                        } else {
                            resp
                        };

                        let wid = format!("result.tab.{}", tab.id);
                        meta.record_response(
                            ui,
                            &resp,
                            FrameInspectEntryInput {
                                widget_id: &wid,
                                role: "tab",
                                label: &tab.title,
                                command_id: None,
                                value: Some(json!({ "selected": selected, "index": *idx, "tab_id": tab.id })),
                                enabled,
                                disabled_reason: (!enabled).then_some("modal dialog is active"),
                                z_layer: ZLayer::Workspace,
                                ellipsized,
                                full_label: ellipsized.then_some(tab.title.as_str()),
                            },
                        );
                        if resp.clicked() && enabled && !selected {
                            intents.push_select_result_tab(*idx);
                        }
                    }
                });
            }

            let can_next = enabled && tab_count > 1 && active_idx + 1 < tab_count;
            let next_resp = ui.add_enabled(can_next, egui::Button::new(">").min_size(Vec2::new(24.0, 24.0)));
            meta.record_response(
                ui,
                &next_resp,
                FrameInspectEntryInput {
                    widget_id: "result.tabs.next",
                    role: "button",
                    label: tr_or_fallback(&app.locale, "results.next_tabs", "Next Tab"),
                    command_id: None,
                    value: None,
                    enabled: can_next,
                    disabled_reason: (!can_next).then_some("At last result tab"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if next_resp.clicked() && can_next {
                let next_i = (active_idx + 1).min(tab_count - 1);
                intents.push_select_result_tab(next_i);
                if let Some((_, _, x0, w)) = tab_layouts.get(next_i) {
                    let x1 = *x0 + *w;
                    if x1 > state.tab_strip_scroll_x + strip_w {
                        state.tab_strip_scroll_x = (x1 - strip_w).clamp(0.0, max_scroll_x);
                    }
                } else {
                    state.tab_strip_scroll_x = (state.tab_strip_scroll_x + 100.0).clamp(0.0, max_scroll_x);
                }
            }
        });
    });

    let body_rect = Rect::from_min_max(pos2(avail.left(), hdr_rect.bottom()), avail.max);
    report.body_rect_pt = body_rect.is_positive().then_some(body_rect);
    if !body_rect.is_positive() {
        return report;
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
        ui.set_clip_rect(body_rect);
        ui.painter().rect_filled(body_rect, CornerRadius::ZERO, tokens.surface_bg);

        let Some(tab) = rs.tabs.get(active_idx) else {
            render_empty_or_failed_result_body(ui, rs, tokens, enabled, meta, intents);
            return;
        };

        let category = ResultTabCategory::for_kind(tab.kind);
        report.active_tab_category = Some(category);

        // Sub-header strip identifying Inferential Report vs Plot-Ready Derived Matrix
        render_tab_category_bar(ui, rs, tab, category, tokens, enabled, meta, intents);

        let is_survival_summary = matches!(rs.analysis.kind, AnalysisKind::Survival(_)) && matches!(tab.kind, ResultTabKind::MainSummary);
        let tab_has_ambig_cell = tab_contains_ambiguity(tab, rs.analysis.p_value_format, is_survival_summary);
        let curve_ambiguous = matches!(tab.kind, ResultTabKind::MainSummary | ResultTabKind::ParametersAndConfidenceIntervals)
            && rs.fitted_curves.iter().any(|c| c.ambiguous);
        let has_ambiguity = tab_has_ambig_cell || curve_ambiguous;
        report.has_ambiguity_callout = has_ambiguity;

        if has_ambiguity {
            render_ambiguity_callout(ui, rs, tokens, enabled, meta, intents);
        }

        let scroll_out =
            egui::ScrollArea::both().id_salt(("egui_shell.results.body", rs.id.as_str(), &tab.id)).auto_shrink([false, false]).show(ui, |ui| {
                render_result_tab_table(ui, rs, tab, active_idx, is_survival_summary, state, tokens, enabled, meta);
                if !tab.footnotes.is_empty() {
                    ui.add_space(6.0);
                    let fn_max_w = (body_rect.width() - 24.0).max(80.0);
                    ui.allocate_ui(vec2(fn_max_w, 0.0), |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let fn_lbl = tr_or_fallback(&app.locale, "results.footnotes", "Footnotes");
                        ui.add(egui::Label::new(RichText::new(fn_lbl).small().strong().color(tokens.text_strong)).wrap());
                        for note in &tab.footnotes {
                            ui.add(egui::Label::new(RichText::new(note).small().color(tokens.text_muted)).wrap());
                        }
                    });
                }
            });
        state.scroll_offset_y = scroll_out.state.clamped_offset().y;
        if scroll_out.content_size.y > scroll_out.inner_rect.height() + 0.5 {
            let bar_w = ui.spacing().scroll.bar_width.max(10.0);
            report.scrollbar_rect_pt = Some(Rect::from_min_max(
                pos2((scroll_out.inner_rect.right() + 2.0).min(body_rect.right()), scroll_out.inner_rect.top()),
                pos2((scroll_out.inner_rect.right() + 2.0 + bar_w).min(body_rect.right()), scroll_out.inner_rect.bottom()),
            ));
        }
    });

    report
}

fn tab_contains_ambiguity(tab: &ResultTab, p_format: PValueFormat, is_survival_summary: bool) -> bool {
    for row in &tab.rows {
        for cell in row {
            if format_result_cell_semantic(cell, p_format, is_survival_summary).is_ambiguous {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn render_tab_category_bar(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    tab: &ResultTab,
    category: ResultTabCategory,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let bar_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), 26.0)).1;
    ui.painter().rect_filled(bar_rect, CornerRadius::ZERO, tokens.header_bg);
    ui.painter()
        .line_segment([pos2(bar_rect.left(), bar_rect.bottom()), pos2(bar_rect.right(), bar_rect.bottom())], Stroke::new(1.0, tokens.divider));

    ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect.shrink2(vec2(8.0, 2.0))), |ui| {
        ui.set_clip_rect(bar_rect);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let badge_col = match category {
                ResultTabCategory::InferentialReport => tokens.role_num,
                ResultTabCategory::PlotReadyMatrix => tokens.role_fx,
            };
            ui.colored_label(badge_col, RichText::new(format!("[{}]", category.badge_label())).small().strong());
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{} ({}×{})", tab.title, tab.rows.len(), tab.column_headers.len())).small().color(tokens.text_primary),
                )
                .truncate(),
            );

            if category == ResultTabCategory::PlotReadyMatrix {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let graph_btn = ui.add_enabled(enabled, egui::Button::new(RichText::new("New Graph from Tab…").small()));
                    meta.record_response(
                        ui,
                        &graph_btn,
                        FrameInspectEntryInput {
                            widget_id: "result.new_graph_from_tab",
                            role: "button",
                            label: "New Graph from Tab…",
                            command_id: Some("sheet.extractResultTab"),
                            value: Some(json!({ "tab_id": tab.id })),
                            enabled,
                            disabled_reason: (!enabled).then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if graph_btn.clicked() && enabled {
                        intents.push_engine(
                            "sheet.extractResultTab",
                            json!({
                                "result_sheet_id": rs.id.as_str(),
                                "tab_id": tab.id,
                                "auto_graph": true
                            }),
                        );
                    }
                });
            }
        });
    });
}

fn render_ambiguity_callout(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let banner_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), 28.0)).1;
    let fill = if tokens.dark { egui::Color32::from_rgb(66, 48, 22) } else { egui::Color32::from_rgb(255, 247, 224) };
    let border = tokens.role_cat;
    ui.painter().rect_filled(banner_rect, CornerRadius::ZERO, fill);
    ui.painter().rect_stroke(banner_rect, CornerRadius::ZERO, Stroke::new(1.0, border), StrokeKind::Inside);

    ui.scope_builder(egui::UiBuilder::new().max_rect(banner_rect.shrink2(vec2(8.0, 2.0))), |ui| {
        ui.set_clip_rect(banner_rect);
        ui.horizontal(|ui| {
            let msg = "??? Ambiguous / Wide CI: Profile-likelihood or covariance bounds did not close; inspect parameter constraints.";
            let lbl = ui.add(egui::Label::new(RichText::new(msg).small().color(tokens.text_strong)).truncate());
            meta.record_response(
                ui,
                &lbl,
                FrameInspectEntryInput {
                    widget_id: "result.ambiguity_callout",
                    role: "label",
                    label: msg,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            lbl.on_hover_text(msg);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn = ui.add_enabled(enabled, egui::Button::new(RichText::new("Edit Params").small()));
                if btn.clicked() && enabled {
                    intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
                }
            });
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn render_result_tab_table(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    tab: &ResultTab,
    active_tab_idx: usize,
    is_survival_summary: bool,
    state: &mut ResultsViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) {
    let num_cols = tab.column_headers.len().max(tab.rows.iter().map(Vec::len).max().unwrap_or(0));
    let row_stub_w = 190.0_f32;
    let col_w = 128.0_f32;
    let row_h = 24.0_f32;
    let total_w = (row_stub_w + (num_cols as f32) * col_w).max(ui.available_width());

    let visible_clip = ui.clip_rect().intersect(meta.viewport_rect);

    // Header Row
    let hdr_rect = ui.allocate_space(vec2(total_w, row_h)).1;
    let hdr_vis = hdr_rect.intersect(visible_clip);
    if hdr_vis.width() >= 4.0 && hdr_vis.height() >= 4.0 {
        let hdr_painter = ui.painter().with_clip_rect(hdr_vis);
        hdr_painter.rect_filled(hdr_vis, CornerRadius::ZERO, tokens.header_bg);
        hdr_painter.rect_stroke(hdr_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);

        let corner_rect = Rect::from_min_size(hdr_rect.min, vec2(row_stub_w, row_h));
        let corner_vis = corner_rect.intersect(hdr_vis);
        if corner_vis.width() >= 4.0 && corner_vis.height() >= 4.0 {
            let corner_lbl = tr_or_fallback(tokens.locale, "results.row_metric", "Row / Metric");
            let _ = paint_table_cell_text(
                ui,
                corner_rect,
                corner_lbl,
                tokens.text_strong,
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                egui::Align::LEFT,
                visible_clip,
            );
        }

        for c_idx in 0..num_cols {
            let cell_rect = Rect::from_min_size(pos2(hdr_rect.left() + row_stub_w + (c_idx as f32) * col_w, hdr_rect.top()), vec2(col_w, row_h));
            let cell_vis = cell_rect.intersect(hdr_vis);
            if cell_vis.width() < 4.0 || cell_vis.height() < 4.0 {
                continue;
            }
            hdr_painter.line_segment([cell_rect.left_top(), cell_rect.left_bottom()], Stroke::new(1.0, tokens.divider));
            let h_txt = tab.column_headers.get(c_idx).map_or("", String::as_str);
            let fallback_col = format!("Col {}", c_idx + 1);
            let display_h = if h_txt.is_empty() { fallback_col.as_str() } else { h_txt };
            let _ = paint_table_cell_text(
                ui,
                cell_rect,
                display_h,
                tokens.text_strong,
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                egui::Align::RIGHT,
                visible_clip,
            );
        }
    }

    // Data & Section Header Rows
    let mut current_section: Option<String> = None;
    for (r_idx, row) in tab.rows.iter().enumerate() {
        let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
        let is_section_row = !row_lbl.is_empty() && row.iter().all(|c| matches!(c, ResultCell::Empty));
        if is_section_row {
            let sec_key = format!("{}:{}:{r_idx}", rs.id.as_str(), tab.id);
            let is_collapsed = state.collapsed_sections.contains(&sec_key);
            current_section = Some(sec_key.clone());

            let sec_rect = ui.allocate_space(vec2(total_w, row_h)).1;
            let sec_vis = sec_rect.intersect(visible_clip);
            if sec_vis.width() < 12.0 || sec_vis.height() < 4.0 {
                continue;
            }
            let sec_painter = ui.painter().with_clip_rect(sec_vis);
            sec_painter.rect_filled(sec_vis, CornerRadius::ZERO, tokens.panel_bg);
            sec_painter.rect_stroke(sec_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);

            let chevron = if is_collapsed { "▸" } else { "▾" };
            let sec_text = format!("{chevron} {row_lbl}");
            let sec_elided = paint_table_cell_text(
                ui,
                sec_rect,
                &sec_text,
                tokens.text_strong,
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                egui::Align::LEFT,
                visible_clip,
            );
            let sec_resp = ui.interact(sec_vis, egui::Id::unique(("res_sec", &sec_key)), if enabled { Sense::click() } else { Sense::hover() });
            let sec_ak = sec_text.clone();
            sec_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, sec_ak.clone()));
            let sec_wid = format!("result.section.{r_idx}");
            meta.record_response(
                ui,
                &sec_resp,
                FrameInspectEntryInput {
                    widget_id: &sec_wid,
                    role: "button",
                    label: &sec_text,
                    command_id: None,
                    value: Some(json!({ "collapsed": is_collapsed })),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: sec_elided,
                    full_label: sec_elided.then_some(sec_text.as_str()),
                },
            );
            if sec_resp.clicked() && enabled {
                if is_collapsed {
                    state.collapsed_sections.remove(&sec_key);
                } else {
                    state.collapsed_sections.insert(sec_key);
                }
            }
            continue;
        }

        if let Some(sec_key) = &current_section
            && state.collapsed_sections.contains(sec_key)
        {
            continue;
        }

        let row_rect = ui.allocate_space(vec2(total_w, row_h)).1;
        let row_vis = row_rect.intersect(visible_clip);
        if row_vis.width() < 4.0 || row_vis.height() < 4.0 {
            continue;
        }
        let bg = if r_idx % 2 == 1 { tokens.panel_bg } else { tokens.surface_bg };
        let row_painter = ui.painter().with_clip_rect(row_vis);
        row_painter.rect_filled(row_vis, CornerRadius::ZERO, bg);
        row_painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(1.0, tokens.divider));

        let stub_rect = Rect::from_min_size(row_rect.min, vec2(row_stub_w, row_h));
        if stub_rect.intersect(row_vis).width() >= 4.0 {
            let _ = paint_table_cell_text(
                ui,
                stub_rect,
                row_lbl,
                tokens.text_primary,
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                egui::Align::LEFT,
                visible_clip,
            );
        }

        for c_idx in 0..num_cols {
            let cell_rect = Rect::from_min_size(pos2(row_rect.left() + row_stub_w + (c_idx as f32) * col_w, row_rect.top()), vec2(col_w, row_h));
            let cell_vis = cell_rect.intersect(row_vis);
            if cell_vis.width() < 4.0 || cell_vis.height() < 4.0 {
                continue;
            }
            row_painter.line_segment([cell_rect.left_top(), cell_rect.left_bottom()], Stroke::new(1.0, tokens.divider));
            let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
            let fmt = format_result_cell_semantic(cell, rs.analysis.p_value_format, is_survival_summary);

            let selected =
                state.selected_cell.as_ref().is_some_and(|(sid, t, r, c)| sid == &rs.id && *t == active_tab_idx && *r == r_idx && *c == c_idx);
            let cell_resp = ui.interact(
                cell_vis,
                egui::Id::unique(("res_cell", rs.id.as_str(), active_tab_idx, r_idx, c_idx)),
                if enabled { Sense::click() } else { Sense::hover() },
            );
            if selected {
                let cell_painter = ui.painter().with_clip_rect(cell_vis);
                cell_painter.rect_filled(cell_vis, CornerRadius::ZERO, tokens.active_bg);
                cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(1.5, tokens.accent), StrokeKind::Inside);
            }
            if cell_resp.clicked() && enabled {
                state.selected_cell = Some((rs.id.clone(), active_tab_idx, r_idx, c_idx));
                state.hovered_row = Some(r_idx);
            }

            if !fmt.text.is_empty() {
                let col = if fmt.is_ambiguous {
                    tokens.role_cat
                } else if fmt.is_significant {
                    tokens.accent
                } else {
                    tokens.text_primary
                };
                let font = FontId::monospace(tokens.metrics.font_grid_numeral_pt - 1.0);
                let elided = paint_table_cell_text(ui, cell_rect, &fmt.text, col, font, egui::Align::RIGHT, visible_clip);
                if elided || fmt.is_ambiguous {
                    let tip = if fmt.is_ambiguous {
                        format!("{} — Profile-likelihood bounds did not close (ambiguous / wide estimate)", fmt.text)
                    } else {
                        fmt.text.clone()
                    };
                    cell_resp.on_hover_text(tip);
                }
            }
        }
    }

    let _ = meta;
}

fn render_empty_or_failed_result_body(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        match &rs.status {
            ResultStatus::Failed { code, message } => {
                ui.colored_label(tokens.link_red, format!("Analysis failed ({}): {message}", code.as_str()));
                let btn = ui.add_enabled(enabled, egui::Button::new("Edit Parameters"));
                meta.record_response(
                    ui,
                    &btn,
                    FrameInspectEntryInput {
                        widget_id: "result.failed.edit_params",
                        role: "button",
                        label: "Edit Parameters",
                        command_id: Some("ui.editParams.open"),
                        value: None,
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if btn.clicked() && enabled {
                    intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
                }
            }
            ResultStatus::Dirty => {
                ui.colored_label(tokens.text_muted, tr_or_fallback(tokens.locale, "w6.view.results.status_pending", "Status: Recalculation pending"));
            }
            ResultStatus::UpToDate => {
                ui.colored_label(tokens.text_muted, "No tabular output generated for this result sheet.");
            }
        }
    });
}

fn paint_table_cell_text(
    ui: &egui::Ui,
    rect: Rect,
    text: &str,
    color: egui::Color32,
    font_id: FontId,
    align: egui::Align,
    visible_clip: Rect,
) -> bool {
    let pad_x = 6.0;
    let clipped = ui.clip_rect().intersect(visible_clip).intersect(rect);
    if text.is_empty() || clipped.width() <= pad_x * 2.0 + 4.0 || clipped.height() <= 4.0 {
        return !text.is_empty();
    }
    let h_clipped = clipped.left() > rect.left() + 0.5 || clipped.right() < rect.right() - 0.5;
    let max_w = (clipped.width() - pad_x * 2.0).max(1.0);
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::text::TextFormat { font_id, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: h_clipped, overflow_character: Some('…') };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let elided = galley.elided;
    let size = galley.size();
    if size.x > clipped.width() - pad_x && !elided {
        return true;
    }
    let min_x = clipped.left() + pad_x;
    let max_x = (clipped.right() - pad_x - size.x).max(min_x);
    let x = match align {
        egui::Align::RIGHT => max_x,
        egui::Align::Center => (clipped.center().x - size.x * 0.5).clamp(min_x, max_x),
        egui::Align::LEFT => min_x,
    };
    let y = rect.center().y - size.y * 0.5;
    ui.painter().with_clip_rect(clipped).galley(pos2(x, y), galley, color);
    elided
}

fn ensure_solid_scrollbar_style(ui: &mut egui::Ui, tokens: &Tokens) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    *scroll = egui::style::ScrollStyle::solid();
    scroll.floating = false;
    scroll.bar_width = tokens.metrics.scroll_bar_thickness.max(10.0);
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
}
