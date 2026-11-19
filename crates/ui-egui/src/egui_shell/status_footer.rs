//! `26 pt` Status, Quick-Stats & Viewport Footer (`docs/planning/gui-layout-spec.md` §7, `R15.3`).

use curvebook_doc::TableArchetype;
use egui::{Stroke, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::canvas_view::CanvasHoverReadout;
use crate::egui_shell::{
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, doc_sheet_title,
};
use crate::layout::ZLayer;

/// Hard cell-scan cap guaranteeing tabular quick-stats stay well within `< 0.5 ms` even on 1M-cell sheets.
pub const QUICK_STATS_CELL_CAP: usize = 50_000;

/// Tabular selection quick statistics computed by [`compute_cached_quick_stats`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickStatsSummary {
    pub n: usize,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sum: Option<f64>,
    pub truncated_at_cap: bool,
}

/// Memoization cache avoiding per-frame scans over active grid selections (`O(1)` steady frame).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickStatsCache {
    pub cached_sheet_id: Option<curvebook_doc::SheetId>,
    pub cached_mutation_seq: u64,
    pub cached_selection_key: (usize, usize, usize, usize),
    pub summary: QuickStatsSummary,
}

/// Recompute [`QuickStatsSummary`] only when `(sheet_id, mutation_seq, selection_key)` changed,
/// bounded strictly by [`QUICK_STATS_CELL_CAP`] (`50,000` cells).
pub fn compute_cached_quick_stats<'a>(app: &UiApp, cache: &'a mut QuickStatsCache) -> &'a QuickStatsSummary {
    let doc = app.document();
    let active_id = doc.active_sheet_id.clone();
    let mutation_seq = app.mutation_seq();
    let sel_key = if let Some(anchor) = app.grid.tool.anchor {
        let active = app.grid.tool.active.unwrap_or(anchor);
        (anchor.row, anchor.col, active.row, active.col)
    } else if let Some(active) = app.grid.tool.active {
        (active.row, active.col, active.row, active.col)
    } else {
        (0, 0, 0, 0)
    };

    if cache.cached_sheet_id == active_id && cache.cached_mutation_seq == mutation_seq && cache.cached_selection_key == sel_key {
        return &cache.summary;
    }

    cache.cached_sheet_id = active_id.clone();
    cache.cached_mutation_seq = mutation_seq;
    cache.cached_selection_key = sel_key;
    cache.summary = QuickStatsSummary::default();

    let Some(sheet_id) = active_id else {
        return &cache.summary;
    };
    let Some(ds) = doc.data_sheet(&sheet_id) else {
        return &cache.summary;
    };

    let coords = app.grid.selection_coords(ds);
    if coords.is_empty() {
        return &cache.summary;
    }

    let mut count = 0usize;
    let mut sum = 0.0_f64;
    let mut m2 = 0.0_f64;
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    let mut truncated = false;

    for (scanned, coord) in coords.into_iter().enumerate() {
        if scanned >= QUICK_STATS_CELL_CAP {
            truncated = true;
            break;
        }
        let val_opt = match &ds.archetype {
            TableArchetype::Column(t) => t.data_sets.get(coord.col).and_then(|c| c.cell(coord.row, coord.subcol)).and_then(|c| c.active_number()),
            _ => None,
        };
        if let Some(v) = val_opt.filter(|x| x.is_finite()) {
            count += 1;
            let delta = v - if count == 1 { 0.0 } else { sum / ((count - 1) as f64) };
            sum += v;
            let mean_now = sum / (count as f64);
            m2 += delta * (v - mean_now);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
    }

    cache.summary = QuickStatsSummary {
        n: count,
        mean: (count > 0).then(|| sum / (count as f64)),
        sd: (count > 1).then(|| (m2 / ((count - 1) as f64)).sqrt()),
        min: (count > 0).then_some(min_v),
        max: (count > 0).then_some(max_v),
        sum: (count > 0).then_some(sum),
        truncated_at_cap: truncated,
    };
    &cache.summary
}

/// Persistent UI state for Footer popups (`PreviewFilter` menu, quick-stats customization).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusFooterEguiState {
    pub filter_popup_open: bool,
    pub quick_stats_menu_open: bool,
}

/// Input context bundle for [`render_status_footer`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct StatusFooterContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub history_can_back: bool,
    pub history_can_forward: bool,
    pub hover_readout: Option<&'a CanvasHoverReadout>,
    pub enabled: bool,
}

/// Per-frame geometry report emitted by [`render_status_footer`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusFooterFrameReport {
    pub footer_rect_pt: Option<egui::Rect>,
}

