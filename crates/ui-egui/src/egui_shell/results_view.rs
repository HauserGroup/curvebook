//! Native `egui` multi-tab reactive `ResultSheet` viewer (`docs/planning/gui-layout-spec.md` §8.10, `R15.3`).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{AnalysisKind, CiBound, PValueFormat, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId};
use curvebook_palette::lerp_oklab;
use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::ResultSheetViewState;
use crate::egui_shell::{
    FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens, UiIntent, color32_to_rgba8, rgba8_to_color32,
};
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

/// Optional find-in-sheet highlight overlay painted on matching cells in the active `ResultSheet` tab (`R15`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResultsFindHighlight {
    pub tab_idx: usize,
    pub active_match: Option<(usize, usize)>,
    pub matching_cells: BTreeSet<(usize, usize)>,
}

/// Persistent `egui_shell` state for the multi-tab `ResultSheet` viewer (`R15.3`, `R13.4`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultsViewEguiState {
    pub scroll_offset_y: f32,
    pub scroll_offset_x: f32,
    pub hovered_row: Option<usize>,
    pub selection_anchor: Option<(SheetId, usize, usize, usize)>,
    pub selected_cell: Option<(SheetId, usize, usize, usize)>,
    pub dragging_selection: bool,
    pub context_menu: Option<(SheetId, usize, Pos2)>,
    pub collapsed_sections: BTreeSet<String>,
    pub tab_strip_scroll_x: f32,
    pub tab_overflow_open: bool,
    pub active_find_matches: Option<ResultsFindHighlight>,
    pub pending_scroll_to_match: Option<(SheetId, usize, usize, usize)>,
    /// Per-sheet active tab index so split-pane secondary `ResultSheet` views switch tabs independently.
    pub per_sheet_active_tab: BTreeMap<SheetId, usize>,
    /// Last `(sheet_id, tab_index)` observed from `app.canvas_dialogs.result_view.active_tab_index` on the primary sheet.
    pub last_synced_primary_tab: Option<(SheetId, usize)>,
    /// Set when programmatic selection changes (e.g. Find navigation) must be mirrored via `SyncResultSelection`.
    pub selection_sync_needed: bool,
}

impl ResultsViewEguiState {
    /// Resolve the active tab index for `sheet_id` using per-sheet state when present.
    #[must_use]
    pub fn active_tab_for_sheet(&self, sheet_id: &SheetId, tab_count: usize, fallback_tab_index: usize) -> usize {
        if tab_count == 0 {
            return 0;
        }
        self.per_sheet_active_tab.get(sheet_id).copied().unwrap_or(fallback_tab_index).min(tab_count - 1)
    }

    /// Request scrolling the results table to `(tab_idx, row_idx, col_idx)` and selecting that cell.
    pub fn scroll_to_match(&mut self, sheet_id: SheetId, tab_idx: usize, row_idx: usize, col_idx: usize) {
        self.per_sheet_active_tab.insert(sheet_id.clone(), tab_idx);
        self.selection_anchor = Some((sheet_id.clone(), tab_idx, row_idx, col_idx));
        self.selected_cell = Some((sheet_id.clone(), tab_idx, row_idx, col_idx));
        self.selection_sync_needed = true;
        self.pending_scroll_to_match = Some((sheet_id, tab_idx, row_idx, col_idx));
    }

