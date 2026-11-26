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
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, UiIntent};
use crate::i18n::is_rtl_locale;
use crate::inspect::ZLayer;

pub use metadata_hooks::collect_info_downstream_references;
pub use mv_formula::{resolve_formula_operand_highlights, studio_formula_signature};
pub use page_layout::arrange_layout_grid_items;

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
    pub source_sheet_id: SheetId,
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

const STUDIO_SNAP_ENABLED_KEY: &str = "curvebook.trailing_studio.snap_enabled";
const STUDIO_TABLE_DECIMALS_KEY: &str = "curvebook.trailing_studio.table_decimals";

pub(crate) fn studio_snap_enabled(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(STUDIO_SNAP_ENABLED_KEY))).unwrap_or(true)
}

pub(crate) fn set_studio_snap_enabled(ctx: &egui::Context, enabled: bool) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(STUDIO_SNAP_ENABLED_KEY), enabled));
}

pub(crate) fn studio_table_decimals(ctx: &egui::Context) -> Option<u8> {
    ctx.data(|d| d.get_temp::<Option<u8>>(egui::Id::unique(STUDIO_TABLE_DECIMALS_KEY))).flatten()
}

pub(crate) fn set_studio_table_decimals(ctx: &egui::Context, decimals: Option<u8>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(STUDIO_TABLE_DECIMALS_KEY), decimals));
}

pub(super) fn format_studio_toggle_tooltip(app: &UiApp, verb: &str) -> String {
    match app.shortcuts.active_combo("ui.inspector.toggle") {
        Some(combo) if !combo.trim().is_empty() => format!("{verb} ({})", crate::menus::format_shortcut_for_platform(combo)),
        _ => verb.to_owned(),
    }
}

/// Input context bundle for [`render_trailing_studio`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct TrailingStudioContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub enabled: bool,
    pub focused: bool,
    pub selected_panel_index: Option<usize>,
    pub selected_graph_annotation: Option<usize>,
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