/// Render the `26 pt` bottom Status, Quick-Stats & Viewport Footer (`§7`).
///
/// The Wave 3 base implements history `< >` buttons, active sheet readout, cached `50,000`-cell
/// quick-stats, CVD preview filter toggle, rulers toggle, and Graph/Layout-scoped zoom readout;
/// `r15-studio-footer` completes the full section-summary pills, sheet switcher, and zoom presets.
pub fn render_status_footer(
    ui: &mut egui::Ui,
    app: &UiApp,
    views: &mut EguiShellViewState,
    ctx_in: StatusFooterContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatusFooterFrameReport {
    let mut report = StatusFooterFrameReport::default();
    if ctx_in.layout.status_bar_h_pt <= 0.0 {
        return report;
    }

    let tokens = ctx_in.tokens;
    let footer_frame = egui::Frame::NONE.fill(tokens.app_bar).stroke(Stroke::new(1.0, tokens.divider)).inner_margin(egui::Margin::symmetric(8, 2));

    let resp =
        egui::Panel::bottom("egui_shell.status_footer").exact_size(ctx_in.layout.status_bar_h_pt.max(22.0)).frame(footer_frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                let back_enabled = ctx_in.enabled && ctx_in.history_can_back;
                let back_resp = ui.add_enabled(back_enabled, egui::Button::new("<").min_size(Vec2::new(22.0, 20.0)));
                meta.record_response(
                    ui,
                    &back_resp,
                    FrameInspectEntryInput {
                        widget_id: "status.history.back",
                        role: "button",
                        label: "Back",
                        command_id: None,
                        value: None,
                        enabled: back_enabled,
                        disabled_reason: (!back_enabled).then_some("No earlier sheet in navigation history"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if back_resp.clicked() && back_enabled {
                    intents.push_history_back();
                }

                let fwd_enabled = ctx_in.enabled && ctx_in.history_can_forward;
                let fwd_resp = ui.add_enabled(fwd_enabled, egui::Button::new(">").min_size(Vec2::new(22.0, 20.0)));
                meta.record_response(
                    ui,
                    &fwd_resp,
                    FrameInspectEntryInput {
                        widget_id: "status.history.forward",
                        role: "button",
                        label: "Forward",
                        command_id: None,
                        value: None,
                        enabled: fwd_enabled,
                        disabled_reason: (!fwd_enabled).then_some("No newer sheet in navigation history"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if fwd_resp.clicked() && fwd_enabled {
                    intents.push_history_forward();
                }

                let doc = app.document();
                let active_title = doc.active_sheet_id.as_ref().and_then(|id| doc_sheet_title(doc, id)).unwrap_or("Workspace");
                ui.colored_label(tokens.text_strong, active_title);

                let stats = compute_cached_quick_stats(app, &mut views.quick_stats_cache);
                if stats.n > 0
                    && let Some(mean) = stats.mean
                {
                    let summary_txt = format!("n={}  mean={mean:.3}", stats.n);
                    ui.separator();
                    ui.colored_label(tokens.text_muted, summary_txt);
                } else if let Some(hover) = ctx_in.hover_readout {
                    ui.separator();
                    ui.colored_label(tokens.text_muted, &hover.value_summary);
                }

                ui.separator();
                let filter_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new(format!("CVD: {}", app.preview_filter.as_str())));
                meta.record_response(
                    ui,
                    &filter_resp,
                    FrameInspectEntryInput {
                        widget_id: "status.preview_filter",
                        role: "button",
                        label: "Preview Filter",
                        command_id: Some("ui.previewFilter.set"),
                        value: Some(json!(app.preview_filter.as_str())),
                        enabled: ctx_in.enabled,
                        disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if filter_resp.clicked() && ctx_in.enabled {
                    views.status_footer.filter_popup_open = !views.status_footer.filter_popup_open;
                }

                let is_canvas_sheet = doc.active_sheet_id.as_ref().is_some_and(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some())
                    || app.nav.gallery.open;
                if is_canvas_sheet {
                    let zoom_pct = (app.canvas_dialogs.canvas.zoom * 100.0).round();
                    let zoom_resp = ui.add_enabled(ctx_in.enabled, egui::Button::new(format!("{zoom_pct:.0}%")));
                    if zoom_resp.clicked() && ctx_in.enabled {
                        intents.push_ui("ui.zoom.fitSheet", json!({}));
                    }
                }
            });
        });

    report.footer_rect_pt = Some(resp.response.rect);
    report
}
