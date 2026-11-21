//! Native `egui` 7-context Trailing Studio Dock (`280 pt -> 36 pt`,
//! `docs/planning/gui-layout-spec.md` §6, `R15.3`).
//!
//! Deterministically resolves and renders all seven context-sensitive Studio surfaces:
//! 1. `MultipleVariablesFormula`: MV Variable & Formula Studio (with live colored operand highlights)
//! 2. `TableSelection`: Structured Table & Selection Studio (decimals, linked provenance, subset exclusion/styling)
//! 3. `MetadataHooks`: Metadata & Hooks Studio (`InfoSheet` typed constant editor + downstream `Referenced By` jump list)
//! 4. `ResultsInterpret`: Results Interpret & Diagnostics (`Assumptions`, diagnostics-only `Guidance`, copyable `Methods`)
//! 5. `GraphInspector`: 5-Tab Graph Inspector (`Data`, `Axes`, `Legends`, `Annotations`, `Canvas` + `More…` modal triggers)
//! 6. `DataPointDrillBack`: Data Point Drill-Back (datum coords, row/col coords, exclude toggle, `Jump to Datasheet Cell`)
//! 7. `PageLayoutComposer`: Page Layout Composer (`85 mm` / `114 mm` / `174 mm` presets, margins, DPI, snap, alignment)

mod drill_back;
mod graph_inspector;
mod helpers;
mod metadata_hooks;
mod mv_formula;
mod page_layout;
mod results_interpret;
mod table_selection;

use curvebook_doc::{SheetId, TableArchetype};
use egui::{Color32, Pos2, Sense, Stroke, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, widgets};
use crate::i18n::is_rtl_locale;
use crate::inspect::ZLayer;

pub use metadata_hooks::collect_info_downstream_references;
pub use mv_formula::{resolve_formula_operand_highlights, studio_formula_signature};

use drill_back::render_data_point_drill_back_studio;
use graph_inspector::render_graph_inspector_studio;
use helpers::{SCROLL_BAR_RESERVE_PT, STUDIO_INNER_PAD, StudioChipSpec, render_collapsed_rail, studio_chip, surface_localized_title};
use metadata_hooks::render_metadata_hooks_studio;
use mv_formula::render_mv_formula_studio;
use page_layout::render_page_layout_composer_studio;
use results_interpret::render_results_interpret_studio;
use table_selection::render_table_selection_studio;

/// Seven automatic Trailing Studio surfaces defined in `docs/planning/gui-layout-spec.md` §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StudioContextSurface {
    MultipleVariablesFormula,
    #[default]
    TableSelection,
    MetadataHooks,
    ResultsInterpret,
    GraphInspector,
    DataPointDrillBack,
    PageLayoutComposer,
}

impl StudioContextSurface {
    /// Stable slug identifying this surface in telemetry and collapsed rail buttons.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::MultipleVariablesFormula => "mv_formula",
            Self::TableSelection => "table_selection",
            Self::MetadataHooks => "metadata_hooks",
            Self::ResultsInterpret => "results_interpret",
            Self::GraphInspector => "graph_inspector",
            Self::DataPointDrillBack => "data_point_drill_back",
            Self::PageLayoutComposer => "page_layout_composer",
        }
    }

    /// Compact 1-2 glyph badge rendered inside the collapsed `36 pt` icon strip.
    #[must_use]
    pub const fn rail_glyph(self) -> &'static str {
        match self {
            Self::MultipleVariablesFormula => "fx",
            Self::TableSelection => "⊞",
            Self::MetadataHooks => "ⓘ",
            Self::ResultsInterpret => "✓",
            Self::GraphInspector => "◈",
            Self::DataPointDrillBack => "⌖",
            Self::PageLayoutComposer => "▤",
        }
    }
}

/// Active tab inside [`StudioContextSurface::ResultsInterpret`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResultsStudioTab {
    #[default]
    Assumptions,
    Guidance,
    Methods,
}