    /// Active rectangular selection `(min_row, max_row, min_col, max_col)` when bound to `(sheet_id, tab_idx)`.
    #[must_use]
    pub fn selected_range(&self, sheet_id: &SheetId, tab_idx: usize) -> Option<(usize, usize, usize, usize)> {
        let (sid_f, t_f, rf, cf) = self.selected_cell.as_ref()?;
        if sid_f != sheet_id || *t_f != tab_idx {
            return None;
        }
        let (ra, ca) = self
            .selection_anchor
            .as_ref()
            .filter(|(sid_a, t_a, _, _)| sid_a == sheet_id && *t_a == tab_idx)
            .map_or((*rf, *cf), |(_, _, ra, ca)| (*ra, *ca));
        Some((ra.min(*rf), ra.max(*rf), ca.min(*cf), ca.max(*cf)))
    }
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

fn contains_ascii_case_insensitive(haystack: &str, needle_lower: &[u8]) -> bool {
    !needle_lower.is_empty() && haystack.as_bytes().windows(needle_lower.len()).any(|w| w.eq_ignore_ascii_case(needle_lower))
}

fn result_cell_is_ambiguous(cell: &ResultCell, is_survival_summary: bool) -> bool {
    match cell {
        ResultCell::Empty | ResultCell::PValue { .. } | ResultCell::Boolean(_) => false,
        ResultCell::Number { value, prefix, .. } => value.is_finite() && prefix.as_deref() == Some("~"),
        ResultCell::ConfidenceInterval { lower, upper } => !is_survival_summary && (matches!(lower, CiBound::Open) || matches!(upper, CiBound::Open)),
        ResultCell::Text(s) => s.contains("???") || contains_ascii_case_insensitive(s, b"ambiguous"),
    }
}

fn format_result_cell_semantic(cell: &ResultCell, p_format: PValueFormat, is_survival_summary: bool, sig_figs: Option<u8>) -> FormattedCellDisplay {
    match cell {
        ResultCell::Empty => FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false },
        ResultCell::Number { value, prefix, units } => {
            if !value.is_finite() {
                // Unreached medians or undefined scalars must remain explicitly blank, never "NaN" or "0".
                return FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false };
            }
            let base = if let Some(d) = sig_figs {
                let mut out = String::new();
                if let Some(p) = prefix {
                    out.push_str(p);
                }
                out.push_str(&curvebook_doc::format_significant_digits(*value, d));
                if let Some(u) = units.as_deref().filter(|u| !u.is_empty()) {
                    out.push(' ');
                    out.push_str(u);
                }
                out
            } else {
                ResultSheetViewState::format_result_cell(cell, p_format)
            };
            let is_amb = prefix.as_deref() == Some("~");
            FormattedCellDisplay { text: base, is_ambiguous: is_amb, is_significant: false }
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            let has_open = matches!(lower, CiBound::Open) || matches!(upper, CiBound::Open);
            if is_survival_summary {
                let lo = format_survival_ci_bound(*lower);
                let hi = format_survival_ci_bound(*upper);
                let text = if lo.is_empty() && hi.is_empty() { String::new() } else { format!("[{lo}, {hi}]") };
                FormattedCellDisplay { text, is_ambiguous: false, is_significant: false }
            } else if has_open {
                let lo = format_ci_bound_semantic_sig(*lower, sig_figs);
                let hi = format_ci_bound_semantic_sig(*upper, sig_figs);
                let text = if matches!(lower, CiBound::Open) && matches!(upper, CiBound::Open) {
                    String::from("??? (ambiguous)")
                } else {
                    format!("[{lo}, {hi}] (???)")
                };
                FormattedCellDisplay { text, is_ambiguous: true, is_significant: false }
            } else if let Some(d) = sig_figs {
                let lo = format_ci_bound_semantic_sig(*lower, Some(d));
                let hi = format_ci_bound_semantic_sig(*upper, Some(d));
                FormattedCellDisplay { text: format!("[{lo}, {hi}]"), is_ambiguous: false, is_significant: false }
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
            let is_amb = s.contains("???") || contains_ascii_case_insensitive(s, b"ambiguous");
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

fn format_ci_bound_semantic_sig(bound: CiBound, sig_figs: Option<u8>) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => match sig_figs {
            Some(d) => curvebook_doc::format_significant_digits(v, d),
            None => format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_owned(),
        },
        CiBound::Finite(_) | CiBound::Open => String::from("???"),
    }
}

/// Format `cell` identically to how [`render_results_view_for_sheet`] displays it on screen.
#[must_use]
pub fn format_result_cell_display_text(cell: &ResultCell, rs: &ResultSheet, tab: &ResultTab) -> String {
    let is_survival_summary = matches!(rs.analysis.kind, AnalysisKind::Survival(_)) && matches!(tab.kind, ResultTabKind::MainSummary);
    let sig_figs = match &rs.analysis.kind {
        AnalysisKind::NonlinearRegression(nl) => nl.output_significant_digits,
        _ => None,
    };
    format_result_cell_semantic(cell, rs.analysis.p_value_format, is_survival_summary, sig_figs).text
}

fn select_result_tab_for_sheet(
    state: &mut ResultsViewEguiState,
    sheet_id: &SheetId,
    new_idx: usize,
    is_primary_sheet: bool,
    intents: &mut PendingUiIntents,
) {
    state.per_sheet_active_tab.insert(sheet_id.clone(), new_idx);
    state.selection_anchor = None;
    state.selected_cell = None;
    if is_primary_sheet {
        state.last_synced_primary_tab = Some((sheet_id.clone(), new_idx));
        intents.push(UiIntent::SelectResultTab { index: new_idx });
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
    let is_primary_sheet = doc.active_sheet_id.as_ref() == Some(&rs.id);
    let shared_tab = if tab_count == 0 { 0 } else { app.canvas_dialogs.result_view.active_tab_index.min(tab_count - 1) };
    let mut active_idx = if tab_count == 0 {
        0
    } else if is_primary_sheet {
        let shared_changed = state.last_synced_primary_tab.as_ref().is_none_or(|(sid, idx)| sid != &rs.id || *idx != shared_tab);
        if shared_changed {
            state.per_sheet_active_tab.insert(rs.id.clone(), shared_tab);
            state.last_synced_primary_tab = Some((rs.id.clone(), shared_tab));
            shared_tab
        } else {
            state.per_sheet_active_tab.get(&rs.id).copied().unwrap_or(shared_tab).min(tab_count - 1)
        }
    } else {
        state.per_sheet_active_tab.get(&rs.id).copied().unwrap_or(0).min(tab_count - 1)
    };

    // Ctrl+PageUp / Ctrl+PageDown keyboard switching across sub-tabs (§8.10)
    if enabled && tab_count > 1 && !ui.ctx().egui_wants_keyboard_input() {
        let prev_pressed = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::PageUp) || i.consume_key(Modifiers::COMMAND, Key::PageUp));
        let next_pressed = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::PageDown) || i.consume_key(Modifiers::COMMAND, Key::PageDown));
        if prev_pressed {
            active_idx = if active_idx == 0 { tab_count - 1 } else { active_idx - 1 };
            select_result_tab_for_sheet(state, &rs.id, active_idx, is_primary_sheet, intents);
        } else if next_pressed {
            active_idx = (active_idx + 1) % tab_count;
            select_result_tab_for_sheet(state, &rs.id, active_idx, is_primary_sheet, intents);
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

            if hdr_rect.width() >= 420.0 {
                let copy_lbl = tr_or_fallback(&app.locale, "btn.copy", "Copy");
                let has_tabs = !rs.tabs.is_empty();
                let copy_resp = ui.add_enabled(enabled && has_tabs, egui::Button::new(RichText::new(copy_lbl).small()));
                meta.record_response(
                    ui,
                    &copy_resp,
                    FrameInspectEntryInput {
                        widget_id: "result.copy_selection",
                        role: "button",
                        label: copy_lbl,
                        command_id: None,
                        value: None,
                        enabled: enabled && has_tabs,
                        disabled_reason: (!enabled || !has_tabs).then_some("No active result tab to copy"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if copy_resp.clicked() && enabled && has_tabs {
                    intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: false });
                }
            }

            ui.separator();

            let avail_strip_w = ui.available_width().max(0.0);
            let max_tab_cap = if tab_count > 5 { 96.0 } else { 120.0 };
            let provisional_max_tab_w = (avail_strip_w - 4.0).clamp(36.0, max_tab_cap);
            let gap = tokens.metrics.space_xs;
            let tab_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
            let tab_text_budget = (provisional_max_tab_w - 14.0).max(8.0);
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
                let w = (g.size().x + 14.0).clamp(36.0_f32.min(provisional_max_tab_w), provisional_max_tab_w);
                tab_layouts.push((idx, tab, cur_x, w));
                cur_x += w + gap;
            }
            let total_tabs_w = (cur_x - gap).max(0.0);
            let overflow_btn_w = 50.0_f32;
            let needs_overflow = tab_count > 1 && total_tabs_w > avail_strip_w + 0.5;
            let strip_w = if needs_overflow { (avail_strip_w - overflow_btn_w - tokens.metrics.space_sm).max(0.0) } else { avail_strip_w };

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

            let tab_h = tokens.metrics.min_hit_target_pt;
            let (strip_rect, strip_resp) = ui.allocate_exact_size(vec2(strip_w, tab_h), Sense::hover());
            if strip_resp.hovered() && max_scroll_x > 0.0 {
                let wheel = ui.input(|i| i.smooth_scroll_delta.x - i.smooth_scroll_delta.y);
                if wheel.abs() > 0.1 {
                    state.tab_strip_scroll_x = (state.tab_strip_scroll_x - wheel).clamp(0.0, max_scroll_x);
                }
            }

            let strip_clip = strip_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            let mut hidden_tabs: Vec<(usize, String, String, f32, f32)> = Vec::new();
            for (idx, tab, x0, w) in &tab_layouts {
                let selected = *idx == active_idx;
                let tab_rect = Rect::from_min_size(pos2(strip_rect.left() - state.tab_strip_scroll_x + *x0, strip_rect.top()), vec2(*w, tab_h));
                let vis = tab_rect.intersect(strip_clip);
                if strip_w < 28.0 || !vis.is_positive() || vis.width() < 28.0 || vis.height() < 8.0 {
                    hidden_tabs.push((*idx, tab.id.clone(), tab.title.clone(), *x0, *w));
                    continue;
                }
                if vis.width() + 2.0 < *w {
                    hidden_tabs.push((*idx, tab.id.clone(), tab.title.clone(), *x0, *w));
                }
                if paint_result_tab_chip(
                    ui,
                    &ResultTabChipSpec {
                        sheet_id: rs.id.as_str(),
                        idx: *idx,
                        tab_id: &tab.id,
                        tab_title: &tab.title,
                        vis,
                        selected,
                        enabled,
                        tokens,
                        tab_font: &tab_font,
                    },
                    meta,
                ) {
                    active_idx = *idx;
                    select_result_tab_for_sheet(state, &rs.id, *idx, is_primary_sheet, intents);
                }
            }

            if needs_overflow && !hidden_tabs.is_empty() {
                let badge_lbl = format!("⋯ +{}", hidden_tabs.len());
                let ov_tip = format!("{} (+{})", tr(&app.locale, "toolbar.overflow"), hidden_tabs.len());
                let ov_resp = ui.add_enabled(enabled, egui::Button::new(&badge_lbl).min_size(vec2(overflow_btn_w, tab_h))).on_hover_text(&ov_tip);
                meta.record_response(
                    ui,
                    &ov_resp,
                    FrameInspectEntryInput {
                        widget_id: "result.tabs.overflow",
                        role: "button",
                        label: &badge_lbl,
                        command_id: None,
                        value: Some(json!({ "hidden_count": hidden_tabs.len(), "open": state.tab_overflow_open })),
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: Some(&ov_tip),
                    },
                );
                if ov_resp.clicked() && enabled {
                    state.tab_overflow_open = !state.tab_overflow_open;
                }
                if !enabled {
                    state.tab_overflow_open = false;
                } else if state.tab_overflow_open
                    && let Some(picked_idx) = render_result_tabs_overflow_popup(
                        ui.ctx(),
                        rs.id.as_str(),
                        tokens,
                        state,
                        &hidden_tabs,
                        active_idx,
                        strip_w,
                        max_scroll_x,
                        ov_resp.rect,
                        meta,
                    )
                {
                    active_idx = picked_idx;
                    select_result_tab_for_sheet(state, &rs.id, picked_idx, is_primary_sheet, intents);
                }
            } else {
                state.tab_overflow_open = false;
            }
        });
    });
    report.active_tab_index = active_idx;

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
        let tab_ctx = ResultTabRenderCtx { rs, tab, tokens, enabled };
        render_tab_category_bar(ui, &tab_ctx, category, meta, intents);

        let is_survival_summary = matches!(rs.analysis.kind, AnalysisKind::Survival(_)) && matches!(tab.kind, ResultTabKind::MainSummary);
        let tab_has_ambig_cell = tab_contains_ambiguity(tab, rs.analysis.p_value_format, is_survival_summary);
        let curve_ambiguous = matches!(tab.kind, ResultTabKind::MainSummary | ResultTabKind::ParametersAndConfidenceIntervals)
            && rs.fitted_curves.iter().any(|c| c.ambiguous);
        let has_ambiguity = tab_has_ambig_cell || curve_ambiguous;
        report.has_ambiguity_callout = has_ambiguity;

        if has_ambiguity {
            render_ambiguity_callout(ui, rs, tokens, enabled, meta, intents);
        }

        let (scroll_to_y, scroll_to_x) = if state.pending_scroll_to_match.as_ref().is_some_and(|(sid, t, _, _)| sid == &rs.id && *t == active_idx) {
            state.pending_scroll_to_match.take().map_or((None, None), |(_, _, target_row, target_col)| {
                expand_section_containing_row(&rs.id, tab, target_row, &mut state.collapsed_sections);
                let vis_row = visible_row_index_for_raw_row(&rs.id, tab, target_row, &state.collapsed_sections);
                let row_stride = 24.0_f32 + ui.spacing().item_spacing.y;
                #[allow(clippy::cast_precision_loss)]
                let y = ((vis_row as f32) * row_stride - row_stride * 2.0).max(0.0);
                #[allow(clippy::cast_precision_loss)]
                let x = if target_col == 0 { 0.0 } else { (190.0_f32 + (target_col as f32) * 128.0 - 128.0).max(0.0) };
                state.scroll_offset_y = y;
                state.scroll_offset_x = x;
                (Some(y), Some(x))
            })
        } else {
            (None, None)
        };

        let mut scroll_area = egui::ScrollArea::both()
            .id_salt(("egui_shell.results.body", rs.id.as_str(), &tab.id))
            .scroll_bar_visibility(if enabled {
                egui::scroll_area::ScrollBarVisibility::AlwaysVisible
            } else {
                egui::scroll_area::ScrollBarVisibility::AlwaysHidden
            })
            .auto_shrink([false, false]);
        if let Some(target_y) = scroll_to_y {
            scroll_area = scroll_area.vertical_scroll_offset(target_y);
        }
        if let Some(target_x) = scroll_to_x {
            scroll_area = scroll_area.horizontal_scroll_offset(target_x);
        }
        let scroll_out = scroll_area.show_viewport(ui, |ui, viewport| {
            render_result_tab_table(ui, &tab_ctx, active_idx, is_survival_summary, viewport, state, meta, intents);
            if !tab.footnotes.is_empty() {
                ui.add_space(6.0);
                let fn_max_w = (body_rect.width() - 24.0).max(80.0);
                let clip_r = ui.clip_rect();
                let paint_clipped_note = |ui: &mut egui::Ui, salt: usize, text: &str, color: Color32, bold: bool| {
                    let mut fmt = egui::TextFormat::simple(FontId::proportional(tokens.metrics.font_ui_small_pt), color);
                    if bold {
                        fmt.font_id = super::fonts::section_heading_font_id(ui.ctx(), tokens);
                    }
                    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), fmt);
                    job.wrap = egui::text::TextWrapping { max_width: fn_max_w, ..Default::default() };
                    let galley = ui.painter().layout_job(job);
                    let raw_r = ui.allocate_space(vec2(fn_max_w, galley.size().y.max(14.0))).1;
                    let vis_r = raw_r.intersect(clip_r);
                    if vis_r.width() >= 2.0 && vis_r.height() >= 2.0 {
                        ui.painter().with_clip_rect(vis_r).galley(raw_r.min, galley, color);
                        let n_resp = ui.interact(vis_r, Id::unique(("res_fn", rs.id.as_str(), &tab.id, salt)), Sense::hover());
                        let ak_txt = text.to_owned();
                        n_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, ak_txt.clone()));
                    }
                };
                let fn_lbl = tr_or_fallback(&app.locale, "results.footnotes", "Footnotes");
                paint_clipped_note(ui, 0, fn_lbl, tokens.text_strong, true);
                for (idx, note) in tab.footnotes.iter().enumerate() {
                    paint_clipped_note(ui, idx + 1, note, tokens.text_muted, false);
                }
            }
        });
        state.scroll_offset_y = scroll_out.state.clamped_offset().y;
        state.scroll_offset_x = scroll_out.state.clamped_offset().x;
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

