//! Native `egui` multi-tab reactive `ResultSheet` viewer (`docs/planning/gui-layout-spec.md` §8.10, `R15.3`).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{AnalysisKind, CiBound, PValueFormat, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId};
use curvebook_palette::lerp_oklab;
use egui::{Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{AssumptionDiagnosticBanner, ResultSheetViewState};
use crate::egui_shell::ui_kit::{
    ActionMenuAnchor, ActionMenuEntry, ActionMenuItem, ActionMenuSpec, TooltipSpec, action_menu_popup, ghost_action_button, paint_dashed_rect_stroke,
    show_tooltip, show_tooltip_for_control, underline_tab_button,
};
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, color32_to_rgba8, rgba8_to_color32};
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

/// Cached null-model comparison readout for `R20.6`'s `Compare vs. Horizontal Line` banner action.
#[derive(Debug, Clone, PartialEq)]
pub struct HorizontalLineReadout {
    pub sheet_id: SheetId,
    pub mutation_seq: u64,
    pub title: String,
    pub overall_summary: String,
    pub detail_lines: Vec<String>,
    pub error_message: Option<String>,
}

/// Cached CI-first per-column geometry for [`render_result_tab_table`].
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct CachedResultTableLayout {
    pub sheet_id: SheetId,
    pub tab_idx: usize,
    pub mutation_seq: u64,
    pub row_count: usize,
    pub col_count: usize,
    pub avail_w_q: i32,
    pub vis_bucket: usize,
    pub font_size_q: i32,
    pub p_fmt: PValueFormat,
    pub sig_figs: Option<u8>,
    pub locale: String,
    pub row_stub_w: f32,
    pub col_widths: Vec<f32>,
    pub col_x_offsets: Vec<f32>,
    pub total_w: f32,
    pub natural_table_width_pt: f32,
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
    /// Latest natural table content width (`row_stub + sum(natural_col_widths)`) for `R20.13` split auto-sizing.
    pub last_natural_table_width_pt: f32,
    /// Per-(sheet, tab) natural table content widths in logical points.
    pub natural_table_widths: BTreeMap<(SheetId, usize), f32>,
    /// Sheet whose `Compare vs. Horizontal Line` null-model comparison readout popover is open.
    pub compare_horizontal_open_sheet: Option<SheetId>,
    /// Cached null-model comparison summary & per-dataset diagnostics.
    pub compare_horizontal_cached: Option<HorizontalLineReadout>,
    /// Sheet whose standalone nonlinear banner `Why?` popover is open.
    pub standalone_banner_why_open: Option<SheetId>,
    /// Sheets dismissed while running standalone `render_results_view_for_sheet`.
    pub standalone_dismissed_banners: BTreeSet<SheetId>,
    /// Deferred horizontal match scroll target until column x-offsets are resolved.
    #[doc(hidden)]
    pub pending_scroll_to_match_col: Option<(SheetId, usize, usize)>,
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

    /// Cached natural table width (`row_stub + sum(natural_col_widths)`) in logical points when available.
    #[must_use]
    pub fn natural_table_width_for_sheet(&self, sheet_id: &SheetId, tab_idx: usize) -> Option<f32> {
        self.natural_table_widths.get(&(sheet_id.clone(), tab_idx)).copied()
    }

    /// Request scrolling the results table to `(tab_idx, row_idx, col_idx)` and selecting that cell.
    pub fn scroll_to_match(&mut self, sheet_id: SheetId, tab_idx: usize, row_idx: usize, col_idx: usize) {
        self.per_sheet_active_tab.insert(sheet_id.clone(), tab_idx);
        self.selection_anchor = Some((sheet_id.clone(), tab_idx, row_idx, col_idx));
        self.selected_cell = Some((sheet_id.clone(), tab_idx, row_idx, col_idx));
        self.selection_sync_needed = true;
        self.pending_scroll_to_match = Some((sheet_id.clone(), tab_idx, row_idx, col_idx));
        self.pending_scroll_to_match_col = Some((sheet_id, tab_idx, col_idx));
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
    pub natural_table_width_pt: f32,
}

#[derive(Debug, Clone)]
struct FormattedCellDisplay {
    text: String,
    is_ambiguous: bool,
    is_significant: bool,
    is_muted: bool,
}

fn contains_ascii_case_insensitive(haystack: &str, needle_lower: &[u8]) -> bool {
    !needle_lower.is_empty() && haystack.as_bytes().windows(needle_lower.len()).any(|w| w.eq_ignore_ascii_case(needle_lower))
}

fn result_cell_is_ambiguous(cell: &ResultCell, is_survival_summary: bool) -> bool {
    match cell {
        ResultCell::Empty | ResultCell::PValue { .. } | ResultCell::Boolean(_) => false,
        ResultCell::Number { value, prefix, .. } => value.is_finite() && prefix.as_deref() == Some("~"),
        ResultCell::ConfidenceInterval { lower, upper } => !is_survival_summary && (matches!(lower, CiBound::Open) || matches!(upper, CiBound::Open)),
        ResultCell::Text(s) => {
            s.contains("???")
                || contains_ascii_case_insensitive(s, b"ambiguous")
                || contains_ascii_case_insensitive(s, b"unstable")
                || contains_ascii_case_insensitive(s, b"very wide")
        }
    }
}

fn format_result_cell_semantic(cell: &ResultCell, p_format: PValueFormat, is_survival_summary: bool, sig_figs: Option<u8>) -> FormattedCellDisplay {
    format_result_cell_semantic_localized(cell, p_format, is_survival_summary, sig_figs, "en")
}

fn format_result_cell_semantic_localized(
    cell: &ResultCell,
    p_format: PValueFormat,
    is_survival_summary: bool,
    sig_figs: Option<u8>,
    locale: &str,
) -> FormattedCellDisplay {
    match cell {
        ResultCell::Empty => FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false, is_muted: false },
        ResultCell::Number { value, prefix, units } => {
            if !value.is_finite() {
                // Unreached medians or undefined scalars must remain explicitly blank, never "NaN" or "0".
                return FormattedCellDisplay { text: String::new(), is_ambiguous: false, is_significant: false, is_muted: false };
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
            FormattedCellDisplay { text: base, is_ambiguous: is_amb, is_significant: false, is_muted: false }
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            if matches!(lower, CiBound::Unavailable) && matches!(upper, CiBound::Unavailable) {
                FormattedCellDisplay {
                    text: tr_or_fallback(locale, "results.ci.unavailable", "Unavailable").to_owned(),
                    is_ambiguous: false,
                    is_significant: false,
                    is_muted: true,
                }
            } else if matches!(lower, CiBound::Unavailable) || matches!(upper, CiBound::Unavailable) {
                let lo = format_ci_bound_semantic_sig(*lower, is_survival_summary, sig_figs, locale);
                let hi = format_ci_bound_semantic_sig(*upper, is_survival_summary, sig_figs, locale);
                FormattedCellDisplay { text: format!("[{lo}, {hi}]"), is_ambiguous: false, is_significant: false, is_muted: true }
            } else if matches!(lower, CiBound::Open) && matches!(upper, CiBound::Open) {
                if is_survival_summary {
                    FormattedCellDisplay {
                        text: tr_or_fallback(locale, "results.ci.not_reached", "Not reached").to_owned(),
                        is_ambiguous: false,
                        is_significant: false,
                        is_muted: true,
                    }
                } else {
                    FormattedCellDisplay {
                        text: tr_or_fallback(locale, "results.ci.not_bounded", "Not bounded").to_owned(),
                        is_ambiguous: true,
                        is_significant: false,
                        is_muted: false,
                    }
                }
            } else if matches!(lower, CiBound::Open) || matches!(upper, CiBound::Open) {
                let lo = format_ci_bound_semantic_sig(*lower, is_survival_summary, sig_figs, locale);
                let hi = format_ci_bound_semantic_sig(*upper, is_survival_summary, sig_figs, locale);
                FormattedCellDisplay { text: format!("[{lo}, {hi}]"), is_ambiguous: !is_survival_summary, is_significant: false, is_muted: false }
            } else if let Some(d) = sig_figs {
                let lo = format_ci_bound_semantic_sig(*lower, is_survival_summary, Some(d), locale);
                let hi = format_ci_bound_semantic_sig(*upper, is_survival_summary, Some(d), locale);
                FormattedCellDisplay { text: format!("[{lo}, {hi}]"), is_ambiguous: false, is_significant: false, is_muted: false }
            } else {
                FormattedCellDisplay {
                    text: ResultSheetViewState::format_result_cell_with_survival(cell, p_format, is_survival_summary),
                    is_ambiguous: false,
                    is_significant: false,
                    is_muted: false,
                }
            }
        }
        ResultCell::PValue { stars, .. } => {
            let text = ResultSheetViewState::format_result_cell(cell, p_format);
            let is_sig = !stars.is_empty() && stars != "ns";
            FormattedCellDisplay { text, is_ambiguous: false, is_significant: is_sig, is_muted: false }
        }
        ResultCell::Text(s) => {
            let trimmed = s.trim();
            if trimmed.contains("???") || trimmed.eq_ignore_ascii_case("Unstable") {
                FormattedCellDisplay {
                    text: tr_or_fallback(locale, "fit.ident.unstable", "Unstable").to_owned(),
                    is_ambiguous: true,
                    is_significant: false,
                    is_muted: false,
                }
            } else if trimmed.eq_ignore_ascii_case("Very wide") {
                FormattedCellDisplay {
                    text: tr_or_fallback(locale, "results.ci.very_wide", "Very wide").to_owned(),
                    is_ambiguous: true,
                    is_significant: false,
                    is_muted: false,
                }
            } else {
                let is_amb = contains_ascii_case_insensitive(trimmed, b"ambiguous")
                    || contains_ascii_case_insensitive(trimmed, b"unstable")
                    || contains_ascii_case_insensitive(trimmed, b"very wide");
                FormattedCellDisplay { text: s.clone(), is_ambiguous: is_amb, is_significant: false, is_muted: false }
            }
        }
        ResultCell::Boolean(b) => {
            FormattedCellDisplay { text: String::from(if *b { "Yes" } else { "No" }), is_ambiguous: false, is_significant: false, is_muted: false }
        }
    }
}

fn format_ci_bound_semantic_sig(bound: CiBound, is_survival_summary: bool, sig_figs: Option<u8>, locale: &str) -> String {
    match bound {
        CiBound::Finite(v) if v.is_finite() => match sig_figs {
            Some(d) => curvebook_doc::format_significant_digits(v, d),
            None => format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_owned(),
        },
        CiBound::Finite(_) | CiBound::Open => {
            if is_survival_summary {
                tr_or_fallback(locale, "results.ci.not_reached", "Not reached").to_owned()
            } else {
                tr_or_fallback(locale, "results.ci.unbounded", "unbounded").to_owned()
            }
        }
        CiBound::Unavailable => tr_or_fallback(locale, "results.ci.unavailable", "Unavailable").to_owned(),
    }
}

/// Format `cell` identically to how [`render_results_view_for_sheet`] displays it on screen.
#[must_use]
pub fn format_result_cell_display_text(cell: &ResultCell, rs: &ResultSheet, tab: &ResultTab) -> String {
    let is_survival_summary = matches!(rs.analysis.kind, AnalysisKind::Survival(_)) && matches!(tab.kind, ResultTabKind::MainSummary);
    let sig_figs = rs.analysis.kind.output_significant_digits();
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
    }
    intents.push(UiIntent::SelectResultTab { index: new_idx, sheet_id: Some(sheet_id.clone()) });
}

/// Render the active [`curvebook_doc::ResultSheet`] with borderless underline tabs, quiet header actions,
/// consolidated diagnostic banner, and CI-first auto-sized table body.
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
    render_results_view_for_sheet_with_options(ui, app, sheet_id, state, tokens, enabled, true, false, meta, intents)
}