/// Resolve the active [`StudioContextSurface`] strictly from live document sheet & brushing state,
/// respecting Split Comparison's explicit [`crate::dialogs_canvas::SplitWorkspaceState::effective_inspector_target`] (`R20.3`).
#[must_use]
pub fn resolve_studio_surface(app: &UiApp) -> StudioContextSurface {
    let doc = app.document();
    let Some(target) = app.split.effective_inspector_target(doc) else {
        return StudioContextSurface::TableSelection;
    };
    if doc.layout_sheet(target).is_some() {
        return StudioContextSurface::PageLayoutComposer;
    }
    if doc.graph_sheet(target).is_some() {
        let valid_brushed = app.brushed_cell.as_ref().is_some_and(|b| doc.data_sheet(&b.sheet_id).is_some());
        return if valid_brushed && !(app.split.enabled && app.split.inspector_pinned) {
            StudioContextSurface::DataPointDrillBack
        } else {
            StudioContextSurface::GraphInspector
        };
    }
    if doc.result_sheet(target).is_some() {
        return StudioContextSurface::ResultsInterpret;
    }
    if doc.info_sheet(target).is_some() {
        return StudioContextSurface::MetadataHooks;
    }
    if let Some(ds) = doc.data_sheet(target)
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
    set_studio_snap_enabled(ui.ctx(), state.snap_enabled);
    set_studio_table_decimals(ui.ctx(), state.table_decimals);
    let surface = resolve_studio_surface(app);
    let mut report = TrailingStudioFrameReport {
        dock_rect_pt: None,
        surface,
        collapsed_to_rail: ctx_in.layout.trailing_collapsed_to_rail,
        operand_highlight_count: 0,
        referenced_by_count: 0,
    };

    if ctx_in.enabled && !ui.ctx().egui_wants_keyboard_input() {
        let on_result_sheet = crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some();
        if crate::egui_shell::inspect_bridge::consume_matching_shortcut(ui.ctx(), app, on_result_sheet, |cmd| cmd == "ui.inspector.toggle").is_some()
        {
            intents.push(UiIntent::ToggleTrailingCollapsedRail);
        }
    }

    if !ctx_in.layout.trailing_visible || ctx_in.layout.trailing_dock_w_pt <= 0.5 {
        return report;
    }

    let w = ctx_in.layout.trailing_dock_w_pt;
    let dock_frame = egui::Frame::NONE
        .fill(ctx_in.tokens.panel_bg)
        .stroke(Stroke::new(1.0, if ctx_in.focused { ctx_in.tokens.focus_ring } else { ctx_in.tokens.divider }))
        .inner_margin(if ctx_in.layout.trailing_collapsed_to_rail { egui::Margin::same(4) } else { egui::Margin::ZERO });

    let doc = app.document();
    let active_id = app.split.effective_inspector_target(doc);

    let panel_builder =
        if ctx_in.layout.rtl_mirrored { egui::Panel::left("egui_shell.trailing_studio") } else { egui::Panel::right("egui_shell.trailing_studio") };
    let resp = panel_builder
        .exact_size(w)
        .resizable(false)
        .frame(dock_frame)
        .show(ui, |ui| {
            if !ctx_in.enabled {
                ui.disable();
            }
            if ctx_in.layout.trailing_collapsed_to_rail {
                render_collapsed_rail(ui, app, ctx_in.tokens, surface, ctx_in.enabled, meta, intents);
                return;
            }

            // Shared 32 pt Panel Header (aligned flush with Navigator and Center Split pane headers).
            let header_h = ctx_in.tokens.metrics.panel_header_h;
            let (header_rect, hdr_region) = ui.allocate_exact_size(Vec2::new(ui.available_width(), header_h), Sense::hover());
            let hdr_enabled = ctx_in.enabled;
            hdr_region.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Region, hdr_enabled, "Trailing Studio Header"));
            ui.painter().rect_filled(header_rect, egui::CornerRadius::ZERO, ctx_in.tokens.panel_bg);
            ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, ctx_in.tokens.divider));
            meta.record_response(
                ui,
                &hdr_region,
                FrameInspectEntryInput {
                    widget_id: "inspector.panel_header",
                    role: "region",
                    label: "Trailing Studio Header",
                    command_id: None,
                    value: None,
                    enabled: ctx_in.enabled,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );

            let mut hdr_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(header_rect.shrink2(Vec2::new(ctx_in.tokens.metrics.space_md, ctx_in.tokens.metrics.space_xs)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            hdr_ui.set_clip_rect(header_rect);

            let base_title = surface_localized_title(ctx_in.tokens.locale, surface);
            let title_owned = if app.split.enabled
                && let Some(target_title) = active_id.and_then(|id| crate::egui_shell::doc_sheet_title(doc, id))
            {
                format!("{base_title} · {target_title}")
            } else {
                base_title.to_owned()
            };
            let right_reserve = if app.split.enabled { 84.0 } else { 32.0 };
            let title_budget = (hdr_ui.available_width() - right_reserve).max(40.0);
            let mut job = egui::text::LayoutJob::simple_singleline(
                title_owned.clone(),
                egui::FontId::proportional(ctx_in.tokens.metrics.font_ui_body_pt),
                ctx_in.tokens.text_strong,
            );
            job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
            job.wrap.max_width = title_budget;
            let galley = hdr_ui.fonts_mut(|f| f.layout_job(job));
            let elided = galley.elided;
            let (t_rect, t_resp) = hdr_ui.allocate_exact_size(Vec2::new(title_budget, ctx_in.tokens.metrics.min_hit_target_pt), Sense::hover());
            if hdr_ui.is_rect_visible(t_rect) {
                let tp = Pos2::new(t_rect.min.x, t_rect.center().y - galley.size().y * 0.5);
                let painter = hdr_ui.painter().with_clip_rect(t_rect);
                painter.galley(tp, galley, ctx_in.tokens.text_strong);
            }
            let acc_title = title_owned.clone();
            t_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Heading, true, acc_title.clone()));
            let t_resp = if elided { t_resp.on_hover_text(title_owned.as_str()) } else { t_resp };
            if t_rect.intersect(hdr_ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
                meta.record_response(
                    &hdr_ui,
                    &t_resp,
                    FrameInspectEntryInput {
                        widget_id: "inspector.surface_title",
                        role: "heading",
                        label: &title_owned,
                        command_id: None,
                        value: Some(json!(surface.slug())),
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(title_owned.as_str()),
                    },
                );
            }

            hdr_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = ctx_in.tokens.metrics.space_xs;
                let collapse_tip = format_studio_toggle_tooltip(app, "Collapse Studio");
                let c_resp = studio_chip(
                    ui,
                    ctx_in.tokens,
                    meta,
                    &StudioChipSpec {
                        label: if is_rtl_locale(ctx_in.tokens.locale) { "«" } else { "»" },
                        tooltip: &collapse_tip,
                        max_w: 24.0,
                        selected: false,
                        enabled: ctx_in.enabled,
                        border_override: None,
                        widget_id: "inspector.collapse_rail",
                        role: "button",
                        command_id: Some("ui.inspector.toggle"),
                        disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                    },
                );
                if c_resp && ctx_in.enabled {
                    intents.push(UiIntent::ToggleTrailingCollapsedRail);
                }

                if app.split.enabled {
                    let pin_label = if app.split.inspector_pinned { "Pinned" } else { "Pin" };
                    let (_, pin_clicked) = crate::egui_shell::ui_kit::ghost_action_button(
                        ui,
                        ctx_in.tokens,
                        meta,
                        "inspector.pin_toggle",
                        pin_label,
                        None,
                        app.split.inspector_pinned,
                        ctx_in.enabled,
                        ZLayer::Workspace,
                    );
                    if pin_clicked {
                        intents.push(UiIntent::ToggleInspectorPin);
                    }
                }
            });

            // When pinned in Split Comparison and `doc.active_sheet_id` differs, offer one-click Switch chip.
            if app.split.enabled
                && app.split.inspector_pinned
                && let Some(doc_active) = doc.active_sheet_id.as_ref()
                && active_id != Some(doc_active)
                && let Some(active_title) = crate::egui_shell::doc_sheet_title(doc, doc_active)
            {
                let switch_lbl = format!("Switch to {active_title}");
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    let (_, clicked) = crate::egui_shell::ui_kit::ghost_action_button(
                        ui,
                        ctx_in.tokens,
                        meta,
                        "inspector.switch_to_active",
                        &switch_lbl,
                        None,
                        false,
                        ctx_in.enabled,
                        ZLayer::Workspace,
                    );
                    if clicked {
                        intents.push(UiIntent::SwitchInspectorTarget { sheet_id: doc_active.clone() });
                    }
                });
            }

            // Reserve dedicated right gutter inside ScrollArea so `Role::ScrollBar` NEVER overlaps any widget.
            ui.spacing_mut().scroll.floating = false;
            let body_margin = egui::Margin::symmetric(ctx_in.tokens.metrics.space_md as i8, ctx_in.tokens.metrics.space_xs as i8);
            egui::Frame::NONE.inner_margin(body_margin).show(ui, |ui| {
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
    let splitter_panel = if ctx_in.layout.rtl_mirrored {
        egui::Panel::left("egui_shell.trailing_dock_splitter")
    } else {
        egui::Panel::right("egui_shell.trailing_dock_splitter")
    };
    splitter_panel.exact_size(splitter_w).resizable(false).frame(egui::Frame::NONE.fill(tokens.panel_bg)).show(ui, |ui| {
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
                intents.push(UiIntent::SyncTrailingWidth { width_pt: default_w });
            } else if resp.dragged() {
                let delta_w = if ctx_in.layout.rtl_mirrored { resp.drag_delta().x } else { -resp.drag_delta().x };
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
                    intents.push(UiIntent::SyncTrailingWidth { width_pt: new_w });
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