fn expand_section_containing_row(sheet_id: &SheetId, tab: &ResultTab, target_row: usize, collapsed_sections: &mut BTreeSet<String>) {
    if collapsed_sections.is_empty() || tab.rows.is_empty() {
        return;
    }
    let upper = target_row.min(tab.rows.len().saturating_sub(1));
    for r in (0..=upper).rev() {
        let lbl = tab.row_labels.get(r).map_or("", String::as_str);
        if !lbl.is_empty() && tab.rows[r].iter().all(|c| matches!(c, ResultCell::Empty)) {
            let sec_key = format!("{}:{}:{r}", sheet_id.as_str(), tab.id);
            collapsed_sections.remove(&sec_key);
            break;
        }
    }
}

/// Map a raw `tab.rows` index to its visible row index after accounting for collapsed sections.
#[must_use]
pub fn visible_row_index_for_raw_row(sheet_id: &SheetId, tab: &ResultTab, target_row: usize, collapsed_sections: &BTreeSet<String>) -> usize {
    if tab.rows.is_empty() {
        return 0;
    }
    let clamped = target_row.min(tab.rows.len().saturating_sub(1));
    if collapsed_sections.is_empty() {
        return clamped;
    }
    let mut visible_idx = 0usize;
    let mut current_section: Option<String> = None;
    for r_idx in 0..clamped {
        let lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
        let is_sec = !lbl.is_empty() && tab.rows[r_idx].iter().all(|c| matches!(c, ResultCell::Empty));
        if is_sec {
            current_section = Some(format!("{}:{}:{r_idx}", sheet_id.as_str(), tab.id));
            visible_idx += 1;
        } else if !current_section.as_ref().is_some_and(|sec_key| collapsed_sections.contains(sec_key)) {
            visible_idx += 1;
        }
    }
    visible_idx
}