/// Active tab inside [`StudioContextSurface::GraphInspector`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphStudioTab {
    #[default]
    Data,
    Axes,
    Legends,
    Annotations,
    Canvas,
}

/// Live colored outline highlight descriptor associating a formula operand with its column color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MvOperandHighlight {
    pub variable_index: usize,
    pub variable_title: String,
    pub color_rgb: [u8; 3],
    pub color_hex: String,
}

impl MvOperandHighlight {
    /// Convert stored RGB triplet into an `egui::Color32`.
    #[must_use]
    pub const fn color32(&self) -> Color32 {
        Color32::from_rgb(self.color_rgb[0], self.color_rgb[1], self.color_rgb[2])
    }
}

/// Cached plotted-point summary used by `DataPointDrillBack` without re-compiling scenes every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrillBackPointEntry {
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
    pub series_label: String,
    pub value_summary: String,
    pub x_pt_tenths: i64,
    pub y_pt_tenths: i64,
}

/// Downstream reference item shown in `MetadataHooks`'s `Referenced By` jump list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoDownstreamRef {
    pub target_sheet_id: SheetId,
    pub target_title: String,
    pub section_tag: &'static str,
    pub binding_summary: String,
}

/// Persistent `egui_shell` UI state for the Trailing Studio Dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailingStudioEguiState {
    pub results_tab: ResultsStudioTab,
    pub graph_tab: GraphStudioTab,
    pub mv_formula_draft: String,
    pub selected_subset_rows_summary: String,
    pub last_copied_methods: bool,
    pub mv_selected_col: usize,
    pub mv_sub_tab: usize,
    pub mv_var_filter: String,
    pub mv_fn_filter: String,
    pub mv_selected_fn: String,
    pub mv_title_draft: String,
    pub mv_recode_from: String,
    pub mv_recode_to: String,
    pub mv_operand_highlights: Vec<MvOperandHighlight>,
    pub mv_synced_sheet: Option<SheetId>,
    pub mv_synced_col: Option<usize>,
    pub table_decimals: Option<u8>,
    pub info_selected_constant: Option<String>,
    pub info_name_draft: String,
    pub info_value_draft: String,
    pub info_units_draft: String,
    pub info_kind_draft: u8,
    pub info_synced_key: Option<(SheetId, String)>,
    pub results_severity_flagged_only: bool,
    pub methods_format: String,
    pub last_copied_methods_text: String,
    pub methods_cache_key: Option<(SheetId, u64, String)>,
    pub methods_cached_text: String,
    pub graph_series_index: usize,
    pub graph_axis_target: u8,
    pub drill_cache_key: Option<(SheetId, u64)>,
    pub drill_cached_points: Vec<DrillBackPointEntry>,
    pub layout_margin_mm: u16,
    pub layout_dpi: u16,
    pub snap_enabled: bool,
}

impl Default for TrailingStudioEguiState {
    fn default() -> Self {
        Self {
            results_tab: ResultsStudioTab::default(),
            graph_tab: GraphStudioTab::default(),
            mv_formula_draft: String::new(),
            selected_subset_rows_summary: String::new(),
            last_copied_methods: false,
            mv_selected_col: 0,
            mv_sub_tab: 0,
            mv_var_filter: String::new(),
            mv_fn_filter: String::new(),
            mv_selected_fn: String::from("ln"),
            mv_title_draft: String::new(),
            mv_recode_from: String::new(),
            mv_recode_to: String::new(),
            mv_operand_highlights: Vec::new(),
            mv_synced_sheet: None,
            mv_synced_col: None,
            table_decimals: None,
            info_selected_constant: None,
            info_name_draft: String::new(),
            info_value_draft: String::new(),
            info_units_draft: String::new(),
            info_kind_draft: 0,
            info_synced_key: None,
            results_severity_flagged_only: false,
            methods_format: String::from("markdown"),
            last_copied_methods_text: String::new(),
            methods_cache_key: None,
            methods_cached_text: String::new(),
            graph_series_index: 0,
            graph_axis_target: 1,
            drill_cache_key: None,
            drill_cached_points: Vec::new(),
            layout_margin_mm: 8,
            layout_dpi: 300,
            snap_enabled: true,
        }
    }
}