/// Render a specific [`curvebook_doc::ResultSheet`] with explicit `pane_focused` and CenterStage banner coordination.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_results_view_for_sheet_with_options(
    ui: &mut egui::Ui,
    app: &UiApp,
    sheet_id: Option<&SheetId>,
    state: &mut ResultsViewEguiState,
    tokens: &Tokens,
    enabled: bool,
    pane_focused: bool,
    suppress_nl_banner: bool,
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

    // Ctrl+PageUp / Ctrl+PageDown keyboard switching across sub-tabs (only when pane is focused).
    if pane_focused && enabled && tab_count > 1 && !ui.ctx().egui_wants_keyboard_input() {
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

    let hdr_h = 32.0_f32.min(avail.height());
    let hdr_rect = Rect::from_min_size(avail.min, vec2(avail.width(), hdr_h));
    report.header_rect_pt = Some(hdr_rect);

    ui.painter().rect_filled(hdr_rect, CornerRadius::ZERO, tokens.panel_bg);
    ui.painter()
        .line_segment([pos2(hdr_rect.left(), hdr_rect.bottom()), pos2(hdr_rect.right(), hdr_rect.bottom())], Stroke::new(1.0, tokens.divider));

    let pad_h = tokens.metrics.space_sm;
    let hdr_inner = Rect::from_min_max(pos2(hdr_rect.left() + pad_h, hdr_rect.top()), pos2(hdr_rect.right() - pad_h, hdr_rect.bottom()));

    let edit_lbl = tr_or_fallback(&app.locale, "results.parameters", "Parameters");
    let edit_shortcut = crate::menus::format_shortcut_for_platform(app.shortcuts.active_combo("ui.editParams.open").unwrap_or("Cmd+T"));
    let edit_summary = crate::menus::command_tooltip(app, "ui.editParams.open").map(|t| t.summary).unwrap_or("");
    let copy_lbl = tr_or_fallback(&app.locale, "results.methods_copy", "Copy");
    let copy_tip = tr_or_fallback(&app.locale, "results.copy_selection_tooltip", "Copy selected cells (TSV)");

    let small_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
    let measure_action_w = |ctx: &egui::Context, text: &str| -> f32 {
        let g = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), small_font.clone(), tokens.text_primary));
        (g.size().x + tokens.metrics.space_sm * 2.0 + 4.0).max(tokens.metrics.min_hit_target_pt)
    };
    let edit_w = measure_action_w(ui.ctx(), edit_lbl);
    let copy_w = measure_action_w(ui.ctx(), copy_lbl);
    let diag_graph_btn: Option<(&'static str, &str, curvebook_doc::DiagnosticPlotKind)> = if matches!(rs.status, ResultStatus::UpToDate) {
        match &rs.analysis.kind {
            AnalysisKind::Multivariate(curvebook_doc::MultivariateSpec::Pca { .. }) => Some((
                "result.create_pca_biplot",
                tr_or_fallback(&app.locale, "results.create_pca_biplot", "PCA Biplot"),
                curvebook_doc::DiagnosticPlotKind::PcaBiplot,
            )),
            AnalysisKind::Multivariate(curvebook_doc::MultivariateSpec::Clustering { .. }) => Some((
                "result.create_dendrogram",
                tr_or_fallback(&app.locale, "results.create_dendrogram", "Dendrogram"),
                curvebook_doc::DiagnosticPlotKind::Dendrogram,
            )),
            _ => None,
        }
    } else {
        None
    };
    let gap = tokens.metrics.space_xs;
    let diag_w = diag_graph_btn.map_or(0.0, |(_, lbl, _)| measure_action_w(ui.ctx(), lbl) + gap);
    let divider_pad = 10.0_f32;
    let right_group_w = (edit_w + gap + copy_w + diag_w).min((hdr_inner.width() * 0.62).max(120.0));
    let right_rect = Rect::from_min_max(pos2((hdr_inner.right() - right_group_w).max(hdr_inner.left()), hdr_inner.top()), hdr_inner.max);
    let divider_x = (right_rect.left() - divider_pad * 0.5).max(hdr_inner.left());
    let tabs_area_rect = Rect::from_min_max(hdr_inner.min, pos2((divider_x - divider_pad * 0.5).max(hdr_inner.left()), hdr_inner.bottom()));

    if right_rect.left() > hdr_inner.left() + 8.0 {
        let div_top = hdr_rect.top() + 5.0;
        let div_bot = hdr_rect.bottom() - 5.0;
        if div_bot > div_top {
            ui.painter().line_segment([pos2(divider_x, div_top), pos2(divider_x, div_bot)], Stroke::new(1.0, tokens.divider));
        }
    }

    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        ui.set_clip_rect(hdr_rect);
        ui.spacing_mut().item_spacing.x = gap;
        let has_tabs = !rs.tabs.is_empty();
        let (mut copy_resp, copy_clicked) =
            ghost_action_button(ui, tokens, meta, "result.copy_selection", copy_lbl, None, false, enabled && has_tabs, ZLayer::Workspace);
        let copy_spec = TooltipSpec {
            title: copy_lbl,
            shortcut: None,
            summary: Some(copy_tip),
            disabled_reason: (!enabled).then_some("modal dialog is active").or_else(|| (!has_tabs).then_some("No result table is available to copy")),
        };
        copy_resp = show_tooltip(&copy_resp, tokens, copy_spec);
        let _ = copy_resp;
        if copy_clicked {
            intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: false });
        }

        let edit_enabled = enabled && !rs.is_imported();
        let (mut edit_resp, edit_clicked) =
            ghost_action_button(ui, tokens, meta, "result.edit_params", edit_lbl, Some("ui.editParams.open"), false, edit_enabled, ZLayer::Workspace);
        let edit_disabled_reason = if !enabled {
            Some("modal dialog is active")
        } else if rs.is_imported() {
            Some("imported result snapshots are read-only")
        } else {
            None
        };
        edit_resp = show_tooltip(
            &edit_resp,
            tokens,
            TooltipSpec {
                title: edit_lbl,
                shortcut: (!edit_shortcut.is_empty()).then_some(edit_shortcut.as_str()),
                summary: (!edit_summary.is_empty()).then_some(edit_summary),
                disabled_reason: edit_disabled_reason,
            },
        );
        let _ = edit_resp;
        if edit_clicked && edit_enabled {
            intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
        }
        if let Some((wid, lbl, diag_kind)) = diag_graph_btn {
            let (diag_resp, diag_clicked) = ghost_action_button(ui, tokens, meta, wid, lbl, Some("graph.create"), false, enabled, ZLayer::Workspace);
            let _ = show_tooltip(
                &diag_resp,
                tokens,
                TooltipSpec { title: lbl, shortcut: None, summary: Some(lbl), disabled_reason: (!enabled).then_some("modal dialog is active") },
            );
            if diag_clicked && enabled {
                intents.push(UiIntent::EngineCommand {
                    command_id: String::from("graph.create"),
                    params: json!({
                        "data_sheet_id": rs.id.as_str(),
                        "kind": curvebook_doc::GraphKind::Diagnostic(diag_kind),
                        "initialized": true,
                    }),
                });
            }
        }
    });

    ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_area_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.set_clip_rect(hdr_rect);
        ui.spacing_mut().item_spacing.x = gap;
        let avail_strip_w = tabs_area_rect.width().max(0.0);
        let bold_tab_font = if super::fonts::has_bold_font(ui.ctx()) {
            FontId::new(tokens.metrics.font_ui_small_pt, egui::FontFamily::Name(super::fonts::BOLD_FONT_FAMILY.into()))
        } else {
            small_font.clone()
        };
        let tab_pad_x = tokens.metrics.space_sm.max(8.0);
        let active_cap = (avail_strip_w * 0.55).max(180.0);
        let inactive_cap = 200.0_f32;
        let min_tab_w = tokens.metrics.min_hit_target_pt.max(48.0);

        let mut natural_widths = Vec::with_capacity(tab_count);
        for (idx, tab) in rs.tabs.iter().enumerate() {
            let f = if idx == active_idx { bold_tab_font.clone() } else { small_font.clone() };
            let g = ui.fonts_mut(|fonts| fonts.layout_no_wrap(tab.title.clone(), f, tokens.text_primary));
            let cap = if idx == active_idx { active_cap } else { inactive_cap };
            let w = (g.size().x + tab_pad_x * 2.0 + 1.0).clamp(min_tab_w.min(avail_strip_w.max(min_tab_w)), cap);
            natural_widths.push(w);
        }

        let all_sum: f32 = natural_widths.iter().sum::<f32>() + gap * (tab_count.saturating_sub(1) as f32);
        let mut visible_indices: Vec<usize> = Vec::with_capacity(tab_count);
        let mut hidden_tabs: Vec<(usize, String, String)> = Vec::new();
        let overflow_btn_w = 48.0_f32;
        let needs_overflow = tab_count > 1 && all_sum > avail_strip_w + 0.5;

        if !needs_overflow {
            visible_indices.extend(0..tab_count);
        } else {
            let strip_budget = (avail_strip_w - overflow_btn_w - gap).max(0.0);
            let active_w = natural_widths.get(active_idx).copied().unwrap_or(min_tab_w).min(strip_budget.max(min_tab_w));
            let mut rem = (strip_budget - active_w).max(0.0);
            let mut keep = vec![false; tab_count];
            if active_idx < tab_count {
                keep[active_idx] = true;
            }
            for (idx, &w) in natural_widths.iter().enumerate() {
                if idx == active_idx {
                    continue;
                }
                let cost = w + gap;
                if cost <= rem + 0.5 {
                    keep[idx] = true;
                    rem = (rem - cost).max(0.0);
                }
            }
            for (idx, tab) in rs.tabs.iter().enumerate() {
                if keep[idx] {
                    visible_indices.push(idx);
                } else {
                    hidden_tabs.push((idx, tab.id.clone(), tab.title.clone()));
                }
            }
        }

        let tab_h = hdr_rect.height();
        for idx in visible_indices {
            let tab = &rs.tabs[idx];
            let selected = idx == active_idx;
            let w = if selected && needs_overflow {
                natural_widths[idx].min((avail_strip_w - overflow_btn_w - gap).max(min_tab_w))
            } else {
                natural_widths[idx]
            };
            let wid = format!("result.tab.{}", tab.id);
            let (_, clicked) = underline_tab_button(
                ui,
                tokens,
                meta,
                &wid,
                &tab.title,
                w,
                tab_h,
                selected,
                enabled,
                Some(json!({ "selected": selected, "index": idx, "tab_id": tab.id })),
            );
            if clicked && !selected {
                active_idx = idx;
                select_result_tab_for_sheet(state, &rs.id, idx, is_primary_sheet, intents);
            }
        }

        if needs_overflow && !hidden_tabs.is_empty() {
            let badge_lbl = format!("⋯ +{}", hidden_tabs.len());
            let ov_tip = format!("{} (+{})", tr(&app.locale, "toolbar.overflow"), hidden_tabs.len());
            let (mut ov_resp, ov_clicked) =
                ghost_action_button(ui, tokens, meta, "result.tabs.overflow", &badge_lbl, None, state.tab_overflow_open, enabled, ZLayer::Workspace);
            ov_resp = show_tooltip(
                &ov_resp,
                tokens,
                TooltipSpec {
                    title: tr(&app.locale, "toolbar.overflow"),
                    shortcut: None,
                    summary: Some(&ov_tip),
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                },
            );
            let anchor_rect = ov_resp.rect;
            if ov_clicked {
                state.tab_overflow_open = !state.tab_overflow_open;
            }
            if !enabled {
                state.tab_overflow_open = false;
            } else if state.tab_overflow_open
                && let Some(picked_idx) =
                    render_result_tabs_overflow_popup(ui.ctx(), rs.id.as_str(), tokens, state, &hidden_tabs, active_idx, anchor_rect, meta)
            {
                active_idx = picked_idx;
                select_result_tab_for_sheet(state, &rs.id, picked_idx, is_primary_sheet, intents);
            }
        } else {
            state.tab_overflow_open = false;
        }
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

        if rs.is_imported() {
            render_imported_result_banner(ui, rs, tokens, enabled, meta, intents);
        }

        let Some(tab) = rs.tabs.get(active_idx) else {
            render_empty_or_failed_result_body(ui, rs, tokens, enabled, meta, intents);
            return;
        };

        let category = ResultTabCategory::for_kind(tab.kind);
        report.active_tab_category = Some(category);

        // Sub-header strip (PlotReadyMatrix only) & consolidated warning banner
        let tab_ctx = ResultTabRenderCtx { rs, tab, tokens, enabled };
        render_tab_category_bar(ui, &tab_ctx, category, &app.locale, meta, intents);

        let is_survival_summary = matches!(rs.analysis.kind, AnalysisKind::Survival(_)) && matches!(tab.kind, ResultTabKind::MainSummary);
        let is_nonlinear = matches!(rs.analysis.kind, AnalysisKind::NonlinearRegression(_));
        if is_nonlinear {
            report.has_ambiguity_callout = false;
            if !suppress_nl_banner
                && !state.standalone_dismissed_banners.contains(&rs.id)
                && !app.egui_views.center_stage.dismissed_banner_sheets.contains(&rs.id)
                && let Some(banner) = AssumptionDiagnosticBanner::from_result_localized(rs, &app.locale)
            {
                render_standalone_nl_diagnostic_banner(ui, app, rs, &banner, tokens, state, enabled, meta, intents);
            }
        } else {
            let tab_has_ambig_cell = tab_contains_ambiguity(tab, rs.analysis.p_value_format, is_survival_summary);
            let curve_ambiguous = matches!(tab.kind, ResultTabKind::MainSummary | ResultTabKind::ParametersAndConfidenceIntervals)
                && rs.fitted_curves.iter().any(|c| c.ambiguous);
            let has_ambiguity = tab_has_ambig_cell || curve_ambiguous;
            report.has_ambiguity_callout = has_ambiguity;

            if has_ambiguity {
                render_ambiguity_callout(ui, rs, &app.locale, tokens, enabled, meta, intents);
            }
        }

        let (scroll_to_y, scroll_to_x) = if state.pending_scroll_to_match.as_ref().is_some_and(|(sid, t, _, _)| sid == &rs.id && *t == active_idx) {
            state.pending_scroll_to_match.take().map_or((None, None), |(_, _, target_row, target_col)| {
                expand_section_containing_row(&rs.id, tab, target_row, &mut state.collapsed_sections);
                let vis_row = visible_row_index_for_raw_row(&rs.id, tab, target_row, &state.collapsed_sections);
                let row_stride = 24.0_f32 + ui.spacing().item_spacing.y;
                #[allow(clippy::cast_precision_loss)]
                let y = ((vis_row as f32) * row_stride - row_stride * 2.0).max(0.0);
                let cached_layout: Option<CachedResultTableLayout> =
                    ui.ctx().data(|d| d.get_temp(Id::unique(("cb_results_col_layout", rs.id.as_str(), active_idx))));
                let cached_x = cached_layout.as_ref().filter(|c| c.sheet_id == rs.id && c.tab_idx == active_idx).and_then(|c| {
                    if target_col == 0 {
                        Some(0.0)
                    } else {
                        c.col_x_offsets.get(target_col - 1).map(|&x_left| (x_left - c.row_stub_w - 24.0).max(0.0))
                    }
                });
                let x = cached_x.unwrap_or_else(|| if target_col == 0 { 0.0 } else { ((target_col - 1) as f32) * 128.0 });
                if cached_x.is_some() {
                    state.pending_scroll_to_match_col = None;
                }
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
        let high_contrast = app.theme.mode == crate::UiThemeMode::HighContrast;
        let scroll_out = scroll_area.show_viewport(ui, |ui, viewport| {
            let natural_w = render_result_tab_table(
                ui,
                app,
                &tab_ctx,
                active_idx,
                is_survival_summary,
                pane_focused,
                high_contrast,
                viewport,
                state,
                meta,
                intents,
            );
            report.natural_table_width_pt = natural_w;
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
    locale: &str,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if category != ResultTabCategory::PlotReadyMatrix {
        return;
    }
    let ResultTabRenderCtx { rs, tab, tokens, enabled } = *ctx;
    let bar_h = tokens.metrics.min_hit_target_pt.max(28.0);
    let bar_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), bar_h)).1;
    ui.painter().rect_filled(bar_rect, CornerRadius::ZERO, tokens.header_bg);
    ui.painter()
        .line_segment([pos2(bar_rect.left(), bar_rect.bottom()), pos2(bar_rect.right(), bar_rect.bottom())], Stroke::new(1.0, tokens.divider));

    let pad_h = tokens.metrics.space_sm;
    let inner_rect = Rect::from_min_max(pos2(bar_rect.left() + pad_h, bar_rect.top()), pos2(bar_rect.right() - pad_h, bar_rect.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect).layout(egui::Layout::right_to_left(egui::Align::Center)), |ui| {
        ui.set_clip_rect(bar_rect);
        let btn_lbl = tr_or_fallback(locale, "results.new_graph_from_tab", "New Graph from Tab");
        let (_, clicked) = ghost_action_button(
            ui,
            tokens,
            meta,
            "result.new_graph_from_tab",
            btn_lbl,
            Some("sheet.extractResultTab"),
            false,
            enabled,
            ZLayer::Workspace,
        );
        if clicked {
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

#[allow(clippy::too_many_arguments)]
fn render_result_tabs_overflow_popup(
    ctx: &egui::Context,
    sheet_id: &str,
    tokens: &Tokens,
    state: &mut ResultsViewEguiState,
    hidden_tabs: &[(usize, String, String)],
    active_idx: usize,
    anchor: Rect,
    meta: &mut FrameInspectMeta,
) -> Option<usize> {
    let wids: Vec<String> = hidden_tabs.iter().map(|(_, tab_id, _)| format!("result.tab.overflow.{tab_id}")).collect();
    let entries: Vec<ActionMenuEntry<'_>> = hidden_tabs
        .iter()
        .zip(wids.iter())
        .map(|((idx, tab_id, title), wid)| {
            let selected = *idx == active_idx;
            ActionMenuEntry::Item(
                ActionMenuItem::menu_item(wid, title)
                    .selected(selected)
                    .value(Some(json!({ "selected": selected, "index": *idx, "tab_id": tab_id }))),
            )
        })
        .collect();

    let out = action_menu_popup(
        ctx,
        tokens,
        Some(meta),
        ActionMenuSpec::dropdown(
            Id::unique(("egui_shell.result_tabs.overflow", sheet_id)),
            "popup.result.tabs.overflow",
            "More Tabs",
            ActionMenuAnchor::Trigger(anchor),
        )
        .max_height(256.0),
        &entries,
    );

    let mut picked_tab: Option<usize> = None;
    if let Some(hit) = out.clicked_entry
        && let Some((idx, _, _)) = hidden_tabs.get(hit)
    {
        picked_tab = Some(*idx);
        state.tab_overflow_open = false;
    } else if out.dismissed {
        state.tab_overflow_open = false;
    }
    picked_tab
}

pub(super) fn trigger_horizontal_line_comparison(
    doc: &curvebook_doc::ProjectDocument,
    mutation_seq: u64,
    sheet_id: &SheetId,
    state: &mut ResultsViewEguiState,
    locale: &str,
) {
    if state.compare_horizontal_open_sheet.as_ref() == Some(sheet_id) {
        state.compare_horizontal_open_sheet = None;
        return;
    }
    state.compare_horizontal_open_sheet = Some(sheet_id.clone());
    let needs_refresh = state.compare_horizontal_cached.as_ref().is_none_or(|c| &c.sheet_id != sheet_id || c.mutation_seq != mutation_seq);
    if !needs_refresh {
        return;
    }
    let title = tr_or_fallback(locale, "results.banner.compare_horizontal_title", "Null-Model Comparison (Horizontal Line)").to_owned();
    let readout = match curvebook_engine::services::compare_result_vs_horizontal_line(doc, sheet_id) {
        Ok(report) => {
            let mut detail_lines = Vec::new();
            for ds in report.per_dataset.iter().take(6) {
                let tag = match ds.classification {
                    curvebook_fit::HorizontalLineClassification::NotDistinguishableFromHorizontalLine => "Horizontal line wins",
                    curvebook_fit::HorizontalLineClassification::IncompleteOrUnboundedCurve => "Real trend / unbounded",
                    curvebook_fit::HorizontalLineClassification::NotApplicableInRobustMode => "Not applicable (Robust)",
                };
                detail_lines.push(format!("{} — {}: {}", ds.label, tag, ds.summary));
            }
            if report.per_dataset.len() > 6 {
                detail_lines.push(format!("+{} additional dataset(s)", report.per_dataset.len() - 6));
            }
            HorizontalLineReadout {
                sheet_id: sheet_id.clone(),
                mutation_seq,
                title,
                overall_summary: report.overall_summary,
                detail_lines,
                error_message: None,
            }
        }
        Err(err) => HorizontalLineReadout {
            sheet_id: sheet_id.clone(),
            mutation_seq,
            title,
            overall_summary: String::new(),
            detail_lines: Vec::new(),
            error_message: Some(err.to_string()),
        },
    };
    state.compare_horizontal_cached = Some(readout);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_horizontal_line_comparison_popup(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    anchor_rect: Rect,
    above: bool,
    sheet_id: &SheetId,
    state: &mut ResultsViewEguiState,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) {
    if !enabled || state.compare_horizontal_open_sheet.as_ref() != Some(sheet_id) {
        return;
    }
    let Some(readout) = state.compare_horizontal_cached.as_ref().filter(|c| &c.sheet_id == sheet_id).cloned() else {
        return;
    };
    let pop_w = (anchor_rect.width() - 16.0).clamp(240.0, 560.0);
    let pop_x = anchor_rect.left() + 8.0;
    let (pivot, pop_pos) = if above {
        (egui::Align2::LEFT_BOTTOM, pos2(pop_x, (anchor_rect.top() - 4.0).max(48.0)))
    } else {
        (egui::Align2::LEFT_TOP, pos2(pop_x, anchor_rect.bottom() + 2.0))
    };

    let mut close_clicked = false;
    // popup-size-exempt: bounded diagnostic readout card (width clamped 240..560pt, viewport-constrained, non-menu popover)
    let area_resp = egui::Area::new(Id::unique(("results_compare_horizontal", sheet_id.as_str())))
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(pop_pos)
        .constrain_to(ui.ctx().content_rect().shrink(4.0))
        .show(ui.ctx(), |pop_ui| {
            let _frame_resp = egui::Frame::popup(pop_ui.style())
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, tokens.border))
                .show(pop_ui, |inner_ui| {
                    inner_ui.set_width(pop_w);
                    inner_ui.horizontal(|hdr| {
                        let title_resp = hdr.label(RichText::new(&readout.title).small().strong().color(tokens.text_strong));
                        meta.record_response(
                            hdr,
                            &title_resp,
                            FrameInspectEntryInput {
                                widget_id: "result.diagnostic_banner.compare_horizontal.title",
                                role: "heading",
                                label: &readout.title,
                                command_id: None,
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::DropdownMenu,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        hdr.with_layout(egui::Layout::right_to_left(egui::Align::Center), |r_ui| {
                            let (_, clicked) = ghost_action_button(
                                r_ui,
                                tokens,
                                meta,
                                "result.diagnostic_banner.compare_horizontal.close",
                                "×",
                                None,
                                false,
                                true,
                                ZLayer::DropdownMenu,
                            );
                            close_clicked = clicked;
                        });
                    });
                    if let Some(err) = &readout.error_message {
                        let err_resp = inner_ui.colored_label(tokens.warning, RichText::new(err).small());
                        meta.record_response(
                            inner_ui,
                            &err_resp,
                            FrameInspectEntryInput {
                                widget_id: "result.diagnostic_banner.compare_horizontal.error",
                                role: "status",
                                label: err,
                                command_id: None,
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::DropdownMenu,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                    } else {
                        let sum_resp = inner_ui.label(RichText::new(&readout.overall_summary).small().color(tokens.text_primary));
                        meta.record_response(
                            inner_ui,
                            &sum_resp,
                            FrameInspectEntryInput {
                                widget_id: "result.diagnostic_banner.compare_horizontal.summary",
                                role: "status",
                                label: &readout.overall_summary,
                                command_id: None,
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::DropdownMenu,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        for (idx, line) in readout.detail_lines.iter().enumerate() {
                            let line_resp = inner_ui.label(RichText::new(line).small().color(tokens.text_muted));
                            meta.record_response(
                                inner_ui,
                                &line_resp,
                                FrameInspectEntryInput {
                                    widget_id: &format!("result.diagnostic_banner.compare_horizontal.detail.{idx}"),
                                    role: "note",
                                    label: line,
                                    command_id: None,
                                    value: None,
                                    enabled: true,
                                    disabled_reason: None,
                                    z_layer: ZLayer::DropdownMenu,
                                    ellipsized: false,
                                    full_label: None,
                                },
                            );
                        }
                    }
                });
        });
    let _ = area_resp;
    if close_clicked {
        state.compare_horizontal_open_sheet = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn render_standalone_nl_diagnostic_banner(
    ui: &mut egui::Ui,
    app: &UiApp,
    rs: &ResultSheet,
    banner: &AssumptionDiagnosticBanner,
    tokens: &Tokens,
    state: &mut ResultsViewEguiState,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = app.locale.as_str();
    let banner_h = 32.0_f32;
    let banner_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), banner_h)).1;
    let fill = rgba8_to_color32(lerp_oklab(color32_to_rgba8(tokens.surface_bg), color32_to_rgba8(tokens.warning), 0.14));
    ui.painter().rect_filled(banner_rect, CornerRadius::ZERO, fill);
    ui.painter().rect_stroke(banner_rect, CornerRadius::ZERO, Stroke::new(tokens.metrics.stroke_thin_pt, tokens.warning), StrokeKind::Inside);

    let pad_x = tokens.metrics.space_sm;
    let inner = Rect::from_min_max(pos2(banner_rect.left() + pad_x, banner_rect.top()), pos2(banner_rect.right() - pad_x, banner_rect.bottom()));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    bar_ui.set_clip_rect(banner_rect);

    let constraints_lbl = tr_or_fallback(locale, "results.banner.constraints", "Constraints");
    let compare_lbl = tr_or_fallback(locale, "results.banner.compare_horizontal", "Compare vs. Horizontal Line");
    let why_lbl = tr_or_fallback(locale, "results.banner.why", "Why?");
    let dismiss_lbl = tr_or_fallback(locale, "results.banner.dismiss", "Dismiss");

    let text_budget = (inner.width() - 400.0).max(80.0);
    let lbl_resp = bar_ui
        .add_sized([text_budget, inner.height()], egui::Label::new(RichText::new(&banner.summary).small().color(tokens.text_strong)).truncate());
    let lbl_resp = show_tooltip(&lbl_resp, tokens, TooltipSpec::new(&banner.summary));
    meta.record_response(
        &bar_ui,
        &lbl_resp,
        FrameInspectEntryInput {
            widget_id: "result.diagnostic_banner",
            role: "status",
            label: &banner.summary,
            command_id: Some("ui.result.inspectDiagnostics"),
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btns| {
        let (_, dismiss_clicked) =
            ghost_action_button(btns, tokens, meta, "result.diagnostic_banner.dismiss", dismiss_lbl, None, false, enabled, ZLayer::Workspace);
        if dismiss_clicked {
            state.standalone_dismissed_banners.insert(rs.id.clone());
            if state.standalone_banner_why_open.as_ref() == Some(&rs.id) {
                state.standalone_banner_why_open = None;
            }
            if state.compare_horizontal_open_sheet.as_ref() == Some(&rs.id) {
                state.compare_horizontal_open_sheet = None;
            }
            intents.push(UiIntent::DismissDiagnosticBanner { sheet_id: rs.id.clone() });
        }

        let why_open = state.standalone_banner_why_open.as_ref() == Some(&rs.id);
        let (_, why_clicked) =
            ghost_action_button(btns, tokens, meta, "result.diagnostic_banner.why", why_lbl, None, why_open, enabled, ZLayer::Workspace);
        if why_clicked {
            state.compare_horizontal_open_sheet = None;
            state.standalone_banner_why_open = if why_open { None } else { Some(rs.id.clone()) };
        }

        let cmp_open = state.compare_horizontal_open_sheet.as_ref() == Some(&rs.id);
        let (_, cmp_clicked) = ghost_action_button(
            btns,
            tokens,
            meta,
            "result.diagnostic_banner.compare_horizontal",
            compare_lbl,
            None,
            cmp_open,
            enabled,
            ZLayer::Workspace,
        );
        if cmp_clicked {
            state.standalone_banner_why_open = None;
            trigger_horizontal_line_comparison(app.document(), app.mutation_seq(), &rs.id, state, locale);
        }

        let (_, constraints_clicked) = ghost_action_button(
            btns,
            tokens,
            meta,
            "result.diagnostic_banner.constraints",
            constraints_lbl,
            Some("ui.editParams.open"),
            false,
            enabled,
            ZLayer::Workspace,
        );
        if constraints_clicked {
            intents.push_ui("ui.editParams.open", json!({ "sheet_id": rs.id.as_str() }));
        }
    });

    if enabled && state.standalone_banner_why_open.as_ref() == Some(&rs.id) && !banner.limitation_note.is_empty() {
        let pop_w = (banner_rect.width() - 16.0).clamp(200.0, 520.0);
        egui::Area::new(Id::unique(("results_standalone_why", rs.id.as_str())))
            .order(egui::Order::Middle)
            .fixed_pos(pos2(banner_rect.left() + 8.0, banner_rect.bottom() + 2.0))
            .show(ui.ctx(), |pop_ui| {
                egui::Frame::popup(pop_ui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.warning)).show(pop_ui, |inner_ui| {
                    inner_ui.set_width(pop_w);
                    inner_ui.label(RichText::new(&banner.limitation_note).small().color(tokens.text_primary));
                });
            });
    }
    render_horizontal_line_comparison_popup(ui, tokens, banner_rect, false, &rs.id, state, enabled, meta);
}

fn render_imported_result_banner(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let is_stale = matches!(rs.status, ResultStatus::ImportedStale);
    let accent = if is_stale { tokens.warning } else { tokens.link_blue };
    let banner_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), 28.0)).1;
    let fill = rgba8_to_color32(lerp_oklab(color32_to_rgba8(tokens.surface_bg), color32_to_rgba8(accent), 0.14));
    ui.painter().rect_filled(banner_rect, CornerRadius::ZERO, fill);
    ui.painter().rect_stroke(banner_rect, CornerRadius::ZERO, Stroke::new(tokens.metrics.stroke_thin_pt, accent), StrokeKind::Inside);

    let prefix = tr_or_fallback(tokens.locale, "results.imported.banner_prefix", "Read-only imported result");
    let not_computed = tr_or_fallback(tokens.locale, "results.imported.not_computed", "not computed by CurveBook");
    let stale_suffix = tr_or_fallback(tokens.locale, "results.imported.stale_suffix", "stale: upstream data changed");
    let msg = match &rs.provenance {
        curvebook_doc::ResultProvenance::Imported { source_format, original_kind_label, .. } => {
            if is_stale {
                format!("{prefix} ({source_format}: {original_kind_label}) — {stale_suffix}.")
            } else {
                format!("{prefix} ({source_format}: {original_kind_label}; {not_computed}).")
            }
        }
        curvebook_doc::ResultProvenance::Computed => format!("{prefix} ({not_computed})."),
    };

    ui.scope_builder(egui::UiBuilder::new().max_rect(banner_rect.shrink2(vec2(tokens.metrics.space_md, tokens.metrics.space_xxs))), |ui| {
        ui.set_clip_rect(banner_rect);
        ui.expand_to_include_rect(banner_rect);
        ui.horizontal(|ui| {
            let lbl = ui.add(egui::Label::new(RichText::new(&msg).small().color(tokens.text_strong)).truncate());
            meta.record_response(
                ui,
                &lbl,
                FrameInspectEntryInput {
                    widget_id: "result.imported_banner",
                    role: "label",
                    label: &msg,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            lbl.on_hover_text(&msg);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_lbl = tr_or_fallback(tokens.locale, "results.imported.rerun_analysis", "Re-run as a CurveBook analysis");
                let btn = ui.add_enabled(enabled, egui::Button::new(RichText::new(btn_lbl).small()));
                meta.record_response(
                    ui,
                    &btn,
                    FrameInspectEntryInput {
                        widget_id: "result.imported.rerun_analysis",
                        role: "button",
                        label: btn_lbl,
                        command_id: Some("ui.analyze.open"),
                        value: None,
                        enabled,
                        disabled_reason: (!enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if btn.clicked() && enabled {
                    let params = match rs.analysis.source_sheet_id.as_ref() {
                        Some(src) => json!({ "sheet_id": src.as_str() }),
                        None => json!({}),
                    };
                    intents.push_ui("ui.analyze.open", params);
                }
            });
        });
    });
}

fn render_ambiguity_callout(
    ui: &mut egui::Ui,
    rs: &ResultSheet,
    locale: &str,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let banner_h = 28.0_f32;
    let banner_rect = ui.allocate_space(vec2(ui.available_width().max(1.0), banner_h)).1;
    let fill = rgba8_to_color32(lerp_oklab(color32_to_rgba8(tokens.surface_bg), color32_to_rgba8(tokens.warning), 0.14));
    let border = tokens.warning;
    ui.painter().rect_filled(banner_rect, CornerRadius::ZERO, fill);
    ui.painter().rect_stroke(banner_rect, CornerRadius::ZERO, Stroke::new(tokens.metrics.stroke_thin_pt, border), StrokeKind::Inside);

    ui.scope_builder(egui::UiBuilder::new().max_rect(banner_rect.shrink2(vec2(tokens.metrics.space_md, tokens.metrics.space_xxs))), |ui| {
        ui.set_clip_rect(banner_rect);
        ui.expand_to_include_rect(banner_rect);
        ui.horizontal(|ui| {
            let msg = tr_or_fallback(
                locale,
                "results.banner.ambiguous_summary",
                "Ambiguous / unbounded fit: profile-likelihood bounds did not close or parameters are unstable.",
            );
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
            let _ = show_tooltip(&lbl, tokens, TooltipSpec::new(msg));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_lbl = tr_or_fallback(locale, "results.banner.constraints", "Constraints");
                let (_, clicked) = ghost_action_button(
                    ui,
                    tokens,
                    meta,
                    "result.ambiguity_callout.constraints",
                    btn_lbl,
                    Some("ui.editParams.open"),
                    false,
                    enabled,
                    ZLayer::Workspace,
                );
                if clicked {
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
struct ResultTableLayout<'a> {
    col_start: usize,
    col_end: usize,
    row_stub_w: f32,
    col_widths: &'a [f32],
    col_x_offsets: &'a [f32],
    row_h: f32,
    total_w: f32,
    visible_clip: Rect,
    pane_focused: bool,
    high_contrast: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColTier {
    Ci,
    Primary,
    RowStub,
    Secondary,
}

impl ColTier {
    const fn slack_weight(self) -> f32 {
        match self {
            Self::Ci => 2.0,
            Self::Primary | Self::RowStub => 1.0,
            Self::Secondary => 0.5,
        }
    }
}

fn classify_col_tier(header: &str, sampled_cells: &[&ResultCell]) -> (ColTier, f32, f32) {
    let lower = header.trim().to_ascii_lowercase();
    let is_ci_header = lower == "ci"
        || lower.ends_with(" ci")
        || lower.contains("95% ci")
        || lower.contains("90% ci")
        || lower.contains("99% ci")
        || lower.contains("ci (")
        || lower.contains("confidence interval")
        || lower.contains("profile");
    if is_ci_header || sampled_cells.iter().any(|c| matches!(c, ResultCell::ConfidenceInterval { .. })) {
        return (ColTier::Ci, 144.0, 320.0);
    }

    const PRIMARY_TERMS: &[&str] = &[
        "best-fit",
        "value",
        "estimate",
        "mean",
        "median",
        "slope",
        "intercept",
        "ic50",
        "ec50",
        "logec50",
        "logic50",
        "hill",
        "p value",
        "p-value",
        "p.value",
        "adjusted p",
        "std",
        "se",
        "r2",
        "r squared",
        "r²",
        "aicc",
        "sum of",
        "df",
        "ratio",
        "diff",
        "q",
        "n",
        "f (",
        "t (",
    ];
    let is_primary_header = PRIMARY_TERMS.iter().any(|term| lower.contains(term));
    let has_numeric_or_p = sampled_cells.iter().any(|c| matches!(c, ResultCell::PValue { .. } | ResultCell::Number { .. }));
    if is_primary_header || has_numeric_or_p {
        return (ColTier::Primary, 92.0, 180.0);
    }

    const LONG_TEXT_TERMS: &[&str] = &["summary", "note", "diagnostic", "assumption", "interpretation", "detail", "reason", "method", "equation"];
    let max_w = if LONG_TEXT_TERMS.iter().any(|term| lower.contains(term)) { 240.0 } else { 160.0 };
    (ColTier::Secondary, 84.0, max_w)
}

fn collect_bounded_sample_rows(total_rows: usize, vis_start: usize) -> Vec<usize> {
    if total_rows <= 256 {
        return (0..total_rows).collect();
    }
    let mut seen = BTreeSet::new();
    for r in 0..128.min(total_rows) {
        seen.insert(r);
    }
    for r in total_rows.saturating_sub(64)..total_rows {
        seen.insert(r);
    }
    let vis_end = (vis_start + 64).min(total_rows);
    for r in vis_start.min(total_rows)..vis_end {
        if seen.len() >= 256 {
            break;
        }
        seen.insert(r);
    }
    seen.into_iter().take(256).collect()
}

fn shrink_tier_toward_min(widths: &mut [f32], mins: &[f32], tiers: &[ColTier], target_tier: ColTier, deficit: &mut f32) {
    if *deficit <= 0.5 {
        return;
    }
    let total_headroom: f32 = widths
        .iter()
        .zip(mins.iter())
        .zip(tiers.iter())
        .filter(|((_, _), tier)| **tier == target_tier)
        .map(|((&w, &min_w), _)| (w - min_w).max(0.0))
        .sum();
    if total_headroom <= 0.5 {
        return;
    }
    let take = (*deficit).min(total_headroom);
    for ((w, &min_w), &tier) in widths.iter_mut().zip(mins.iter()).zip(tiers.iter()) {
        if tier == target_tier {
            let room = (*w - min_w).max(0.0);
            if room > 0.0 {
                let delta = (take * (room / total_headroom)).min(room);
                *w -= delta;
                *deficit -= delta;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_or_reuse_table_layout(
    ui: &egui::Ui,
    app: &UiApp,
    rs: &ResultSheet,
    active_tab_idx: usize,
    tab: &ResultTab,
    is_survival_summary: bool,
    num_cols: usize,
    avail_w: f32,
    vis_row_start: usize,
    tokens: &Tokens,
) -> (f32, Vec<f32>, Vec<f32>, f32, f32) {
    let avail_w_q = (avail_w * 2.0).round() as i32;
    let vis_bucket = vis_row_start / 64;
    let font_size_q = (tokens.metrics.font_ui_small_pt * 10.0).round() as i32;
    let sig_figs = rs.analysis.kind.output_significant_digits();
    let p_fmt = rs.analysis.p_value_format;
    let locale = app.locale.as_str();

    let cache_id = Id::unique(("cb_results_col_layout", rs.id.as_str(), active_tab_idx));
    if let Some(c) = ui.ctx().data(|d| d.get_temp::<CachedResultTableLayout>(cache_id))
        && c.sheet_id == rs.id
        && c.tab_idx == active_tab_idx
        && c.mutation_seq == app.mutation_seq()
        && c.row_count == tab.rows.len()
        && c.col_count == num_cols
        && c.avail_w_q == avail_w_q
        && c.vis_bucket == vis_bucket
        && c.font_size_q == font_size_q
        && c.p_fmt == p_fmt
        && c.sig_figs == sig_figs
        && c.locale == locale
    {
        return (c.row_stub_w, c.col_widths.clone(), c.col_x_offsets.clone(), c.total_w, c.natural_table_width_pt);
    }

    let sample_rows = collect_bounded_sample_rows(tab.rows.len(), vis_row_start);
    let pad_chrome = 16.0_f32;
    let hdr_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
    let num_font = super::fonts::aligned_numeral_font_id(ui.ctx(), tokens);
    let bold_num_font = if super::fonts::has_bold_font(ui.ctx()) {
        FontId::new(tokens.metrics.font_grid_numeral_pt, egui::FontFamily::Name(super::fonts::BOLD_FONT_FAMILY.into()))
    } else {
        num_font.clone()
    };

    let measure_w = |text: &str, font: &FontId| -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), tokens.text_primary).size().x)
    };

    let corner_lbl = tr_or_fallback(locale, "results.row_metric", "Row / Metric");
    let mut natural_stub_w = measure_w(corner_lbl, &hdr_font) + pad_chrome;
    for &r_idx in &sample_rows {
        if let Some(lbl) = tab.row_labels.get(r_idx) {
            natural_stub_w = natural_stub_w.max(measure_w(lbl, &hdr_font) + pad_chrome);
        }
    }
    let stub_min = 120.0_f32;
    let stub_max = 260.0_f32;
    natural_stub_w = natural_stub_w.clamp(stub_min, stub_max);

    let mut tiers = Vec::with_capacity(num_cols + 1);
    let mut min_ws = Vec::with_capacity(num_cols + 1);
    let mut max_ws = Vec::with_capacity(num_cols + 1);
    let mut natural_ws = Vec::with_capacity(num_cols + 1);

    tiers.push(ColTier::RowStub);
    min_ws.push(stub_min);
    max_ws.push(stub_max);
    natural_ws.push(natural_stub_w);

    for c_idx in 0..num_cols {
        let h_txt = tab.column_headers.get(c_idx).map_or("", String::as_str);
        let fallback_col = format!("Col {}", c_idx + 1);
        let display_h = if h_txt.is_empty() { fallback_col.as_str() } else { h_txt };
        let sampled_cells: Vec<&ResultCell> = sample_rows.iter().filter_map(|&r_idx| tab.rows.get(r_idx).and_then(|row| row.get(c_idx))).collect();
        let (tier, tier_min, tier_max) = classify_col_tier(display_h, &sampled_cells);
        let mut max_text_w = measure_w(display_h, &hdr_font) + pad_chrome;
        for &r_idx in &sample_rows {
            if let Some(cell) = tab.rows.get(r_idx).and_then(|row| row.get(c_idx)) {
                let fmt = format_result_cell_semantic_localized(cell, p_fmt, is_survival_summary, sig_figs, locale);
                if !fmt.text.is_empty() {
                    let f = if fmt.is_significant { &bold_num_font } else { &num_font };
                    max_text_w = max_text_w.max(measure_w(&fmt.text, f) + pad_chrome);
                }
            }
        }
        tiers.push(tier);
        min_ws.push(tier_min);
        max_ws.push(tier_max);
        natural_ws.push(max_text_w.clamp(tier_min, tier_max));
    }

    let natural_table_width_pt: f32 = natural_ws.iter().sum();
    let mut resolved_ws = natural_ws.clone();
    let sum_min: f32 = min_ws.iter().sum();

    if natural_table_width_pt <= avail_w {
        let caps: Vec<f32> = natural_ws.iter().zip(max_ws.iter()).map(|(&nat, &max_auto)| (nat * 1.45).min(max_auto)).collect();
        let mut slack = avail_w - natural_table_width_pt;
        for _ in 0..3 {
            if slack <= 0.5 {
                break;
            }
            let total_weight: f32 = resolved_ws
                .iter()
                .zip(caps.iter())
                .zip(tiers.iter())
                .filter(|&((&w, &cap), _)| w + 0.5 < cap)
                .map(|((_, _), tier)| tier.slack_weight())
                .sum();
            if total_weight <= 0.0 {
                break;
            }
            let round_slack = slack;
            for ((w, &cap), &tier) in resolved_ws.iter_mut().zip(caps.iter()).zip(tiers.iter()) {
                let headroom = (cap - *w).max(0.0);
                if headroom > 0.0 {
                    let add = (round_slack * (tier.slack_weight() / total_weight)).min(headroom);
                    *w += add;
                    slack -= add;
                }
            }
        }
    } else {
        let mut deficit = natural_table_width_pt - avail_w;
        shrink_tier_toward_min(&mut resolved_ws, &min_ws, &tiers, ColTier::Secondary, &mut deficit);
        shrink_tier_toward_min(&mut resolved_ws, &min_ws, &tiers, ColTier::RowStub, &mut deficit);
        shrink_tier_toward_min(&mut resolved_ws, &min_ws, &tiers, ColTier::Primary, &mut deficit);
        if avail_w >= sum_min {
            shrink_tier_toward_min(&mut resolved_ws, &min_ws, &tiers, ColTier::Ci, &mut deficit);
        }
    }

    let row_stub_w = resolved_ws[0];
    let col_widths = resolved_ws[1..].to_vec();
    let mut col_x_offsets = Vec::with_capacity(num_cols + 1);
    col_x_offsets.push(row_stub_w);
    for &w in &col_widths {
        let next_x = col_x_offsets.last().copied().unwrap_or(row_stub_w) + w;
        col_x_offsets.push(next_x);
    }
    let total_w = col_x_offsets.last().copied().unwrap_or(row_stub_w).max(avail_w);

    let cached_layout = CachedResultTableLayout {
        sheet_id: rs.id.clone(),
        tab_idx: active_tab_idx,
        mutation_seq: app.mutation_seq(),
        row_count: tab.rows.len(),
        col_count: num_cols,
        avail_w_q,
        vis_bucket,
        font_size_q,
        p_fmt,
        sig_figs,
        locale: locale.to_owned(),
        row_stub_w,
        col_widths: col_widths.clone(),
        col_x_offsets: col_x_offsets.clone(),
        total_w,
        natural_table_width_pt,
    };
    ui.ctx().data_mut(|d| d.insert_temp(cache_id, cached_layout));
    (row_stub_w, col_widths, col_x_offsets, total_w, natural_table_width_pt)
}

fn diagnostic_metric_summary(locale: &str, label: &str) -> Option<&'static str> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("hougaard") || lower.contains("|g1|") {
        Some(tr_or_fallback(locale, "results.diag.hougaard.summary", "Curvature skewness: |g1| < 0.1 linear; > 1.0 skewed symmetric CI"))
    } else if lower == "dependency" || lower == "parameter dependency" {
        Some(tr_or_fallback(locale, "results.diag.dependency.summary", "Parameter collinearity (0 to 1): > 0.99 signals redundant parameters"))
    } else if lower == "sy.x" {
        Some(tr_or_fallback(locale, "results.diag.syx.summary", "Residual standard deviation around fitted curve; lower means closer fit"))
    } else if lower == "aicc" {
        Some(tr_or_fallback(locale, "results.diag.aicc.summary", "Corrected Akaike criterion balancing fit and parameters; lower is better"))
    } else if lower == "bic" {
        Some(tr_or_fallback(locale, "results.diag.bic.summary", "Bayesian criterion penalizing extra parameters; lower is better"))
    } else if lower == "confidence interval" || lower == "ci" || lower.ends_with("% ci") {
        Some(tr_or_fallback(locale, "results.diag.ci.summary", "Interval likely containing true value; unbounded means weak ID"))
    } else if trimmed == "R²"
        || lower == "r^2"
        || lower == "r squared"
        || lower == "adj. r²"
        || lower == "adjusted r-squared"
        || lower == "pseudo r²"
        || lower == "pseudo r-squared"
    {
        Some(tr_or_fallback(locale, "results.diag.r2.summary", "Fraction of variance explained (0 to 1); higher is a tighter fit"))
    } else if lower == "vif" {
        Some(tr_or_fallback(locale, "results.diag.vif.summary", "Variance inflation factor: < 5 low collinearity; > 10 severe overlap"))
    } else if lower == "std. error" || lower == "se" || lower == "std. error of mean" {
        Some(tr_or_fallback(locale, "results.diag.se.summary", "Standard error measuring uncertainty of the parameter estimate"))
    } else if lower.starts_with("p value") || lower == "g-g p" || lower == "h-f p" {
        Some(tr_or_fallback(locale, "results.diag.pvalue.summary", "Probability of seeing an effect this extreme if null hypothesis holds"))
    } else {
        None
    }
}

fn render_result_table_header(ui: &mut egui::Ui, tab: &ResultTab, tokens: &Tokens, layout: ResultTableLayout<'_>) {
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
        let corner_elided = paint_table_cell_text(
            ui,
            corner_rect,
            corner_lbl,
            tokens.text_strong,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            egui::Align::LEFT,
            layout.visible_clip,
        );
        if corner_elided {
            let resp = ui.interact(corner_vis, egui::Id::unique(("res_hdr_corner", tab.id.as_str())), Sense::hover());
            let _ = show_tooltip(&resp, tokens, TooltipSpec::new(corner_lbl));
        }
    }

    for c_idx in layout.col_start..layout.col_end {
        let col_x = layout.col_x_offsets[c_idx];
        let col_w = layout.col_widths[c_idx];
        let cell_rect = Rect::from_min_size(pos2(hdr_rect.left() + col_x, hdr_rect.top()), vec2(col_w, layout.row_h));
        let cell_vis = cell_rect.intersect(hdr_vis);
        if cell_vis.width() < 4.0 || cell_vis.height() < 4.0 {
            continue;
        }
        hdr_painter.line_segment([cell_rect.left_top(), cell_rect.left_bottom()], Stroke::new(1.0, tokens.divider));
        let h_txt = tab.column_headers.get(c_idx).map_or("", String::as_str);
        let fallback_col = format!("Col {}", c_idx + 1);
        let display_h = if h_txt.is_empty() { fallback_col.as_str() } else { h_txt };
        let h_elided = paint_table_cell_text(
            ui,
            cell_rect,
            display_h,
            tokens.text_strong,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            egui::Align::RIGHT,
            layout.visible_clip,
        );
        let diag_tip = diagnostic_metric_summary(tokens.locale, display_h);
        if h_elided || diag_tip.is_some() {
            let hdr_resp = ui.interact(cell_vis, egui::Id::unique(("res_hdr", tab.id.as_str(), c_idx)), Sense::hover());
            let _ = show_tooltip_for_control(
                &hdr_resp,
                tokens,
                display_h,
                h_elided,
                TooltipSpec { title: display_h, shortcut: None, summary: diag_tip, disabled_reason: None },
            );
        }
    }
}

fn render_result_section_row(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    r_idx: usize,
    row_lbl: &str,
    state: &mut ResultsViewEguiState,
    layout: ResultTableLayout<'_>,
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
        let clicked_inside =
            state.selected_range(rs_id, active_tab_idx).is_some_and(|(r0, r1, c0, c1)| r_idx >= r0 && r_idx <= r1 && c_idx >= c0 && c_idx <= c1);
        if !clicked_inside {
            state.selection_anchor = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
            state.selected_cell = Some((rs_id.clone(), active_tab_idx, r_idx, c_idx));
            state.selection_sync_needed = true;
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
    layout: ResultTableLayout<'a>,
}

fn render_result_data_row(
    ui: &mut egui::Ui,
    ctx: &ResultTabRenderCtx<'_>,
    input: ResultDataRowInput<'_>,
    state: &mut ResultsViewEguiState,
    meta: &mut FrameInspectMeta,
) {
    let ResultTabRenderCtx { rs, tab, tokens, enabled } = *ctx;
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
    let stub_vis = stub_rect.intersect(row_vis);
    if stub_vis.width() >= 4.0 {
        let stub_elided = paint_table_cell_text(
            ui,
            stub_rect,
            row_lbl,
            tokens.text_primary,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            egui::Align::LEFT,
            layout.visible_clip,
        );
        let stub_diag_tip = diagnostic_metric_summary(tokens.locale, row_lbl);
        if (stub_elided || stub_diag_tip.is_some()) && !row_lbl.is_empty() {
            let stub_resp = ui.interact(stub_vis, egui::Id::unique(("res_stub", rs.id.as_str(), active_tab_idx, r_idx)), Sense::hover());
            let _ = show_tooltip_for_control(
                &stub_resp,
                tokens,
                row_lbl,
                stub_elided,
                TooltipSpec { title: row_lbl, shortcut: None, summary: stub_diag_tip, disabled_reason: None },
            );
        }
    }

    let locale = tokens.locale;
    for c_idx in layout.col_start..layout.col_end {
        let col_x = layout.col_x_offsets[c_idx];
        let col_w = layout.col_widths[c_idx];
        let cell_rect = Rect::from_min_size(pos2(row_rect.left() + col_x, row_rect.top()), vec2(col_w, layout.row_h));
        let cell_vis = cell_rect.intersect(row_vis);
        if cell_vis.width() < 4.0 || cell_vis.height() < 4.0 {
            continue;
        }
        row_painter.line_segment([cell_rect.left_top(), cell_rect.left_bottom()], Stroke::new(1.0, tokens.divider));
        let cell = row.get(c_idx).unwrap_or(&ResultCell::Empty);
        let sig_figs = rs.analysis.kind.output_significant_digits();
        let fmt = format_result_cell_semantic_localized(cell, rs.analysis.p_value_format, is_survival_summary, sig_figs, locale);

        let mut cell_resp = ui.interact(
            cell_vis,
            egui::Id::unique(("res_cell", rs.id.as_str(), active_tab_idx, r_idx, c_idx)),
            if enabled { Sense::click() } else { Sense::hover() },
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
        let is_focused_cell =
            state.selected_cell.as_ref().is_some_and(|(sid, t, r, c)| sid == &rs.id && *t == active_tab_idx && *r == r_idx && *c == c_idx);
        let find_hl = state.active_find_matches.as_ref().filter(|h| h.tab_idx == active_tab_idx);
        let find_active = find_hl.is_some_and(|h| h.active_match == Some((r_idx, c_idx)));
        let find_match = find_active || find_hl.is_some_and(|h| h.matching_cells.contains(&(r_idx, c_idx)));
        paint_result_cell_selection_and_find(
            ui,
            tokens,
            cell_rect,
            cell_vis,
            layout.pane_focused,
            layout.high_contrast,
            selected,
            is_focused_cell,
            find_match,
            find_active,
        );

        let cell_ctx = ResultCellPaintCtx { rs, tab, cell, tokens, fmt: &fmt, cell_rect, visible_clip: layout.visible_clip, r_idx, c_idx };
        let elided = paint_result_cell_text_and_tooltip(ui, &cell_ctx, &mut cell_resp);
        record_result_cell_meta(ui, meta, &cell_resp, &fmt.text, enabled, r_idx, c_idx, elided);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_result_cell_selection_and_find(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    cell_rect: Rect,
    cell_vis: Rect,
    pane_focused: bool,
    high_contrast: bool,
    selected: bool,
    is_focused_cell: bool,
    find_match: bool,
    find_active: bool,
) {
    if !selected && !find_match {
        return;
    }
    let cell_painter = ui.painter().with_clip_rect(cell_vis);
    if selected {
        let sel_fill = if pane_focused { tokens.active_bg } else { tokens.selection_unemphasized_bg };
        cell_painter.rect_filled(cell_vis, CornerRadius::ZERO, sel_fill);
    }
    if find_match {
        let tint = tokens.warning.linear_multiply(if find_active { 0.38 } else { 0.22 });
        cell_painter.rect_filled(cell_vis, CornerRadius::ZERO, tint);
    }
    if find_active {
        cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.accent), StrokeKind::Inside);
    } else if selected {
        if pane_focused {
            let focus_w = if is_focused_cell { 1.5 } else { 1.0 };
            cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(focus_w, tokens.accent), StrokeKind::Inside);
        } else if high_contrast {
            paint_dashed_rect_stroke(&cell_painter, cell_rect, Stroke::new(1.0, tokens.border));
        } else {
            let stroke_col = if is_focused_cell { tokens.border } else { tokens.divider };
            cell_painter.rect_stroke(cell_rect, CornerRadius::ZERO, Stroke::new(1.0, stroke_col), StrokeKind::Inside);
        }
    }
}

struct ResultCellPaintCtx<'a> {
    rs: &'a ResultSheet,
    tab: &'a ResultTab,
    cell: &'a ResultCell,
    tokens: &'a Tokens,
    fmt: &'a FormattedCellDisplay,
    cell_rect: Rect,
    visible_clip: Rect,
    r_idx: usize,
    c_idx: usize,
}

fn format_full_precision_scalar(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    if value == 0.0 {
        return String::from("0");
    }
    let abs_v = value.abs();
    if (1e-4..1e12).contains(&abs_v) {
        let s = curvebook_doc::format_significant_digits(value, 12);
        if s.contains('.') && !s.contains('e') && !s.contains('E') {
            return s.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        return s;
    }
    format!("{value:.10e}")
}

fn full_precision_detail_for_cell(cell: &ResultCell, disp_text: &str, locale: &str) -> Option<String> {
    match cell {
        ResultCell::Number { value, prefix, units } if value.is_finite() => {
            let full_raw = format_full_precision_scalar(*value);
            if full_raw.is_empty() {
                return None;
            }
            let mut full_disp = String::new();
            if let Some(p) = prefix {
                full_disp.push_str(p);
            }
            full_disp.push_str(&full_raw);
            if let Some(u) = units.as_deref().filter(|u| !u.is_empty()) {
                full_disp.push(' ');
                full_disp.push_str(u);
            }
            let rounded_by_text = full_disp != disp_text.trim() && {
                let core = disp_text.trim().strip_prefix('~').unwrap_or(disp_text.trim());
                let numeric_head = core.split_whitespace().next().unwrap_or(core);
                numeric_head.parse::<f64>().map_or(true, |parsed| (parsed - *value).abs() > value.abs() * 1e-12 + 1e-15)
            };
            if rounded_by_text {
                let lbl = tr_or_fallback(locale, "results.cell.full_precision", "Full precision");
                Some(format!("{lbl}: {full_disp}"))
            } else {
                None
            }
        }
        ResultCell::PValue { raw, .. } if raw.is_finite() => {
            let full_raw = format_full_precision_scalar(*raw);
            let same = disp_text.trim().parse::<f64>().is_ok_and(|p| (p - *raw).abs() <= raw.abs() * 1e-12 + 1e-15);
            if !same && full_raw != disp_text.trim() {
                let lbl = tr_or_fallback(locale, "results.cell.full_precision", "Full precision");
                Some(format!("{lbl}: {full_raw}"))
            } else {
                None
            }
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            let (CiBound::Finite(lo), CiBound::Finite(hi)) = (lower, upper) else {
                return None;
            };
            if !lo.is_finite() || !hi.is_finite() {
                return None;
            }
            let full_ci = format!("[{}, {}]", format_full_precision_scalar(*lo), format_full_precision_scalar(*hi));
            if full_ci != disp_text.trim() {
                let lbl = tr_or_fallback(locale, "results.cell.full_precision", "Full precision");
                Some(format!("{lbl}: {full_ci}"))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn hougaard_band_note(cell: &ResultCell) -> Option<&'static str> {
    let ResultCell::Number { value, .. } = cell else {
        return None;
    };
    if !value.is_finite() {
        return None;
    }
    let abs_g1 = value.abs();
    if abs_g1 < 0.1 {
        Some("|g1| < 0.1: close-to-linear asymptotic CI behavior")
    } else if abs_g1 <= 1.0 {
        Some("0.1 <= |g1| <= 1.0: moderate parameter nonlinearity")
    } else {
        Some("|g1| > 1.0: strong nonlinearity; symmetric CI is skewed")
    }
}

fn paint_result_cell_text_and_tooltip(ui: &mut egui::Ui, ctx: &ResultCellPaintCtx<'_>, cell_resp: &mut egui::Response) -> bool {
    let ResultCellPaintCtx { rs, tab, cell, tokens, fmt, cell_rect, visible_clip, r_idx, c_idx } = *ctx;
    if fmt.text.is_empty() {
        return false;
    }
    let col = if fmt.is_ambiguous {
        tokens.role_cat
    } else if fmt.is_muted {
        tokens.text_muted
    } else if fmt.is_significant {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let font = if fmt.is_significant && super::fonts::has_bold_font(ui.ctx()) {
        FontId::new(tokens.metrics.font_grid_numeral_pt, egui::FontFamily::Name(super::fonts::BOLD_FONT_FAMILY.into()))
    } else {
        super::fonts::aligned_numeral_font_id(ui.ctx(), tokens)
    };
    let elided = paint_table_cell_text(ui, cell_rect, &fmt.text, col, font, egui::Align::RIGHT, visible_clip);
    let col_hdr = tab.column_headers.get(c_idx).map_or("", String::as_str);
    let row_lbl = tab.row_labels.get(r_idx).map_or("", String::as_str);
    let cell_detail = rs.cell_detail(&tab.id, r_idx, c_idx);
    let full_prec = full_precision_detail_for_cell(cell, &fmt.text, tokens.locale);
    let is_hougaard_col = col_hdr.eq_ignore_ascii_case("Hougaard |g1|") || col_hdr.to_ascii_lowercase().contains("hougaard");
    let g1_band = if is_hougaard_col { hougaard_band_note(cell) } else { None };

    let mut summary_parts = Vec::new();
    if elided {
        summary_parts.push(fmt.text.clone());
    }
    if let Some(fp) = full_prec
        && !cell_detail.is_some_and(|d| d.detail.contains(&fp))
    {
        summary_parts.push(fp);
    }
    if let Some(detail) = cell_detail {
        if let Some(raw) = detail.raw_value.filter(|v| v.is_finite()) {
            let raw_s = format_full_precision_scalar(raw);
            if !raw_s.is_empty() && !detail.detail.contains(&raw_s) && !summary_parts.iter().any(|s| s.contains(&raw_s)) {
                let lbl = tr_or_fallback(tokens.locale, "results.cell.full_precision", "Full precision");
                summary_parts.push(format!("{lbl}: {raw_s}"));
            }
        }
        if !detail.detail.trim().is_empty() {
            summary_parts.push(detail.detail.clone());
        }
    }
    if let Some(band) = g1_band {
        summary_parts.push(band.to_owned());
    }
    if fmt.is_ambiguous && !summary_parts.iter().any(|s| s.to_ascii_lowercase().contains("ambiguous") || s.to_ascii_lowercase().contains("unstable"))
    {
        summary_parts.push(tr_or_fallback(tokens.locale, "results.banner.ambiguous_summary", "Ambiguous / unbounded estimate").to_owned());
    }

    if !summary_parts.is_empty() {
        let context_title = match (col_hdr.trim(), row_lbl.trim()) {
            ("", "") => fmt.text.clone(),
            ("", r) => r.to_owned(),
            (c, "") => c.to_owned(),
            (c, r) => format!("{c} — {r}"),
        };
        let summary_text = summary_parts.join(" • ");
        let use_title = if elided && summary_parts.len() == 1 { fmt.text.as_str() } else { context_title.as_str() };
        let use_summary = if elided && summary_parts.len() == 1 { None } else { Some(summary_text.as_str()) };
        *cell_resp = show_tooltip(cell_resp, tokens, TooltipSpec { title: use_title, shortcut: None, summary: use_summary, disabled_reason: None });
    }
    elided
}

#[allow(clippy::too_many_arguments)]
fn record_result_cell_meta(
    ui: &egui::Ui,
    meta: &mut FrameInspectMeta,
    cell_resp: &egui::Response,
    text: &str,
    enabled: bool,
    r_idx: usize,
    c_idx: usize,
    elided: bool,
) {
    let ak_cell = if text.trim().is_empty() { format!("R{}C{}", r_idx + 1, c_idx + 1) } else { text.to_owned() };
    cell_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Cell, enabled, ak_cell.clone()));
    let cell_wid = format!("result.cell.{r_idx}.{c_idx}");
    meta.record_response(
        ui,
        cell_resp,
        FrameInspectEntryInput {
            widget_id: &cell_wid,
            role: "gridcell",
            label: if text.is_empty() { "Cell" } else { text },
            command_id: None,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: elided,
            full_label: elided.then_some(text),
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn render_result_tab_table(
    ui: &mut egui::Ui,
    app: &UiApp,
    ctx: &ResultTabRenderCtx<'_>,
    active_tab_idx: usize,
    is_survival_summary: bool,
    pane_focused: bool,
    high_contrast: bool,
    viewport: Rect,
    state: &mut ResultsViewEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> f32 {
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
    let row_h = 24.0_f32;
    let spacing_y = ui.spacing().item_spacing.y;
    let row_stride = row_h + spacing_y;
    let body_top_est = (viewport.min.y - row_stride).max(0.0);
    let vis_row_start = ((body_top_est / row_stride).floor().max(0.0) as usize).min(tab.rows.len());
    let avail_w = viewport.width().max(ui.available_width()).max(1.0);
    let table_origin_x = ui.available_rect_before_wrap().left();

    let (row_stub_w, col_widths, col_x_offsets, total_w, natural_table_width_pt) =
        compute_or_reuse_table_layout(ui, app, rs, active_tab_idx, tab, is_survival_summary, num_cols, avail_w, vis_row_start, tokens);
    state.last_natural_table_width_pt = natural_table_width_pt;
    state.natural_table_widths.insert((rs.id.clone(), active_tab_idx), natural_table_width_pt);

    if let Some((ref m_sid, m_tab, target_col)) = state.pending_scroll_to_match_col
        && m_sid == &rs.id
        && m_tab == active_tab_idx
    {
        state.pending_scroll_to_match_col = None;
        if target_col > 0 && target_col - 1 < num_cols {
            let x_left = col_x_offsets[target_col - 1];
            let col_w = col_widths[target_col - 1];
            ui.scroll_to_rect(Rect::from_min_size(pos2(table_origin_x + x_left, viewport.top()), vec2(col_w, row_h)), None);
        }
    }

    let vis_x_min = viewport.min.x.max(0.0);
    let vis_x_max = viewport.max.x.max(0.0);
    let col_start = col_x_offsets[1..=num_cols].partition_point(|&right| right <= vis_x_min);
    let col_end = col_x_offsets[..num_cols].partition_point(|&left| left < vis_x_max).min(num_cols).max(col_start);
    let layout = ResultTableLayout {
        col_start,
        col_end,
        row_stub_w,
        col_widths: &col_widths,
        col_x_offsets: &col_x_offsets,
        row_h,
        total_w,
        visible_clip: ui.clip_rect().intersect(meta.viewport_rect),
        pane_focused,
        high_contrast,
    };

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
    natural_table_width_pt
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
    let copy_sel_lbl = tr(tokens.locale, "r13.result.copy_selection");
    let copy_all_lbl = tr(tokens.locale, "r13.result.copy_table");
    let entries = [
        ActionMenuEntry::Item(ActionMenuItem::menu_item("result.ctx.copy_selection", copy_sel_lbl)),
        ActionMenuEntry::Item(ActionMenuItem::menu_item("result.ctx.copy_table", copy_all_lbl).command_id(Some("results.copy_table"))),
    ];
    let out = action_menu_popup(
        ui.ctx(),
        tokens,
        Some(meta),
        ActionMenuSpec::context(Id::unique(("result_ctx_menu", rs.id.as_str())), "popup.result.ctx", "Result Actions", anchor),
        &entries,
    );
    match out.clicked_entry {
        Some(0) => {
            intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: false });
            state.context_menu = None;
        }
        Some(1) => {
            intents.push(UiIntent::CopyResultTable { sheet_id: rs.id.clone(), force_full: true });
            state.context_menu = None;
        }
        _ if out.dismissed => {
            state.context_menu = None;
        }
        _ => {}
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
            ResultStatus::Imported => {
                ui.colored_label(
                    tokens.text_muted,
                    tr_or_fallback(tokens.locale, "results.imported.empty_body", "Imported result snapshot (no tabular output stored)."),
                );
            }
            ResultStatus::ImportedStale => {
                ui.colored_label(
                    tokens.stale,
                    tr_or_fallback(
                        tokens.locale,
                        "results.imported.empty_stale_body",
                        "Imported result snapshot is stale (upstream data changed; no tabular output stored).",
                    ),
                );
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
        return false;
    }
    let h_clipped = clipped.left() > rect.left() + 0.5 || clipped.right() < rect.right() - 0.5;
    let max_w = (clipped.width() - pad_x * 2.0).max(1.0);
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::text::TextFormat { font_id, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: h_clipped, overflow_character: Some('…') };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let elided = galley.elided;
    let size = galley.size();
    if size.x > clipped.width() - pad_x && !elided {
        return false;
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

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, MultiplicitySpec, NonlinearFitSpec, PValueFormat, ProjectDocument,
        ResultCell, ResultCellDetail, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SurvivalAnalysisSpec,
    };

    use super::*;
    use crate::UiThemeMode;

    fn make_nl_test_sheet() -> (ProjectDocument, SheetId) {
        let mut doc = ProjectDocument::default();
        let rs_id = SheetId::new("res-nl-test");
        let spec = AnalysisSpec::new(
            rs_id.clone(),
            SheetId::new("ds-1"),
            AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
        )
        .unwrap();
        let mut rs = ResultSheet::new(spec).unwrap();
        rs.status = ResultStatus::UpToDate;
        rs.tabs.push(ResultTab {
            id: String::from("nonlinear_parameters"),
            title: String::from("Best-fit Values & Confidence Intervals"),
            kind: ResultTabKind::ParametersAndConfidenceIntervals,
            column_headers: vec![
                String::from("Best-fit value"),
                String::from("Std. Error"),
                String::from("95% CI (profile likelihood)"),
                String::from("P value"),
                String::from("Summary Notes"),
            ],
            row_labels: vec![String::from("LogIC50"), String::from("HillSlope"), String::from("Top"), String::from("Bottom")],
            rows: vec![
                vec![
                    ResultCell::Number { value: -6.125, prefix: None, units: None },
                    ResultCell::Number { value: 0.142, prefix: None, units: None },
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Open },
                    ResultCell::PValue { raw: 0.0002, formatted: String::from("0.0002"), stars: String::from("***") },
                    ResultCell::Text(String::from("Very long diagnostic observation summary note for ellipsis metadata check")),
                ],
                vec![
                    ResultCell::Text(String::from("Unstable")),
                    ResultCell::Number { value: 0.89, prefix: None, units: None },
                    ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open },
                    ResultCell::PValue { raw: 0.42, formatted: String::from("0.4200"), stars: String::new() },
                    ResultCell::Text(String::from("OK")),
                ],
                vec![
                    ResultCell::Number { value: 99.5, prefix: None, units: None },
                    ResultCell::Number { value: 2.1, prefix: None, units: None },
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Unavailable },
                    ResultCell::PValue { raw: 0.01, formatted: String::from("0.0100"), stars: String::from("*") },
                    ResultCell::Text(String::from("Check")),
                ],
                vec![
                    ResultCell::Number { value: 1.0, prefix: None, units: None },
                    ResultCell::Number { value: 0.5, prefix: None, units: None },
                    ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable },
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
            ],
            footnotes: Vec::new(),
        });
        rs.tabs.push(ResultTab {
            id: String::from("residuals"),
            title: String::from("Residuals Matrix"),
            kind: ResultTabKind::Residuals,
            column_headers: vec![String::from("Residual")],
            row_labels: vec![String::from("Row 1")],
            rows: vec![vec![ResultCell::Number { value: 0.25, prefix: None, units: None }]],
            footnotes: Vec::new(),
        });
        rs.cell_details.push(ResultCellDetail {
            tab_id: String::from("nonlinear_parameters"),
            row: 1,
            col: 0,
            raw_value: Some(1.234e4),
            detail: String::from("Raw solver iterate: 12340 (masked Unstable)"),
        });
        doc.result_sheets.push(rs);
        doc.active_sheet_id = Some(rs_id.clone());
        (doc, rs_id)
    }

    fn collect_text_colors(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Vec<Color32> {
        let mut out = Vec::new();
        for cs in shapes {
            if let egui::Shape::Text(ts) = &cs.shape
                && ts.galley.text().contains(needle)
            {
                if let Some(sec) = ts.galley.job.sections.first() {
                    out.push(sec.format.color);
                }
            } else if let egui::Shape::Vec(children) = &cs.shape {
                for ch in children {
                    if let egui::Shape::Text(ts) = ch
                        && ts.galley.text().contains(needle)
                        && let Some(sec) = ts.galley.job.sections.first()
                    {
                        out.push(sec.format.color);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn r20_10_explicit_ci_and_unstable_formatting_zero_question_marks() {
        let fit_half = ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Open };
        let fit_both = ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open };
        let unavail_half = ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Unavailable };
        let unavail_both = ResultCell::ConfidenceInterval { lower: CiBound::Unavailable, upper: CiBound::Unavailable };
        let surv_half = ResultCell::ConfidenceInterval { lower: CiBound::Finite(14.0), upper: CiBound::Open };

        let d_half = format_result_cell_semantic(&fit_half, PValueFormat::FourDecimal, false, None);
        assert_eq!(d_half.text, "[0.1753, unbounded]");
        assert!(d_half.is_ambiguous && !d_half.is_muted);

        let d_both = format_result_cell_semantic(&fit_both, PValueFormat::FourDecimal, false, None);
        assert_eq!(d_both.text, "Not bounded");
        assert!(d_both.is_ambiguous && !d_both.is_muted);

        let d_uh = format_result_cell_semantic(&unavail_half, PValueFormat::FourDecimal, false, None);
        assert_eq!(d_uh.text, "[0.1753, Unavailable]");
        assert!(d_uh.is_muted && !d_uh.is_ambiguous);

        let d_ub = format_result_cell_semantic(&unavail_both, PValueFormat::FourDecimal, false, None);
        assert_eq!(d_ub.text, "Unavailable");
        assert!(d_ub.is_muted && !d_ub.is_ambiguous);

        let d_sh = format_result_cell_semantic(&surv_half, PValueFormat::FourDecimal, true, None);
        assert_eq!(d_sh.text, "[14, Not reached]");
        assert!(!d_sh.is_ambiguous && !d_sh.is_muted);

        let d_sb = format_result_cell_semantic(&fit_both, PValueFormat::FourDecimal, true, None);
        assert_eq!(d_sb.text, "Not reached");
        assert!(!d_sb.is_ambiguous && d_sb.is_muted);

        let unstable = ResultCell::Text(String::from("Unstable"));
        let legacy_q = ResultCell::Text(String::from("??? (ambiguous)"));
        for cell in [&unstable, &legacy_q] {
            let d = format_result_cell_semantic(cell, PValueFormat::FourDecimal, false, None);
            assert_eq!(d.text, "Unstable");
            assert!(!d.text.contains("???"));
            assert!(d.is_ambiguous);
            assert!(result_cell_is_ambiguous(cell, false));
        }
        assert!(result_cell_is_ambiguous(&fit_both, false));
        assert!(!result_cell_is_ambiguous(&fit_both, true));

        let surv_spec = SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
            event_code: None,
            censored_code: None,
            treat_other_as: Default::default(),
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        };
        let mut s_rs = ResultSheet::new(AnalysisSpec::new("s-1", "ds-s", AnalysisKind::Survival(surv_spec)).unwrap()).unwrap();
        let s_tab = ResultTab {
            id: String::from("main"),
            title: String::from("Survival Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("95% CI")],
            row_labels: vec![String::from("Median")],
            rows: vec![vec![fit_both.clone()]],
            footnotes: Vec::new(),
        };
        s_rs.tabs.push(s_tab.clone());
        assert_eq!(format_result_cell_display_text(&fit_both, &s_rs, &s_tab), "Not reached");
    }

    #[test]
    fn r20_10_bounded_sampling_and_ci_first_layout_720_and_540_and_ellipsized_meta() {
        let sampled = collect_bounded_sample_rows(10_000, 5_000);
        assert_eq!(sampled.len(), 256);
        assert!(sampled.contains(&0) && sampled.contains(&127) && sampled.contains(&5_000) && sampled.contains(&9_999));

        let (doc, rs_id) = make_nl_test_sheet();
        let app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let tokens = Tokens::default();

        for width_pt in [720.0_f32, 540.0_f32] {
            let ctx = egui::Context::default();
            crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
            let mut rv = ResultsViewEguiState::default();
            let mut meta = FrameInspectMeta::new();
            let mut report = ResultsViewFrameReport::default();
            for _ in 0..2 {
                let mut intents = PendingUiIntents::new();
                meta = FrameInspectMeta::new();
                let mut out = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width_pt, 420.0))), ..Default::default() },
                    |ui| {
                        meta.begin_pass(ui.ctx());
                        report = render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv, &tokens, true, &mut meta, &mut intents);
                    },
                );
                out.textures_delta.clear();
            }
            assert!(report.natural_table_width_pt > 400.0);
            assert_eq!(rv.natural_table_width_for_sheet(&rs_id, 0), Some(report.natural_table_width_pt));

            let ci_entry = meta.find_by_widget_id("result.cell.0.2").expect("CI cell must be visible");
            assert!(!ci_entry.ellipsized, "CI cell must not ellipsize at {width_pt}pt: {ci_entry:?}");
            assert!(ci_entry.clipped_rect.width() >= 140.0, "CI col must preserve min 144pt width at {width_pt}pt");
        }

        // Verify narrow viewport (~540pt) with secondary text column visible records true ellipsized + full_label metadata
        // while keeping the CI column wide & un-ellipsized.
        let mut narrow_doc = ProjectDocument::default();
        let narrow_id = SheetId::new("res-nl-narrow");
        let mut narrow_rs = ResultSheet::new(
            AnalysisSpec::new(
                narrow_id.clone(),
                SheetId::new("ds-1"),
                AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")),
            )
            .unwrap(),
        )
        .unwrap();
        narrow_rs.status = ResultStatus::UpToDate;
        let long_note = "Very long diagnostic observation summary note for ellipsis metadata check";
        narrow_rs.tabs.push(ResultTab {
            id: String::from("nonlinear_parameters"),
            title: String::from("Best-fit Values"),
            kind: ResultTabKind::ParametersAndConfidenceIntervals,
            column_headers: vec![String::from("Value"), String::from("95% CI (profile likelihood)"), String::from("Summary Notes")],
            row_labels: vec![String::from("LogIC50")],
            rows: vec![vec![
                ResultCell::Number { value: -6.125, prefix: None, units: None },
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.1753), upper: CiBound::Open },
                ResultCell::Text(String::from(long_note)),
            ]],
            footnotes: Vec::new(),
        });
        narrow_doc.result_sheets.push(narrow_rs);
        narrow_doc.active_sheet_id = Some(narrow_id.clone());
        let narrow_app = UiApp::from_session(curvebook_engine::Session::from_document(narrow_doc));
        let ctx = egui::Context::default();
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
        let mut rv = ResultsViewEguiState::default();
        let mut meta = FrameInspectMeta::new();
        for _ in 0..2 {
            let mut intents = PendingUiIntents::new();
            meta = FrameInspectMeta::new();
            let mut out =
                ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(540.0, 420.0))), ..Default::default() }, |ui| {
                    meta.begin_pass(ui.ctx());
                    let _ = render_results_view_for_sheet(ui, &narrow_app, Some(&narrow_id), &mut rv, &tokens, true, &mut meta, &mut intents);
                });
            out.textures_delta.clear();
        }
        let ci_narrow = meta.find_by_widget_id("result.cell.0.1").expect("CI cell in narrow 3-col layout");
        assert!(!ci_narrow.ellipsized, "CI cell must not ellipsize in narrow 540pt layout: {ci_narrow:?}");
        let note_entry = meta.find_by_widget_id("result.cell.0.2").expect("note cell must be visible inside 540pt");
        assert!(note_entry.ellipsized, "narrow long secondary text must record ellipsized=true");
        assert_eq!(note_entry.full_label.as_deref(), Some(long_note));
    }

    #[test]
    fn r20_10_r20_13_underline_tabs_ghost_actions_subheader_and_sig_p_across_themes() {
        let (doc, rs_id) = make_nl_test_sheet();
        let mut app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            app.set_theme_mode(mode);
            let tokens = Tokens::for_mode(mode);
            let ctx = egui::Context::default();
            crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
            let mut rv = ResultsViewEguiState::default();
            let mut meta = FrameInspectMeta::new();
            let mut shapes = Vec::new();
            for _ in 0..2 {
                let mut intents = PendingUiIntents::new();
                meta = FrameInspectMeta::new();
                let mut out = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(840.0, 420.0))), ..Default::default() },
                    |ui| {
                        meta.begin_pass(ui.ctx());
                        let _ = render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv, &tokens, true, &mut meta, &mut intents);
                    },
                );
                out.textures_delta.clear();
                shapes = out.shapes;
            }
            let params = meta.find_by_widget_id("result.edit_params").expect("Parameters button");
            assert_eq!(params.label, "Parameters");
            assert!(!params.label.contains("Cmd+T"));
            assert!(params.value.is_none(), "Parameters button must not be selected at rest");
            let params_has_active_fill = shapes.iter().any(|cs| match &cs.shape {
                egui::Shape::Rect(r) => r.fill == tokens.active_bg && r.rect.intersects(params.clipped_rect),
                egui::Shape::Vec(v) => {
                    v.iter().any(|ch| matches!(ch, egui::Shape::Rect(r) if r.fill == tokens.active_bg && r.rect.intersects(params.clipped_rect)))
                }
                _ => false,
            });
            assert!(!params_has_active_fill, "Parameters ghost button must not paint active_bg pill fill at rest in {mode:?}");
            assert!(meta.find_by_widget_id("result.copy_selection").is_some());

            // Long active tab (>120pt) must NOT be crushed to the retired 96/120pt cap.
            let tab0 = meta.find_by_widget_id("result.tab.nonlinear_parameters").expect("tab 0");
            assert!(tab0.clipped_rect.width() > 140.0, "expected un-capped content-first tab width, got {}", tab0.clipped_rect.width());

            // Sub-header collapses on Inferential Report (no [Inferential Report] or NxM badge).
            assert!(meta.find_by_widget_id("result.new_graph_from_tab").is_none());
            assert!(collect_text_colors(&shapes, "Inferential Report").is_empty());

            // Significant P-value paints with tokens.text_strong, NEVER tokens.accent (Locked Decision #7).
            let p_cols = collect_text_colors(&shapes, "0.0002 ***");
            assert!(!p_cols.is_empty());
            assert!(p_cols.iter().all(|&c| c == tokens.text_strong && c != tokens.accent), "sig P-value used wrong color in {mode:?}: {p_cols:?}");

            // PlotReadyMatrix tab displays dot-free "New Graph from Tab".
            let mut intents = PendingUiIntents::new();
            rv.per_sheet_active_tab.insert(rs_id.clone(), 1);
            meta = FrameInspectMeta::new();
            let mut out =
                ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(840.0, 420.0))), ..Default::default() }, |ui| {
                    meta.begin_pass(ui.ctx());
                    let _ = render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv, &tokens, true, &mut meta, &mut intents);
                });
            out.textures_delta.clear();
            let new_g = meta.find_by_widget_id("result.new_graph_from_tab").expect("PlotReadyMatrix action");
            assert_eq!(new_g.label, "New Graph from Tab");
        }
    }

    #[test]
    fn r20_11_r20_20_pane_focused_gate_dimming_and_right_click_outside_selection() {
        let (doc, rs_id) = make_nl_test_sheet();
        let app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);

        let mut rv = ResultsViewEguiState {
            selection_anchor: Some((rs_id.clone(), 0, 0, 0)),
            selected_cell: Some((rs_id.clone(), 0, 1, 1)),
            ..Default::default()
        };

        // 1. Unfocused pane ignores Ctrl+PageDown and paints selection_unemphasized_bg.
        let mut meta = FrameInspectMeta::new();
        let mut intents = PendingUiIntents::new();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 420.0))),
                events: vec![egui::Event::Key { key: Key::PageDown, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL }],
                ..Default::default()
            },
            |ui| {
                meta.begin_pass(ui.ctx());
                let rep =
                    render_results_view_for_sheet_with_options(ui, &app, Some(&rs_id), &mut rv, &tokens, true, false, false, &mut meta, &mut intents);
                assert_eq!(rep.active_tab_index, 0, "unfocused pane must ignore Ctrl+PageDown");
            },
        );
        out.textures_delta.clear();
        let has_unemph = out.shapes.iter().any(|cs| match &cs.shape {
            egui::Shape::Rect(r) => r.fill == tokens.selection_unemphasized_bg,
            egui::Shape::Vec(v) => v.iter().any(|ch| matches!(ch, egui::Shape::Rect(r) if r.fill == tokens.selection_unemphasized_bg)),
            _ => false,
        });
        assert!(has_unemph, "unfocused Results selection must paint tokens.selection_unemphasized_bg");

        // 2. Right-click outside (0..=1, 0..=1) at (2, 1) immediately moves selection to (2, 1).
        let cell_2_1 = meta.find_by_widget_id("result.cell.2.1").expect("cell (2,1)").clipped_rect.center();
        let mut intents2 = PendingUiIntents::new();
        let mut out2 = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 420.0))),
                events: vec![
                    egui::Event::PointerMoved(cell_2_1),
                    egui::Event::PointerButton { pos: cell_2_1, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
                    egui::Event::PointerButton { pos: cell_2_1, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
                ],
                ..Default::default()
            },
            |ui| {
                meta = FrameInspectMeta::new();
                meta.begin_pass(ui.ctx());
                let _ =
                    render_results_view_for_sheet_with_options(ui, &app, Some(&rs_id), &mut rv, &tokens, true, true, false, &mut meta, &mut intents2);
            },
        );
        out2.textures_delta.clear();
        assert_eq!(rv.selected_range(&rs_id, 0), Some((2, 2, 1, 1)), "right-click outside selection must move selection to clicked cell");
        assert!(intents2.intents().iter().any(|i| matches!(i, UiIntent::SyncResultSelection { anchor: (2, 1), focus: (2, 1), .. })));
    }

    #[test]
    fn r20_6_ambiguous_nl_banner_renders_once_and_preserves_ext01() {
        let (mut doc, rs_id) = make_nl_test_sheet();
        doc.result_sheets[0].checklist.push(ChecklistItem {
            assumption: String::from("Parameter identifiability (Ambiguous)"),
            status: ChecklistStatus::Violated,
            diagnostic_metric: Some(String::from("2 of 3 datasets (VEHICLE, COMPOUND_A)")),
            explanation: String::from("Ambiguous / unbounded fits in 2 of 3 datasets (VEHICLE, COMPOUND_A). Profile likelihood open."),
            remediation: None,
        });
        doc.result_sheets[0].checklist.push(ChecklistItem {
            assumption: String::from("X-axis scale matches log10(X) equation"),
            status: ChecklistStatus::Warn,
            diagnostic_metric: None,
            explanation: String::from("X values span positive non-log concentration scale."),
            remediation: None,
        });
        let app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);
        let mut rv = ResultsViewEguiState::default();
        let mut meta = FrameInspectMeta::new();
        let mut intents = PendingUiIntents::new();
        let mut out =
            ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 480.0))), ..Default::default() }, |ui| {
                meta.begin_pass(ui.ctx());
                let _ = render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv, &tokens, true, &mut meta, &mut intents);
            });
        out.textures_delta.clear();
        assert!(meta.find_by_widget_id("result.ambiguity_callout").is_none(), "NL sheet must not render legacy top ambiguity_callout");
        let banner = meta.find_by_widget_id("result.diagnostic_banner").expect("single unified NL banner");
        assert!(banner.label.contains("VEHICLE") && banner.label.contains("2 of 3 datasets"));
        assert!(banner.label.contains("X-axis scale matches log10(X) equation"));
        assert!(meta.find_by_widget_id("result.diagnostic_banner.constraints").is_some());
        assert!(meta.find_by_widget_id("result.diagnostic_banner.compare_horizontal").is_some());
    }

    #[test]
    fn r20_21_results_diagnostic_headers_precision_and_signed_hougaard_tooltips() {
        let mut tab = ResultTab {
            id: String::from("nonlinear_parameters"),
            title: String::from("Parameters"),
            kind: ResultTabKind::ParametersAndConfidenceIntervals,
            column_headers: vec![String::from("Best-fit value"), String::from("Dependency"), String::from("Hougaard |g1|"), String::from("Value")],
            row_labels: vec![String::from("EC50"), String::from("Sy.x")],
            rows: vec![
                vec![
                    ResultCell::Number { value: 1.23456789012, prefix: None, units: None },
                    ResultCell::Number { value: 0.9951, prefix: None, units: None },
                    ResultCell::Number { value: 1.42, prefix: None, units: None },
                    ResultCell::Number { value: 4.0, prefix: None, units: None },
                ],
                vec![ResultCell::Number { value: 0.0456789123, prefix: None, units: None }, ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
            ],
            footnotes: Vec::new(),
        };
        let mut rs = ResultSheet::new(
            AnalysisSpec::new("res-diag", "ds-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl"))).unwrap(),
        )
        .unwrap();
        rs.status = ResultStatus::UpToDate;
        rs.tabs.clear();
        rs.tabs.push(tab.clone());
        rs.cell_details.push(curvebook_doc::ResultCellDetail {
            tab_id: tab.id.clone(),
            row: 0,
            col: 2,
            raw_value: Some(-1.42),
            detail: String::from("Signed g1 = -1.420000 (right/left asymmetric curvature)"),
        });
        let _ = &mut tab;

        let mut doc = ProjectDocument::default();
        doc.result_sheets.push(rs);
        let rs_id = SheetId::new("res-diag");
        let app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| tokens.configure_style(s));
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);

        let mut rv = ResultsViewEguiState::default();

        fn collect_shape_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for cs in shapes {
                collect_shape_rec(&cs.shape, out);
            }
        }
        fn collect_shape_rec(s: &egui::epaint::Shape, out: &mut Vec<String>) {
            match s {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Vec(v) => {
                    for ch in v {
                        collect_shape_rec(ch, out);
                    }
                }
                _ => {}
            }
        }

        let mut run_pass = |events: Vec<egui::Event>, time: f64| -> (Vec<String>, Rect, Rect, Rect, Rect, Rect) {
            let mut r_hdr = Rect::NOTHING;
            let mut r_prec = Rect::NOTHING;
            let mut r_g1 = Rect::NOTHING;
            let mut r_exact = Rect::NOTHING;
            let mut r_syx = Rect::NOTHING;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 460.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut meta = FrameInspectMeta::new();
                    let mut intents = PendingUiIntents::new();
                    meta.begin_pass(ui.ctx());
                    let _ = render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv, &tokens, true, &mut meta, &mut intents);
                    if let Some(c0) = meta.find_by_widget_id("result.cell.0.0") {
                        r_prec = c0.clipped_rect;
                    }
                    if let Some(c2) = meta.find_by_widget_id("result.cell.0.2") {
                        r_g1 = c2.clipped_rect;
                        r_hdr = Rect::from_min_max(
                            pos2(c2.clipped_rect.left() + 4.0, c2.clipped_rect.top() - 20.0),
                            pos2(c2.clipped_rect.right() - 4.0, c2.clipped_rect.top() - 4.0),
                        );
                    }
                    if let Some(c3) = meta.find_by_widget_id("result.cell.0.3") {
                        r_exact = c3.clipped_rect;
                    }
                    if let Some(c10) = meta.find_by_widget_id("result.cell.1.0") {
                        r_syx = Rect::from_min_max(
                            pos2(16.0, c10.clipped_rect.top() + 2.0),
                            pos2(c10.clipped_rect.left() - 8.0, c10.clipped_rect.bottom() - 2.0),
                        );
                    }
                },
            );
            out.textures_delta.clear();
            let mut texts = Vec::new();
            collect_shape_texts(&out.shapes, &mut texts);
            (texts, r_hdr, r_prec, r_g1, r_exact, r_syx)
        };

        let (_, hdr_hougaard_rect, cell_prec_rect, cell_g1_rect, cell_exact_rect, row_syx_rect) = run_pass(Vec::new(), 0.0);
        // 1. Hover "Hougaard |g1|" header.
        let _ = run_pass(vec![egui::Event::PointerMoved(hdr_hougaard_rect.center())], 1.0);
        let _ = run_pass(Vec::new(), 1.50);
        let (hdr_tips, _, _, _, _, _) = run_pass(Vec::new(), 1.55);
        assert!(hdr_tips.iter().any(|t| t.contains("< 0.1") && t.contains("> 1.0")), "Hougaard header tooltip missing thresholds: {hdr_tips:?}");

        // 2. Hover "Sy.x" GoF row stub.
        let _ = run_pass(vec![egui::Event::PointerMoved(row_syx_rect.center())], 2.0);
        let _ = run_pass(Vec::new(), 2.50);
        let (syx_tips, _, _, _, _, _) = run_pass(Vec::new(), 2.55);
        assert!(syx_tips.iter().any(|t| t.to_ascii_lowercase().contains("residual standard deviation")), "Sy.x tooltip missing: {syx_tips:?}");

        // 3. Hover rounded numeric cell 1.23456789012.
        let _ = run_pass(vec![egui::Event::PointerMoved(cell_prec_rect.center())], 3.0);
        let _ = run_pass(Vec::new(), 3.50);
        let (prec_tips, _, _, _, _, _) = run_pass(Vec::new(), 3.55);
        assert!(prec_tips.iter().any(|t| t.contains("1.23456789012")), "rounded cell tooltip missing full precision: {prec_tips:?}");

        // 4. Hover Hougaard |g1| cell -> signed g1 AND > 1.0 skew band.
        let _ = run_pass(vec![egui::Event::PointerMoved(cell_g1_rect.center())], 4.0);
        let _ = run_pass(Vec::new(), 4.50);
        let (g1_tips, _, _, _, _, _) = run_pass(Vec::new(), 4.55);
        assert!(g1_tips.iter().any(|t| t.contains("-1.42") && t.contains("|g1| > 1.0")), "Hougaard cell missing signed g1 + band: {g1_tips:?}");

        // 5. Hover exact unrounded non-diagnostic integer cell "4" -> no full-precision echo popup.
        let _ = run_pass(vec![egui::Event::PointerMoved(cell_exact_rect.center())], 5.0);
        let _ = run_pass(Vec::new(), 5.50);
        let (exact_tips, _, _, _, _, _) = run_pass(Vec::new(), 5.55);
        assert!(!exact_tips.iter().any(|t| t.contains("Full precision")), "exact integer must not show Full precision: {exact_tips:?}");
    }
}