fn tab_contains_ambiguity(tab: &ResultTab, _p_format: PValueFormat, is_survival_summary: bool) -> bool {
    tab.rows.iter().any(|row| row.iter().any(|cell| result_cell_is_ambiguous(cell, is_survival_summary)))
}

struct ResultTabRenderCtx<'a> {
    rs: &'a ResultSheet,
    tab: &'a ResultTab,
    tokens: &'a Tokens,
    enabled: bool,
}

fn render_tab_category_bar(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    category: ResultTabCategory,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ResultTabRenderCtx { rs, tab, tokens, enabled } = *ctx;
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

#[allow(clippy::too_many_arguments)]
fn render_result_tabs_overflow_popup(
    ctx: &egui::Context,
    sheet_id: &str,
    tokens: &Tokens,
    state: &mut ResultsViewEguiState,
    hidden_tabs: &[(usize, String, String, f32, f32)],
    active_idx: usize,
    strip_w: f32,
    max_scroll_x: f32,
    anchor: Rect,
    meta: &mut FrameInspectMeta,
) -> Option<usize> {
    let mut picked_tab: Option<usize> = None;
    let screen = ctx.content_rect();
    let popup_w = 220.0_f32.min((screen.width() - 16.0).max(120.0));
    let est_h = (hidden_tabs.len() as f32 * 28.0 + 16.0).clamp(40.0, 260.0);
    let px = (anchor.right() - popup_w).clamp(screen.left() + 4.0, (screen.right() - popup_w - 4.0).max(screen.left() + 4.0));
    let py = (anchor.bottom() + tokens.metrics.space_xs).clamp(screen.top() + 4.0, (screen.bottom() - est_h - 4.0).max(screen.top() + 4.0));
    let area_id = Id::unique(("egui_shell.result_tabs.overflow", sheet_id));

    let area = egui::Area::new(area_id).fixed_pos(pos2(px, py)).order(Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, tokens.border))
            .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
            .inner_margin(egui::Margin::same(tokens.metrics.space_sm as i8))
            .show(ui, |ui| {
                ui.set_min_width((popup_w - 12.0).max(96.0));
                ui.spacing_mut().item_spacing.y = tokens.metrics.space_xs;
                egui::ScrollArea::vertical().id_salt(("result_tabs_overflow_scroll", sheet_id)).max_height(240.0).auto_shrink([false, true]).show(
                    ui,
                    |ui| {
                        for (idx, tab_id, title, x0, w) in hidden_tabs {
                            let selected = *idx == active_idx;
                            let item_resp = ui.add_sized(
                                vec2(ui.available_width().max(80.0), tokens.metrics.min_hit_target_pt),
                                egui::Button::selectable(selected, title.as_str()),
                            );
                            if item_resp.rect.intersect(ui.clip_rect()).is_positive() {
                                let wid = format!("result.tab.overflow.{tab_id}");
                                meta.record_response(
                                    ui,
                                    &item_resp,
                                    FrameInspectEntryInput {
                                        widget_id: &wid,
                                        role: "menuitem",
                                        label: title,
                                        command_id: None,
                                        value: Some(json!({ "selected": selected, "index": *idx, "tab_id": tab_id })),
                                        enabled: true,
                                        disabled_reason: None,
                                        z_layer: ZLayer::DropdownMenu,
                                        ellipsized: false,
                                        full_label: None,
                                    },
                                );
                            }
                            if item_resp.clicked() {
                                picked_tab = Some(*idx);
                                let x1 = *x0 + *w;
                                state.tab_strip_scroll_x = (x1 - strip_w).max(*x0).clamp(0.0, max_scroll_x);
                                state.tab_overflow_open = false;
                            }
                        }
                    },
                );
            });
    });

    let popup_rect = area.response.rect.intersect(meta.viewport_rect);
    if popup_rect.is_positive() {
        area.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Menu, true, String::from("More Tabs")));
        meta.record(FrameInspectNodeMeta {
            egui_id: area.response.id,
            accesskit_id: area.response.id.accesskit_id(),
            widget_id: String::from("popup.result.tabs.overflow"),
            role: String::from("menu"),
            label: String::from("More Tabs"),
            command_id: None,
            value: Some(json!(true)),
            enabled: true,
            disabled_reason: None,
            unclipped_rect: area.response.rect,
            clipped_rect: popup_rect,
            z_layer: ZLayer::DropdownMenu,
            ellipsized: false,
            full_label: None,
            action_rank: None,
        });
    }

    let esc_pressed = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let outside_clicked =
        ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p)));
    if esc_pressed || outside_clicked {
        state.tab_overflow_open = false;
    }
    picked_tab
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
    let fill = rgba8_to_color32(lerp_oklab(color32_to_rgba8(tokens.surface_bg), color32_to_rgba8(tokens.warning), 0.14));
    let border = tokens.warning;
    ui.painter().rect_filled(banner_rect, CornerRadius::ZERO, fill);
    ui.painter().rect_stroke(banner_rect, CornerRadius::ZERO, Stroke::new(tokens.metrics.stroke_thin_pt, border), StrokeKind::Inside);

    ui.scope_builder(egui::UiBuilder::new().max_rect(banner_rect.shrink2(vec2(tokens.metrics.space_md, tokens.metrics.space_xxs))), |ui| {
        ui.set_clip_rect(banner_rect);
        ui.expand_to_include_rect(banner_rect);
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

#[derive(Clone, Copy)]
struct ResultPointerInput {
    ptr_down: bool,
    ptr_pressed: bool,
    ptr_pos: Option<egui::Pos2>,
    mod_shift_now: bool,
}

#[derive(Clone, Copy)]
struct ResultTableLayout {
    col_start: usize,
    col_end: usize,
    row_stub_w: f32,
    col_w: f32,
    row_h: f32,
    total_w: f32,
    visible_clip: Rect,
}

fn render_result_table_header(ui: &mut egui::Ui, tab: &ResultTab, tokens: &Tokens, layout: ResultTableLayout) {
    let hdr_rect = ui.allocate_space(vec2(layout.total_w, layout.row_h)).1;
    let hdr_vis = hdr_rect.intersect(layout.visible_clip);
    if hdr_vis.width() < 4.0 || hdr_vis.height() < 4.0 {
        return;
    }
    let hdr_painter = ui.painter().with_clip_rect(hdr_vis);
    hdr_painter.rect_filled(hdr_vis, CornerRadius::ZERO, tokens.header_bg);
    hdr_painter.rect_stroke(hdr_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);

    let corner_rect = Rect::from_min_size(hdr_rect.min, vec2(layout.row_stub_w, layout.row_h));
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
            layout.visible_clip,
        );
    }

    for c_idx in layout.col_start..layout.col_end {
        let cell_rect = Rect::from_min_size(
            pos2(hdr_rect.left() + layout.row_stub_w + (c_idx as f32) * layout.col_w, hdr_rect.top()),
            vec2(layout.col_w, layout.row_h),
        );
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
            layout.visible_clip,
        );
    }
}