/// Input context bundle for [`render_trailing_studio`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct TrailingStudioContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub enabled: bool,
    pub focused: bool,
}

/// Per-frame geometry & active-surface report emitted by [`render_trailing_studio`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrailingStudioFrameReport {
    pub dock_rect_pt: Option<egui::Rect>,
    pub surface: StudioContextSurface,
    pub collapsed_to_rail: bool,
    pub operand_highlight_count: usize,
    pub referenced_by_count: usize,
}

/// Resolve the active [`StudioContextSurface`] strictly from live document sheet & brushing state.
#[must_use]
pub fn resolve_studio_surface(app: &UiApp) -> StudioContextSurface {
    let doc = app.document();
    let Some(active) = doc.active_sheet_id.as_ref() else {
        return StudioContextSurface::TableSelection;
    };
    if doc.layout_sheet(active).is_some() {
        return StudioContextSurface::PageLayoutComposer;
    }
    if doc.graph_sheet(active).is_some() {
        return if app.brushed_cell.is_some() { StudioContextSurface::DataPointDrillBack } else { StudioContextSurface::GraphInspector };
    }
    if doc.result_sheet(active).is_some() {
        return StudioContextSurface::ResultsInterpret;
    }
    if doc.info_sheet(active).is_some() {
        return StudioContextSurface::MetadataHooks;
    }
    if let Some(ds) = doc.data_sheet(active)
        && matches!(ds.archetype, TableArchetype::MultipleVariables(_))
    {
        return StudioContextSurface::MultipleVariablesFormula;
    }
    StudioContextSurface::TableSelection
}

/// Render the `280 pt -> 36 pt` 7-context Trailing Studio Dock.
pub fn render_trailing_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> TrailingStudioFrameReport {
    let surface = resolve_studio_surface(app);
    let mut report = TrailingStudioFrameReport {
        dock_rect_pt: None,
        surface,
        collapsed_to_rail: ctx_in.layout.trailing_collapsed_to_rail,
        operand_highlight_count: 0,
        referenced_by_count: 0,
    };

    if ctx_in.enabled && !ui.ctx().egui_wants_keyboard_input() {
        let shift_cmd = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let shift_ctrl = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        let shift_mac = egui::Modifiers::MAC_CMD | egui::Modifiers::SHIFT;
        let toggle_shortcut = ui.ctx().input_mut(|i| {
            i.consume_key(shift_cmd, egui::Key::I) || i.consume_key(shift_ctrl, egui::Key::I) || i.consume_key(shift_mac, egui::Key::I)
        });
        if toggle_shortcut {
            intents.push_toggle_trailing_collapsed_rail();
        }
    }

    if !ctx_in.layout.trailing_visible || ctx_in.layout.trailing_dock_w_pt <= 0.5 {
        return report;
    }

    let w = ctx_in.layout.trailing_dock_w_pt;
    let dock_frame = egui::Frame::NONE
        .fill(ctx_in.tokens.panel_bg)
        .stroke(Stroke::new(1.0, if ctx_in.focused { ctx_in.tokens.focus_ring } else { ctx_in.tokens.border }))
        .inner_margin(egui::Margin::same(if ctx_in.layout.trailing_collapsed_to_rail { 4 } else { 6 }));

    let doc = app.document();
    let active_id = doc.active_sheet_id.as_ref();

    let resp = egui::Panel::right("egui_shell.trailing_studio")
        .exact_size(w)
        .resizable(false)
        .frame(dock_frame)
        .show(ui, |ui| {
            if ctx_in.layout.trailing_collapsed_to_rail {
                render_collapsed_rail(ui, ctx_in.tokens, surface, ctx_in.enabled, meta, intents);
                return;
            }

            // Expanded Studio Header
            ui.horizontal(|ui| {
                let title = surface_localized_title(ctx_in.tokens.locale, surface);
                let title_budget = (ui.available_width() - 32.0).max(40.0);
                let mut job = egui::text::LayoutJob::simple_singleline(
                    title.to_owned(),
                    egui::FontId::proportional(ctx_in.tokens.metrics.font_ui_title_pt),
                    ctx_in.tokens.text_strong,
                );
                job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
                job.wrap.max_width = title_budget;
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let elided = galley.elided;
                let (t_rect, t_resp) = ui.allocate_exact_size(Vec2::new(title_budget, 24.0), Sense::hover());
                if ui.is_rect_visible(t_rect) {
                    let tp = Pos2::new(t_rect.min.x, t_rect.center().y - galley.size().y * 0.5);
                    let painter = ui.painter().with_clip_rect(t_rect);
                    painter.galley(tp, galley, ctx_in.tokens.text_strong);
                }
                let acc_title = title.to_owned();
                t_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Heading, true, acc_title.clone()));
                let t_resp = if elided { t_resp.on_hover_text(title) } else { t_resp };
                if t_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
                    meta.record_response(
                        ui,
                        &t_resp,
                        FrameInspectEntryInput {
                            widget_id: "inspector.surface_title",
                            role: "heading",
                            label: title,
                            command_id: None,
                            value: Some(json!(surface.slug())),
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::Workspace,
                            ellipsized: elided,
                            full_label: elided.then_some(title),
                        },
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let c_resp = studio_chip(
                        ui,
                        ctx_in.tokens,
                        meta,
                        &StudioChipSpec {
                            label: if is_rtl_locale(ctx_in.tokens.locale) { "«" } else { "»" },
                            tooltip: "Collapse Studio (Cmd/Ctrl+Shift+I)",
                            max_w: 24.0,
                            selected: false,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: "inspector.collapse_rail",
                            role: "button",
                            command_id: Some("ui.inspector.toggle"),
                            disabled_reason: None,
                        },
                    );
                    if c_resp {
                        intents.push_toggle_trailing_collapsed_rail();
                    }
                });
            });

            ui.add_space(2.0);
            widgets::divider(ui);
            ui.add_space(4.0);

            // Reserve dedicated right gutter inside ScrollArea so `Role::ScrollBar` NEVER overlaps any widget.
            ui.spacing_mut().scroll.floating = false;
            let content_max_w = (ui.available_width() - SCROLL_BAR_RESERVE_PT).max(120.0);
            egui::ScrollArea::vertical().id_salt("egui_shell.trailing_studio.scroll").auto_shrink([false, false]).show(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { right: STUDIO_INNER_PAD as i8, ..Default::default() }).show(ui, |ui| {
                    ui.set_max_width(content_max_w);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    match surface {
                        StudioContextSurface::MultipleVariablesFormula => {
                            if let Some(ds) = active_id.and_then(|id| doc.data_sheet(id))
                                && let TableArchetype::MultipleVariables(mv) = &ds.archetype
                            {
                                render_mv_formula_studio(ui, ds, mv, state, ctx_in, meta, intents);
                                report.operand_highlight_count = state.mv_operand_highlights.len();
                            }
                        }
                        StudioContextSurface::TableSelection => {
                            let ds = active_id.and_then(|id| doc.data_sheet(id));
                            render_table_selection_studio(ui, app, ds, state, ctx_in, meta, intents);
                        }
                        StudioContextSurface::MetadataHooks => {
                            if let Some(info) = active_id.and_then(|id| doc.info_sheet(id)) {
                                report.referenced_by_count = render_metadata_hooks_studio(ui, app, info, state, ctx_in, meta, intents);
                            }
                        }
                        StudioContextSurface::ResultsInterpret => {
                            if let Some(rs) = active_id.and_then(|id| doc.result_sheet(id)) {
                                render_results_interpret_studio(ui, app, rs, state, ctx_in, meta, intents);
                            }
                        }
                        StudioContextSurface::GraphInspector => {
                            if let Some(gs) = active_id.and_then(|id| doc.graph_sheet(id)) {
                                render_graph_inspector_studio(ui, app, gs, state, ctx_in, meta, intents);
                            }
                        }
                        StudioContextSurface::DataPointDrillBack => {
                            if let Some(gs) = active_id.and_then(|id| doc.graph_sheet(id)) {
                                render_data_point_drill_back_studio(ui, app, gs, state, ctx_in, meta, intents);
                            }
                        }
                        StudioContextSurface::PageLayoutComposer => {
                            if let Some(ls) = active_id.and_then(|id| doc.layout_sheet(id)) {
                                render_page_layout_composer_studio(ui, app, ls, state, ctx_in, meta, intents);
                            }
                        }
                    }
                });
            });
        })
        .response
        .rect;

    report.dock_rect_pt = Some(resp);
    if !ctx_in.layout.trailing_collapsed_to_rail && ctx_in.layout.trailing_visible && ctx_in.layout.trailing_splitter_w_pt > 0.5 {
        render_trailing_dock_splitter(ui, app, ctx_in, meta, intents);
    }
    report
}