fn render_result_section_row(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    r_idx: usize,
    row_lbl: &str,
    state: &mut ResultsViewEguiState,
    layout: ResultTableLayout,
    meta: &mut FrameInspectMeta,
) -> String {
    let ResultTabRenderCtx { rs, tab, tokens, enabled } = *ctx;
    let sec_key = format!("{}:{}:{r_idx}", rs.id.as_str(), tab.id);
    let is_collapsed = state.collapsed_sections.contains(&sec_key);
    let out_key = sec_key.clone();

    let sec_rect = ui.allocate_space(vec2(layout.total_w, layout.row_h)).1;
    let sec_vis = sec_rect.intersect(layout.visible_clip);
    if sec_vis.width() < 12.0 || sec_vis.height() < 4.0 {
        return out_key;
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
        layout.visible_clip,
    );
    let sec_resp = ui.interact(sec_vis, egui::Id::unique(("res_sec", &sec_key)), if enabled { Sense::click() } else { Sense::hover() });
    let sec_ak = sec_text.clone();
    sec_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, sec_ak.clone()));
    let sec_wid = format!("result.section.{r_idx}");
    meta.record_response_with_unclipped(
        ui,
        &sec_resp,
        sec_rect,
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
    out_key
}

#[derive(Clone, Copy)]
struct ResultCellAddr<'a> {
    rs_id: &'a SheetId,
    tab_idx: usize,
    r_idx: usize,
    c_idx: usize,
}