fn render_trailing_dock_splitter(
    ui: &mut egui::Ui,
    app: &UiApp,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let metrics = tokens.metrics;
    let splitter_w = ctx_in.layout.trailing_splitter_w_pt;
    egui::Panel::right("egui_shell.trailing_dock_splitter")
        .exact_size(splitter_w)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(tokens.panel_bg))
        .show(ui, |ui| {
            let split_rect = ui.available_rect_before_wrap();
            let _ = ui.allocate_rect(split_rect, Sense::hover());
            let slop = ((metrics.splitter_hit_slop_w - split_rect.width()) * 0.5).max(0.0);
            let hit_rect = split_rect.expand2(Vec2::new(slop, 0.0)).intersect(meta.viewport_rect);
            ui.set_clip_rect(hit_rect);
            let enabled = ctx_in.enabled;
            let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
            let resp = ui.interact(hit_rect, ui.scope_id().with("inspector_dock_splitter"), sense);
            resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Splitter, enabled, "Inspector Dock Splitter"));
            if enabled && (resp.hovered() || resp.dragged()) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            let line_col = if enabled && (resp.hovered() || resp.dragged()) { tokens.accent } else { tokens.border };
            ui.painter().vline(split_rect.center().x, split_rect.y_range(), Stroke::new(1.0, line_col));

            if enabled {
                if resp.double_clicked() {
                    let default_w = app.egui_views.workspace_preset.default_dock_config().trailing_width_pt;
                    intents.push_sync_trailing_width(default_w);
                } else if resp.dragged() {
                    let delta_w = -resp.drag_delta().x;
                    if delta_w.abs() > 0.01 {
                        let vw = ui.ctx().content_rect().width().max(1.0);
                        let target_center_min = if !ctx_in.layout.leading_collapsed_to_rail
                            && !ctx_in.layout.trailing_collapsed_to_rail
                            && vw >= metrics.inspector_collapse_breakpoint_w
                        {
                            480.0
                        } else {
                            metrics.center_stage_min_w
                        };
                        let max_for_center = (vw
                            - ctx_in.layout.leading_dock_w_pt
                            - ctx_in.layout.leading_splitter_w_pt
                            - ctx_in.layout.trailing_splitter_w_pt
                            - target_center_min)
                            .max(metrics.inspector_min_w);
                        let new_w =
                            (ctx_in.layout.trailing_dock_w_pt + delta_w).clamp(metrics.inspector_min_w, metrics.inspector_max_w.min(max_for_center));
                        intents.push_sync_trailing_width(new_w);
                    }
                }
            }

            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.dock_splitter",
                    role: "splitter",
                    label: "Inspector Dock Splitter",
                    command_id: None,
                    value: Some(json!({ "width_pt": ctx_in.layout.trailing_dock_w_pt })),
                    enabled,
                    disabled_reason: (!enabled).then_some("modal dialog is active"),
                    z_layer: ZLayer::SplitHandle,
                    ellipsized: false,
                    full_label: None,
                },
            );
        });
}