fn update_result_cell_selection_from_pointer(
    ui: &egui::Ui,
    cell: ResultCellAddr<'_>,
    cell_vis: Rect,
    cell_resp: &egui::Response,
    ptr: ResultPointerInput,
    state: &mut ResultsViewEguiState,
) {
    let rs_id = cell.rs_id;
    let active_tab_idx = cell.tab_idx;
    let r_idx = cell.r_idx;
    let c_idx = cell.c_idx;
    if ptr.ptr_pressed && ptr.ptr_pos.is_some_and(|p| cell_vis.contains(p)) {
        if !ptr.mod_shift_now {
            state.selection_anchor = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        } else if state.selection_anchor.as_ref().is_none_or(|(sid, t, _, _)| sid != rs_id || *t != active_tab_idx) {
            let same_tab_selected = state.selected_cell.as_ref().filter(|(sid, t, _, _)| sid == rs_id && *t == active_tab_idx).cloned();
            state.selection_anchor = same_tab_selected.or_else(|| Some((rs_id.clone(), active_tab_idx, r_idx, c_idx)));
        }
        state.selected_cell = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        state.hovered_row = Some(r_idx);
        state.dragging_selection = true;
    } else if state.dragging_selection && ptr.ptr_down && ptr.ptr_pos.is_some_and(|p| cell_vis.contains(p)) {
        state.selected_cell = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        state.hovered_row = Some(r_idx);
    } else if cell_resp.clicked() {
        if ptr.mod_shift_now {
            if state.selection_anchor.as_ref().is_none_or(|(sid, t, _, _)| sid != rs_id || *t != active_tab_idx) {
                let same_tab_selected = state.selected_cell.as_ref().filter(|(sid, t, _, _)| sid == rs_id && *t == active_tab_idx).cloned();
                state.selection_anchor = same_tab_selected.or_else(|| Some((rs_id.clone(), active_tab_idx, r_idx, c_idx)));
            }
        } else {
            state.selection_anchor = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        }
        state.selected_cell = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        state.hovered_row = Some(r_idx);
    }
    if cell_resp.secondary_clicked() {
        if state.selected_range(rs_id, active_tab_idx).is_none() {
            state.selection_anchor = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
            state.selected_cell = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
        }
        let anchor = ui.input(|i| i.pointer.interact_pos()).unwrap_or(cell_vis.center());
        state.context_menu = Some((rs_id.clone(), active_tab_idx, anchor));
    }
}

struct ResultDataRowInput<'a> {
    active_tab_idx: usize,
    r_idx: usize,
    row_lbl: &'a str,
    row: &'a [ResultCell],
    is_survival_summary: bool,
    ptr: ResultPointerInput,
    layout: ResultTableLayout,
}

fn render_result_data_row(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    input: ResultDataRowInput<'_>,
    state: &mut ResultsViewEguiState,
    meta: &mut FrameInspectMeta,
) {
    let ResultTabRenderCtx { rs, tokens, enabled, .. } = *ctx;
    let ResultDataRowInput { active_tab_idx, r_idx, row_lbl, row, is_survival_summary, ptr, layout } = input;
    let row_rect = ui.allocate_space(vec2(layout.total_w, layout.row_h)).1;
    let row_vis = row_rect.intersect(layout.visible_clip);
    if row_vis.width() < 4.0 || row_vis.height() < 4.0 {
        return;
    }
    let bg = if r_idx % 2 == 1 { tokens.panel_bg } else { tokens.surface_bg };
    let row_painter = ui.painter().with_clip_rect(row_vis);
    row_painter.rect_filled(row_vis, CornerRadius::ZERO, bg);
    row_painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(1.0, tokens.divider));

    let stub_rect = Rect::from_min_size(row_rect.min, vec2(layout.row_stub_w, layout.row_h));
    if stub_rect.intersect(row_vis).width() >= 4.0 {
        let _ = paint_table_cell_text(
            ui,
            stub_rect,
            row_lbl,
            tokens.text_primary,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            egui::Align::LEFT,
            layout.visible_clip,
        );
    }

    for c_idx in layout.col_start..layout.col_end {
        let cell_rect = Rect::from_min_size(
            pos2(row_rect.left() + layout.row_stub_w + (c_idx as f32) * layout.col_w, row_rect.top()),
            vec2(layout.col_w, layout.row_h),
        );
        let cell_vis = cell_rect.intersect(row_vis);
        if cell_vis.width() < 4.0 || cell_vis.height() < 4.0 {
            continue;
        }
        row_painter.line_segment([cell_rect.left_top(), cell_rect.left_bottom()], Stroke::new(1.0, tokens.divider));
        let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
        let sig_figs = match &rs.analysis.kind {
            AnalysisKind::NonlinearRegression(nl) => nl.output_significant_digits,
            _ => None,
        };
        let fmt = format_result_cell_semantic(cell, rs.analysis.p_value_format, is_survival_summary, sig_figs);

        let cell_resp = ui.interact(
            cell_vis,
            egui::Id::unique(("res_cell", rs.id.as_str(), active_tab_idx, r_idx, c_idx)),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        let ak_cell = if fmt.text.trim().is_empty() { format!("R{}C{}", r_idx + 1, c_idx + 1) } else { fmt.text.clone() };
        cell_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Cell, enabled, ak_cell.clone()));
        let cell_wid = format!("result.cell.{r_idx}.{c_idx}");
        meta.record_response(
            ui,
            &cell_resp,
            FrameInspectEntryInput {
                widget_id: &cell_wid,
                role: "gridcell",
                label: if fmt.text.is_empty() { "Cell" } else { &fmt.text },
                command_id: None,
                value: None,
                enabled,
                disabled_reason: (!enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );

        if enabled && state.context_menu.is_none() {
            update_result_cell_selection_from_pointer(
                ui,
                ResultCellAddr { rs_id: &rs.id, tab_idx: active_tab_idx, r_idx, c_idx },
                cell_vis,
                &cell_resp,
                ptr,
                state,
            );
        }

        let selected =
            state.selected_range(&rs.id, active_tab_idx).is_some_and(|(r0, r1, c0, c1)| r_idx >= r0 && r_idx <= r1 && c_idx >= c0 && c_idx <= c1);
        let find_hl = state.active_find_matches.as_ref().filter(|h| h.tab_idx == active_tab_idx);
        let find_active = find_hl.is_some_and(|h| h.active_match == Some((r_idx, c_idx)));
        let find_match = find_active || find_hl.is_some_and(|h| h.matching_cells.contains(&(r_idx, c_idx)));
        if selected || find_match {
            let cell_painter = ui.painter().with_clip_rect(cell_vis);
            if selected {
                cell_painter.rect_filled(cell_vis, CornerRadius::ZERO, tokens.active_bg);
            }
            if find_match {
                let tint = tokens.warning.linear_multiply(if find_active { 0.38 } else { 0.22 });
                cell_painter.rect_filled(cell_vis, CornerRadius::ZERO, tint);
            }
            if find_active {
                cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.accent), StrokeKind::Inside);
            } else if selected {
                cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(1.5, tokens.accent), StrokeKind::Inside);
            }
        }

        if !fmt.text.is_empty() {
            let col = if fmt.is_ambiguous {
                tokens.role_cat
            } else if fmt.is_significant {
                tokens.accent
            } else {
                tokens.text_primary
            };
            let font = super::fonts::aligned_numeral_font_id(ui.ctx(), tokens);
            let elided = paint_table_cell_text(ui, cell_rect, &fmt.text, col, font, egui::Align::RIGHT, layout.visible_clip);
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

#[allow(clippy::too_many_arguments)]
fn render_result_tab_table(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    active_tab_idx: usize,
    is_survival_summary: bool,
    viewport: Rect,
    state: &mut ResultsViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ResultTabRenderCtx { rs, tab, tokens, .. } = *ctx;
    let prev_range = state.selected_range(&rs.id, active_tab_idx);
    let ptr_down = ui.input(|i| i.pointer.primary_down());
    let ptr_released = ui.input(|i| i.pointer.primary_released());
    let ptr = ResultPointerInput {
        ptr_down,
        ptr_pressed: ui.input(|i| i.pointer.primary_pressed()),
        ptr_pos: ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.latest_pos())),
        mod_shift_now: ui.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|ev| match ev {
                    egui::Event::PointerButton { modifiers, .. } => Some(modifiers.shift),
                    _ => None,
                })
                .unwrap_or(i.modifiers.shift)
        }),
    };
    let num_cols = tab.column_headers.len().max(tab.rows.iter().map(Vec::len).max().unwrap_or(0));
    let row_stub_w = 190.0_f32;
    let col_w = 128.0_f32;
    let row_h = 24.0_f32;
    let vis_x_min = (viewport.min.x - row_stub_w).max(0.0);
    let vis_x_max = (viewport.max.x - row_stub_w).max(0.0);
    let col_start = ((vis_x_min / col_w).floor().max(0.0) as usize).saturating_sub(1).min(num_cols);
    let col_end = (((vis_x_max / col_w).ceil().max(0.0) as usize) + 1).min(num_cols).max(col_start);
    let layout = ResultTableLayout {
        col_start,
        col_end,
        row_stub_w,
        col_w,
        row_h,
        total_w: (row_stub_w + (num_cols as f32) * col_w).max(ui.available_width()),
        visible_clip: ui.clip_rect().intersect(meta.viewport_rect),
    };

    let spacing_y = ui.spacing().item_spacing.y;
    let row_stride = row_h + spacing_y;

    render_result_table_header(ui, tab, tokens, layout);

    let is_section_at = |r_idx: usize| -> bool {
        let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
        !row_lbl.is_empty() && tab.rows[r_idx].iter().all(|c| matches!(c, ResultCell::Empty))
    };

    let body_top = (viewport.min.y - row_stride).max(0.0);
    let body_bot = (viewport.max.y - row_stride).max(0.0);
    if state.collapsed_sections.is_empty() {
        let total_rows = tab.rows.len();
        let start_r = ((body_top / row_stride).floor().max(0.0) as usize).saturating_sub(1).min(total_rows);
        let end_r = (((body_bot / row_stride).ceil().max(0.0) as usize) + 1).min(total_rows).max(start_r);
        if start_r > 0 {
            let spacer_h = ((start_r as f32) * row_stride - spacing_y).max(0.0);
            ui.allocate_space(vec2(layout.total_w, spacer_h));
        }
        for r_idx in start_r..end_r {
            let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
            let row = &tab.rows[r_idx];
            if is_section_at(r_idx) {
                let _ = render_result_section_row(ui, ctx, r_idx, row_lbl, state, layout, meta);
            } else {
                render_result_data_row(
                    ui,
                    ctx,
                    ResultDataRowInput { active_tab_idx, r_idx, row_lbl, row, is_survival_summary, ptr, layout },
                    state,
                    meta,
                );
            }
        }
        let tail = total_rows.saturating_sub(end_r);
        if tail > 0 {
            let spacer_h = ((tail as f32) * row_stride - spacing_y).max(0.0);
            ui.allocate_space(vec2(layout.total_w, spacer_h));
        }
    } else {
        let mut visible_rows: Vec<(usize, bool)> = Vec::with_capacity(tab.rows.len());
        let mut current_section: Option<String> = None;
        for r_idx in 0..tab.rows.len() {
            if is_section_at(r_idx) {
                let sec_key = format!("{}:{}:{r_idx}", rs.id.as_str(), tab.id);
                current_section = Some(sec_key);
                visible_rows.push((r_idx, true));
            } else if !current_section.as_ref().is_some_and(|sec_key| state.collapsed_sections.contains(sec_key)) {
                visible_rows.push((r_idx, false));
            }
        }
        let total_vis = visible_rows.len();
        let start_d = ((body_top / row_stride).floor().max(0.0) as usize).saturating_sub(1).min(total_vis);
        let end_d = (((body_bot / row_stride).ceil().max(0.0) as usize) + 1).min(total_vis).max(start_d);
        if start_d > 0 {
            let spacer_h = ((start_d as f32) * row_stride - spacing_y).max(0.0);
            ui.allocate_space(vec2(layout.total_w, spacer_h));
        }
        for &(r_idx, is_sec) in &visible_rows[start_d..end_d] {
            let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
            if is_sec {
                let _ = render_result_section_row(ui, ctx, r_idx, row_lbl, state, layout, meta);
            } else {
                let row = &tab.rows[r_idx];
                render_result_data_row(
                    ui,
                    ctx,
                    ResultDataRowInput { active_tab_idx, r_idx, row_lbl, row, is_survival_summary, ptr, layout },
                    state,
                    meta,
                );
            }
        }
        let tail = total_vis.saturating_sub(end_d);
        if tail > 0 {
            let spacer_h = ((tail as f32) * row_stride - spacing_y).max(0.0);
            ui.allocate_space(vec2(layout.total_w, spacer_h));
        }
    }

    if ptr_released || !ptr_down {
        state.dragging_selection = false;
    }

    let next_range = state.selected_range(&rs.id, active_tab_idx);
    if (next_range != prev_range || state.selection_sync_needed)
        && let (Some((sid_a, t_a, ra, ca)), Some((sid_f, t_f, rf, cf))) = (&state.selection_anchor, &state.selected_cell)
        && sid_a == &rs.id
        && *t_a == active_tab_idx
        && sid_f == &rs.id
        && *t_f == active_tab_idx
    {
        state.selection_sync_needed = false;
        intents.push(UiIntent::SyncResultSelection { sheet_id: rs.id.clone(), tab_index: active_tab_idx, anchor: (*ra, *ca), focus: (*rf, *cf) });
    }

    render_result_context_menu(ui, ctx, active_tab_idx, state, meta, intents);
}

fn render_result_context_menu(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    active_tab_idx: usize,
    state: &mut ResultsViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ResultTabRenderCtx { rs, tokens, enabled, .. } = *ctx;
    if !enabled {
        state.context_menu = None;
        return;
    }
    let Some((ref menu_sheet, menu_tab, anchor)) = state.context_menu.clone() else {
        return;
    };
    if menu_sheet != &rs.id || menu_tab != active_tab_idx {
        state.context_menu = None;
        return;
    }
    let screen = ui.ctx().content_rect();
    let pos = pos2(
        anchor.x.clamp(screen.left() + 4.0, (screen.right() - 176.0).max(screen.left() + 4.0)),
        anchor.y.clamp(screen.top() + 4.0, (screen.bottom() - 88.0).max(screen.top() + 4.0)),
    );
    let mut close_menu = false;
    let area_resp =
        egui::Area::new(Id::unique(("result_ctx_menu", rs.id.as_str()))).order(egui::Order::Foreground).fixed_pos(pos).show(ui.ctx(), |menu_ui| {
            egui::Frame::popup(menu_ui.style())
                .fill(tokens.surface_bg)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(4))
                .show(menu_ui, |ui| {
                    ui.set_min_width(156.0);
                    let copy_sel_lbl = tr(tokens.locale, "r13.result.copy_selection");
                    let btn_sel = ui.add_sized([152.0, 26.0], egui::Button::new(copy_sel_lbl).fill(Color32::TRANSPARENT));
                    meta.record_response(
                        ui,
                        &btn_sel,
                        FrameInspectEntryInput {
                            widget_id: "result.ctx.copy_selection",
                            role: "menuitem",
                            label: copy_sel_lbl,
                            command_id: None,
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::ContextMenu,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if btn_sel.clicked() {
                        intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: false });
                        close_menu = true;
                    }
                    let copy_all_lbl = tr(tokens.locale, "r13.result.copy_table");
                    let btn_all = ui.add_sized([152.0, 26.0], egui::Button::new(copy_all_lbl).fill(Color32::TRANSPARENT));
                    meta.record_response(
                        ui,
                        &btn_all,
                        FrameInspectEntryInput {
                            widget_id: "result.ctx.copy_table",
                            role: "menuitem",
                            label: copy_all_lbl,
                            command_id: Some("results.copy_table"),
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::ContextMenu,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if btn_all.clicked() {
                        intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: true });
                        close_menu = true;
                    }
                });
        });
    let clicked_outside = ui.input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !area_resp.response.rect.contains(p)));
    if close_menu || clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
        state.context_menu = None;
    }
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

struct ResultTabChipSpec<'a> {
    sheet_id: &'a str,
    idx: usize,
    tab_id: &'a str,
    tab_title: &'a str,
    vis: Rect,
    selected: bool,
    enabled: bool,
    tokens: &'a Tokens,
    tab_font: &'a FontId,
}

fn paint_result_tab_chip(ui: &mut egui::Ui, spec: &ResultTabChipSpec<'_>, meta: &mut FrameInspectMeta) -> bool {
    let tokens = spec.tokens;
    let vis = spec.vis;
    let resp = ui.interact(
        vis,
        ui.make_persistent_id(("egui_shell.result_tab", spec.sheet_id, spec.idx)),
        if spec.enabled { Sense::click() } else { Sense::hover() },
    );
    let bg = if !spec.enabled {
        tokens.panel_bg
    } else if spec.selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.surface_bg
    };
    let fg = if !spec.enabled {
        tokens.text_disabled
    } else if spec.selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let border = if !spec.enabled {
        tokens.divider
    } else if spec.selected {
        tokens.accent
    } else {
        tokens.border
    };
    let painter = ui.painter().with_clip_rect(vis);
    painter.rect_filled(vis, CornerRadius::same(tokens.metrics.radius_md), bg);
    painter.rect_stroke(vis, CornerRadius::same(tokens.metrics.radius_md), Stroke::new(tokens.metrics.stroke_thin_pt, border), StrokeKind::Inside);

    let pad_x = tokens.metrics.space_sm;
    let avail_text_w = (vis.width() - pad_x * 2.0).max(4.0);
    let mut job = egui::text::LayoutJob::single_section(
        spec.tab_title.to_owned(),
        egui::text::TextFormat { font_id: spec.tab_font.clone(), color: fg, ..Default::default() },
    );
    job.wrap = egui::text::TextWrapping { max_width: avail_text_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
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

    let enabled = spec.enabled;
    let selected = spec.selected;
    let ak_title = spec.tab_title.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, enabled, selected, ak_title.clone()));
    let resp = if !enabled {
        resp.on_hover_text("modal dialog is active")
    } else if ellipsized {
        resp.on_hover_text(spec.tab_title)
    } else {
        resp
    };

    let wid = format!("result.tab.{}", spec.tab_id);
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "tab",
            label: spec.tab_title,
            command_id: None,
            value: Some(json!({ "selected": selected, "index": spec.idx, "tab_id": spec.tab_id })),
            enabled,
            disabled_reason: (!enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(spec.tab_title),
        },
    );
    resp.clicked() && enabled && !selected
}
