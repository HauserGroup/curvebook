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

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisKind, AxisSpec, ChecklistStatus, ConstantValue, DataSheet, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphAnnotation,
    GraphSheet, GraphicalDataObject, GroupedArrangement, HookSource, InfoSheet, LayoutSheet, LevelOrderKind, MultipleVariablesTable, PageOrientation,
    PairwiseDisplayMode, PanelBadgeStyle, ParameterScope, ProjectDocument, ResultSheet, ResultStatus, SeriesSpec, SheetId, TableArchetype,
    VariableColumn, VariableRole, VariableSource,
};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_math::AxisScale;
use curvebook_palette::{CanvasPreviewFilter, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_tools::GridCellCoord;
use curvebook_transform::{DependencyTarget, extract_dependencies, parse_formula, validate_no_formula_cycle};
use egui::{Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_r9::sanitize_diagnostics_only_text;
use crate::egui_shell::{
    FrameInspectEntryInput, FrameInspectMeta, OKABE_ITO_SERIES_PALETTE, PendingUiIntents, ResponsiveShellLayout, Tokens, doc_sheet_title, widgets,
};
use crate::i18n::{is_rtl_locale, tr};
use crate::inspect::ZLayer;

const STUDIO_INNER_PAD: f32 = 8.0;
const SCROLL_BAR_RESERVE_PT: f32 = 16.0;
const SECTION_BTN_H: f32 = 22.0;

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

/// Extract colored column operand highlights from an MV formula expression in deterministic palette order.
#[must_use]
pub fn resolve_formula_operand_highlights(vars: &[VariableColumn], formula: &str) -> Vec<MvOperandHighlight> {
    let Ok(expr) = parse_formula(formula) else {
        return Vec::new();
    };
    let deps = extract_dependencies(&expr);
    let mut seen_cols = BTreeSet::new();
    let mut out = Vec::new();

    for dep in deps {
        let resolved_idx = match dep {
            DependencyTarget::ColumnIndex(idx) if idx < vars.len() => Some(idx),
            DependencyTarget::ColumnName(ref name) => {
                let needle = name.trim();
                vars.iter().position(|v| v.title.trim().eq_ignore_ascii_case(needle))
            }
            DependencyTarget::InfoConstant(_) | DependencyTarget::ColumnIndex(_) => None,
        };
        if let Some(idx) = resolved_idx
            && seen_cols.insert(idx)
        {
            let swatch = OKABE_ITO_SERIES_PALETTE[idx % OKABE_ITO_SERIES_PALETTE.len()];
            let [r, g, b, _] = swatch.to_array();
            out.push(MvOperandHighlight {
                variable_index: idx,
                variable_title: vars[idx].title.clone(),
                color_rgb: [r, g, b],
                color_hex: format!("#{r:02X}{g:02X}{b:02X}"),
            });
        }
    }
    out
}

/// Scan `ProjectDocument` for every downstream sheet referencing constants or notes from `info`.
#[must_use]
pub fn collect_info_downstream_references(doc: &ProjectDocument, info: &InfoSheet) -> Vec<InfoDownstreamRef> {
    let mut refs = Vec::new();
    let info_matches = |hook_sheet: Option<&SheetId>, paired_source: Option<&SheetId>| -> bool {
        if let Some(explicit) = hook_sheet {
            return explicit == &info.id;
        }
        paired_source.is_some_and(|src| info.linked_data_sheets.contains(src)) || doc.info_sheets.first().is_some_and(|p| p.id == info.id)
    };

    for ds_id in &info.linked_data_sheets {
        if let Some(ds) = doc.data_sheet(ds_id) {
            refs.push(InfoDownstreamRef {
                target_sheet_id: ds.id.clone(),
                target_title: ds.title.clone(),
                section_tag: "Data",
                binding_summary: String::from("Paired info table"),
            });
        }
    }

    for ds in &doc.data_sheets {
        if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
            for var in &mv.variables {
                if let VariableSource::Calculated { formula } = &var.source
                    && let Ok(expr) = parse_formula(formula)
                {
                    let info_keys: Vec<String> = extract_dependencies(&expr)
                        .into_iter()
                        .filter_map(|d| match d {
                            DependencyTarget::InfoConstant(k) => Some(k),
                            _ => None,
                        })
                        .collect();
                    if !info_keys.is_empty() {
                        refs.push(InfoDownstreamRef {
                            target_sheet_id: ds.id.clone(),
                            target_title: ds.title.clone(),
                            section_tag: "Data",
                            binding_summary: format!("Formula `{}` uses Info({})", var.title, info_keys.join(", ")),
                        });
                    }
                }
            }
        }
    }

    for rs in &doc.result_sheets {
        if let AnalysisKind::NonlinearRegression(nl) = &rs.analysis.kind {
            for rule in &nl.parameter_rules {
                if let ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id, constant_name }) = &rule.scope
                    && info_matches(info_sheet_id.as_ref(), Some(&rs.analysis.source_sheet_id))
                {
                    refs.push(InfoDownstreamRef {
                        target_sheet_id: rs.id.clone(),
                        target_title: rs.title().to_owned(),
                        section_tag: "Result",
                        binding_summary: format!("Constraint `{}` <- {{{{info:{constant_name}}}}}", rule.name),
                    });
                }
            }
        }
    }

    for gs in &doc.graph_sheets {
        for hook in gs.spec.inline_hooks() {
            if let HookSource::InfoConstant { info_sheet_id, constant_name } = hook
                && info_matches(info_sheet_id.as_ref(), Some(&gs.spec.primary_source_sheet_id))
            {
                refs.push(InfoDownstreamRef {
                    target_sheet_id: gs.id.clone(),
                    target_title: gs.title.clone(),
                    section_tag: "Graph",
                    binding_summary: format!("Text/title hook {{{{info:{constant_name}}}}}"),
                });
            }
        }
    }

    for ls in &doc.layout_sheets {
        for hook in ls.spec.inline_hooks() {
            if let HookSource::InfoConstant { info_sheet_id, constant_name } = hook
                && info_matches(info_sheet_id.as_ref(), None)
            {
                refs.push(InfoDownstreamRef {
                    target_sheet_id: ls.id.clone(),
                    target_title: ls.title.clone(),
                    section_tag: "Layout",
                    binding_summary: format!("Callout hook {{{{info:{constant_name}}}}}"),
                });
            }
        }
    }

    refs
}

struct FormulaFnEntry {
    name: &'static str,
    signature: &'static str,
    insert: &'static str,
    summary: &'static str,
}

const STUDIO_FORMULA_FUNCS: &[FormulaFnEntry] = &[
    FormulaFnEntry { name: "ln", signature: "ln(x)", insert: "ln(", summary: "Natural logarithm base e" },
    FormulaFnEntry { name: "log10", signature: "log10(x)", insert: "log10(", summary: "Common base-10 logarithm" },
    FormulaFnEntry { name: "exp", signature: "exp(x)", insert: "exp(", summary: "Exponential e^x" },
    FormulaFnEntry { name: "sqrt", signature: "sqrt(x)", insert: "sqrt(", summary: "Square root of x >= 0" },
    FormulaFnEntry { name: "abs", signature: "abs(x)", insert: "abs(", summary: "Absolute magnitude |x|" },
    FormulaFnEntry { name: "mean", signature: "mean(var)", insert: "mean(", summary: "Column arithmetic mean" },
    FormulaFnEntry { name: "sd", signature: "sd(var)", insert: "sd(", summary: "Sample standard deviation" },
    FormulaFnEntry { name: "sem", signature: "sem(var)", insert: "sem(", summary: "Standard error of the mean" },
    FormulaFnEntry { name: "median", signature: "median(var)", insert: "median(", summary: "Sample median (50th pct)" },
    FormulaFnEntry { name: "iqr", signature: "iqr(var)", insert: "iqr(", summary: "Interquartile range Q3 - Q1" },
    FormulaFnEntry { name: "if", signature: "if(cond, t, f)", insert: "if(", summary: "Conditional branch per row" },
    FormulaFnEntry { name: "coalesce", signature: "coalesce(a, b)", insert: "coalesce(", summary: "First non-missing value" },
    FormulaFnEntry { name: "Info", signature: "Info(\"Key\")", insert: "Info(\"", summary: "Linked InfoSheet constant" },
    FormulaFnEntry { name: "countif", signature: "countif(cond)", insert: "countif(", summary: "Conditional row count" },
    FormulaFnEntry { name: "sumif", signature: "sumif(var, cond)", insert: "sumif(", summary: "Conditional column sum" },
    FormulaFnEntry { name: "meanif", signature: "meanif(var, cond)", insert: "meanif(", summary: "Conditional column mean" },
];

/// Return the canonical formula signature shown by the trailing Formula Studio (`#14`).
#[must_use]
pub fn studio_formula_signature(name: &str) -> Option<&'static str> {
    STUDIO_FORMULA_FUNCS.iter().find(|entry| entry.name == name).map(|entry| entry.signature)
}

#[allow(clippy::too_many_arguments)]
fn studio_chip(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    tooltip: &str,
    max_w: f32,
    selected: bool,
    enabled: bool,
    border_override: Option<Color32>,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    role: &str,
    command_id: Option<&str>,
    disabled_reason: Option<&str>,
) -> bool {
    let font_id = egui::FontId::proportional(11.5);
    let tc = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let max_text_w = (max_w - 14.0).max(12.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id.clone(), tc);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = max_text_w;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let w = (galley.size().x + 14.0).clamp(24.0_f32.min(max_w), max_w.max(24.0));
    let rect = ui.allocate_space(Vec2::new(w, SECTION_BTN_H)).1;
    let visible = rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if !visible.is_positive() || visible.width() < 4.0 || visible.height() < 4.0 {
        return false;
    }

    let resp =
        ui.interact(visible, ui.make_persistent_id(("egui_shell.studio_chip", widget_id)), if enabled { Sense::click() } else { Sense::hover() });
    let (galley, ellipsized) = if visible.width() < rect.width() - 0.5 {
        let mut vis_job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, tc);
        vis_job.wrap = egui::text::TextWrapping {
            max_width: (visible.width() - 14.0).max(4.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let vg = ui.fonts_mut(|f| f.layout_job(vis_job));
        let ve = vg.elided;
        (vg, ve)
    } else {
        let e = galley.elided;
        (galley, e)
    };

    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let bc = if !enabled {
        tokens.divider
    } else if let Some(ov) = border_override {
        ov
    } else if selected || resp.hovered() {
        tokens.accent
    } else {
        tokens.button_border
    };
    let sw = if border_override.is_some() { 1.5 } else { 1.0 };

    let painter = ui.painter().with_clip_rect(visible);
    painter.rect(rect, 4.0, bg, Stroke::new(sw, bc), StrokeKind::Inside);
    if galley.size().x <= visible.width() - 8.0 || ellipsized {
        let tp = Pos2::new(visible.min.x + 7.0, rect.center().y - galley.size().y * 0.5);
        painter.galley(tp, galley, tc);
    }

    let acc_name = label.to_owned();
    let ak_role = match role {
        "tab" => egui::Role::Tab,
        "text" => egui::Role::Label,
        _ => egui::Role::Button,
    };
    resp.widget_info(move || egui::WidgetInfo::labeled(ak_role, enabled, acc_name.clone()));

    let resp = if !enabled {
        if let Some(r) = disabled_reason { resp.on_disabled_hover_text(r) } else { resp }
    } else if ellipsized {
        resp.on_hover_text(label)
    } else if !tooltip.is_empty() && tooltip != label {
        resp.on_hover_text(tooltip)
    } else {
        resp
    };

    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role,
            label,
            command_id,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some(disabled_reason).flatten(),
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    resp.clicked() && enabled
}

#[allow(clippy::too_many_arguments)]
fn studio_full_button(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    tooltip: &str,
    selected: bool,
    enabled: bool,
    primary: bool,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    command_id: Option<&str>,
    disabled_reason: Option<&str>,
) -> bool {
    let w = ui.available_width().max(48.0);
    let font_id = egui::FontId::proportional(11.5);
    let tc = if !enabled {
        tokens.text_disabled
    } else if primary {
        tokens.paper_white
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let rect = ui.allocate_space(Vec2::new(w, 24.0)).1;
    let visible = rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if !visible.is_positive() || visible.width() < 4.0 || visible.height() < 4.0 {
        return false;
    }

    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, tc);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = (visible.width() - 16.0).max(8.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let ellipsized = galley.elided;
    let resp = ui.interact(
        visible,
        ui.make_persistent_id(("egui_shell.studio_full_button", widget_id)),
        if enabled { Sense::click() } else { Sense::hover() },
    );

    let bg = if !enabled {
        tokens.panel_bg
    } else if primary {
        if resp.hovered() { tokens.accent_strong } else { tokens.accent }
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let bc = if !enabled {
        tokens.divider
    } else if primary || selected || resp.hovered() {
        tokens.accent
    } else {
        tokens.button_border
    };

    let painter = ui.painter().with_clip_rect(visible);
    painter.rect(rect, 5.0, bg, Stroke::new(1.0, bc), StrokeKind::Inside);
    if galley.size().x <= visible.width() - 8.0 || ellipsized {
        let tp = Pos2::new(visible.min.x + ((visible.width() - galley.size().x) * 0.5).max(8.0), rect.center().y - galley.size().y * 0.5);
        painter.galley(tp, galley, tc);
    }

    let acc_name = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, acc_name.clone()));

    let resp = if !enabled {
        if let Some(r) = disabled_reason { resp.on_disabled_hover_text(r) } else { resp }
    } else if ellipsized {
        resp.on_hover_text(label)
    } else if !tooltip.is_empty() && tooltip != label {
        resp.on_hover_text(tooltip)
    } else {
        resp
    };

    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label,
            command_id,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some(disabled_reason).flatten(),
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    resp.clicked() && enabled
}

fn surface_localized_title(locale: &str, surface: StudioContextSurface) -> &'static str {
    let key = match surface {
        StudioContextSurface::MultipleVariablesFormula => "mv.editor_btn",
        StudioContextSurface::MetadataHooks => "nav.info",
        StudioContextSurface::ResultsInterpret => "results.interpret",
        StudioContextSurface::DataPointDrillBack => "inspector.section.drill_back",
        _ => "",
    };
    if !key.is_empty() {
        let translated = tr(locale, key);
        if translated != key && !translated.is_empty() {
            return translated;
        }
    }
    match surface {
        StudioContextSurface::MultipleVariablesFormula => "MV Formula Studio",
        StudioContextSurface::TableSelection => "Table & Selection",
        StudioContextSurface::MetadataHooks => "Metadata & Hooks",
        StudioContextSurface::ResultsInterpret => "Interpret & Diagnostics",
        StudioContextSurface::GraphInspector => "Graph Inspector",
        StudioContextSurface::DataPointDrillBack => "Data Point Drill-Back",
        StudioContextSurface::PageLayoutComposer => "Page Layout Composer",
    }
}

fn render_passive_collapsed_rail_badge(ui: &mut egui::Ui, tokens: &Tokens, glyph: &str) {
    let font_id = egui::FontId::proportional(11.5);
    let mut job = egui::text::LayoutJob::simple_singleline(glyph.to_owned(), font_id, tokens.text_disabled);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = 12.0;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let w = (galley.size().x + 14.0).clamp(24.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, SECTION_BTN_H), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, 4.0, tokens.panel_bg, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        let tp = Pos2::new(rect.min.x + 7.0, rect.center().y - galley.size().y * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(4.0, 1.0)));
        painter.galley(tp, galley, tokens.text_disabled);
    }
}

fn render_collapsed_rail(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    surface: StudioContextSurface,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = &tokens.locale;
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let expand_resp = studio_chip(
            ui,
            tokens,
            if is_rtl_locale(locale) { "»" } else { "«" },
            "Expand Studio (Cmd/Ctrl+Shift+I)",
            26.0,
            false,
            enabled,
            None,
            meta,
            "inspector.expand_rail",
            "button",
            Some("ui.inspector.toggle"),
            (!enabled).then_some("modal dialog is active"),
        );
        if expand_resp && enabled {
            intents.push_toggle_trailing_collapsed_rail();
        }

        widgets::divider(ui);

        let all_surfaces = [
            StudioContextSurface::MultipleVariablesFormula,
            StudioContextSurface::TableSelection,
            StudioContextSurface::MetadataHooks,
            StudioContextSurface::ResultsInterpret,
            StudioContextSurface::GraphInspector,
            StudioContextSurface::DataPointDrillBack,
            StudioContextSurface::PageLayoutComposer,
        ];
        for ctx_surface in all_surfaces {
            if !enabled {
                // While a modal dialog scrim covers the workspace, omit behind-backdrop AccessKit `Tab`
                // nodes so foreground modal scrollbars cannot collide with occluded rail items.
                render_passive_collapsed_rail_badge(ui, tokens, ctx_surface.rail_glyph());
                continue;
            }
            let active = ctx_surface == surface;
            let title = surface_localized_title(locale, ctx_surface);
            let wid = format!("inspector.rail.{}", ctx_surface.slug());
            let r = studio_chip(
                ui,
                tokens,
                ctx_surface.rail_glyph(),
                title,
                26.0,
                active,
                enabled,
                active.then_some(tokens.accent),
                meta,
                &wid,
                "tab",
                None,
                None,
            );
            if r {
                intents.push_toggle_trailing_collapsed_rail();
            }
        }
    });
}

fn role_label(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "Continuous",
        VariableRole::Categorical => "Categorical",
        VariableRole::Label => "Label",
        VariableRole::Calculated => "Calculated",
    }
}

fn render_mv_formula_studio(
    ui: &mut egui::Ui,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let col_count = mv.variables.len();
    if col_count > 0 && state.mv_selected_col >= col_count {
        state.mv_selected_col = 0;
    }
    let active_col = state.mv_selected_col;

    // Sync formula & title drafts cleanly whenever selected sheet or column changes.
    if state.mv_synced_sheet.as_ref() != Some(&ds.id) || state.mv_synced_col != Some(active_col) {
        state.mv_synced_sheet = Some(ds.id.clone());
        state.mv_synced_col = Some(active_col);
        if let Some(var) = mv.variables.get(active_col) {
            state.mv_title_draft = var.title.clone();
            state.mv_formula_draft = match &var.source {
                VariableSource::Calculated { formula } => formula.clone(),
                VariableSource::Entered => String::new(),
            };
        }
    }

    // Keep live colored operand column outline highlights synced with the active formula!
    let effective_formula = if !state.mv_formula_draft.trim().is_empty() {
        state.mv_formula_draft.as_str()
    } else if let Some(VariableSource::Calculated { formula }) = mv.variables.get(active_col).map(|v| &v.source) {
        formula.as_str()
    } else {
        ""
    };
    state.mv_operand_highlights = resolve_formula_operand_highlights(&mv.variables, effective_formula);

    // Variable selector row
    widgets::subheader(ui, tr(locale, "mv.panel.var_name"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let max_w = (ui.available_width() * 0.48).max(60.0);
        for (idx, var) in mv.variables.iter().enumerate() {
            let hl = state.mv_operand_highlights.iter().find(|h| h.variable_index == idx).map(MvOperandHighlight::color32);
            let wid = format!("inspector.mv.col.{idx}");
            let r = studio_chip(ui, tokens, &var.title, &var.title, max_w, idx == active_col, ctx_in.enabled, hl, meta, &wid, "button", None, None);
            if r {
                state.mv_selected_col = idx;
                state.mv_synced_col = None;
            }
        }
    });

    ui.add_space(4.0);
    let sub_tabs = [tr(locale, "mv.editor_btn"), "Role & Levels"];
    if let Some(next_sub) = widgets::tab_bar(ui, &sub_tabs, state.mv_sub_tab) {
        state.mv_sub_tab = next_sub;
    }
    ui.add_space(4.0);

    if state.mv_sub_tab == 0 {
        let mut resp_title = egui::TextEdit::singleline(&mut state.mv_title_draft).desired_width(ui.available_width()).hint_text("Variable title");
        if !ctx_in.enabled {
            resp_title = resp_title.interactive(false);
        }
        ui.add(resp_title);

        ui.add_space(4.0);
        widgets::subheader(ui, "f(x) =");
        let mut formula_edit = egui::TextEdit::singleline(&mut state.mv_formula_draft).desired_width(ui.available_width()).hint_text("= [Dose] * 2");
        if !ctx_in.enabled {
            formula_edit = formula_edit.interactive(false);
        }
        let f_resp = ui.add(formula_edit);
        if f_resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
            meta.record_response(
                ui,
                &f_resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.mv.formula_input",
                    role: "textbox",
                    label: "f(x) formula",
                    command_id: Some("table.setCalculatedVariable"),
                    value: Some(json!(state.mv_formula_draft)),
                    enabled: ctx_in.enabled,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }

        // Recompute operand highlights after user edit inside same frame
        state.mv_operand_highlights = resolve_formula_operand_highlights(&mv.variables, &state.mv_formula_draft);

        // Live validation + cycle badge
        let formula_status = if state.mv_formula_draft.trim().is_empty() {
            Ok("Enter an expression referencing [Columns]".to_owned())
        } else if active_col < col_count {
            validate_no_formula_cycle(mv, active_col, &state.mv_formula_draft)
                .map(|()| format!("Valid formula • {} operand highlight(s)", state.mv_operand_highlights.len()))
                .map_err(|e| e.to_string())
        } else {
            parse_formula(&state.mv_formula_draft).map(|_| String::from("Valid formula")).map_err(|e| e.to_string())
        };
        match &formula_status {
            Ok(msg) => ui.label(egui::RichText::new(msg).size(11.0).color(tokens.link_green)),
            Err(err) => ui.label(egui::RichText::new(err).size(11.0).color(tokens.link_red)),
        };

        // Colored operand column outline chips
        if !state.mv_operand_highlights.is_empty() {
            ui.add_space(2.0);
            widgets::dim_label(ui, "Operand Highlights");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (i, hl) in state.mv_operand_highlights.iter().enumerate() {
                    let lbl = format!("[{}] {}", hl.variable_title, hl.color_hex);
                    let wid = format!("inspector.mv.operand_highlight.{i}");
                    let _ = studio_chip(
                        ui,
                        tokens,
                        &lbl,
                        &format!("Outlined operand column `{}` ({})", hl.variable_title, hl.color_hex),
                        ui.available_width(),
                        false,
                        true,
                        Some(hl.color32()),
                        meta,
                        &wid,
                        "text",
                        None,
                        None,
                    );
                }
            });
        }

        let can_apply = ctx_in.enabled && !state.mv_formula_draft.trim().is_empty() && formula_status.is_ok();
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            "Apply Formula",
            "Save calculated variable formula (table.setCalculatedVariable)",
            false,
            can_apply,
            true,
            meta,
            "inspector.mv.apply_formula",
            Some("table.setCalculatedVariable"),
            (!can_apply).then_some("enter a valid non-circular formula"),
        ) {
            let title =
                if state.mv_title_draft.trim().is_empty() { format!("Calc {}", col_count + 1) } else { state.mv_title_draft.trim().to_owned() };
            let mut params = json!({
                "sheet_id": ds.id.as_str(),
                "title": title,
                "formula": state.mv_formula_draft.trim(),
            });
            if active_col < col_count
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("col"), json!(active_col));
            }
            intents.push_engine("table.setCalculatedVariable", params);
        }

        ui.add_space(6.0);
        widgets::subheader(ui, tr(locale, "mv.panel.column_vars"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            let max_w = (ui.available_width() * 0.48).max(56.0);
            for (idx, var) in mv.variables.iter().enumerate() {
                let hl = state.mv_operand_highlights.iter().find(|h| h.variable_index == idx).map(MvOperandHighlight::color32);
                let wid = format!("inspector.mv.insert_var.{idx}");
                let r = studio_chip(
                    ui,
                    tokens,
                    &format!("[{}]", var.title),
                    "Insert variable reference into formula",
                    max_w,
                    false,
                    ctx_in.enabled,
                    hl,
                    meta,
                    &wid,
                    "button",
                    None,
                    None,
                );
                if r {
                    if !state.mv_formula_draft.is_empty() && !state.mv_formula_draft.ends_with(['(', ' ', '+', '-', '*', '/']) {
                        state.mv_formula_draft.push(' ');
                    }
                    state.mv_formula_draft.push('[');
                    state.mv_formula_draft.push_str(&var.title);
                    state.mv_formula_draft.push(']');
                }
            }
        });

        ui.add_space(4.0);
        widgets::subheader(ui, tr(locale, "mv.panel.func_ref"));
        ui.add(egui::TextEdit::singleline(&mut state.mv_fn_filter).desired_width(ui.available_width()).hint_text("Search functions..."));
        let filter_lower = state.mv_fn_filter.trim().to_ascii_lowercase();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (idx, func) in STUDIO_FORMULA_FUNCS.iter().enumerate() {
                if !filter_lower.is_empty() && !func.name.contains(&filter_lower) && !func.summary.to_ascii_lowercase().contains(&filter_lower) {
                    continue;
                }
                let wid = format!("inspector.mv.fn.{idx}");
                let r = studio_chip(
                    ui,
                    tokens,
                    func.name,
                    func.summary,
                    84.0,
                    state.mv_selected_fn == func.name,
                    ctx_in.enabled,
                    None,
                    meta,
                    &wid,
                    "button",
                    None,
                    None,
                );
                if r {
                    state.mv_selected_fn = String::from(func.name);
                    state.mv_formula_draft.push_str(func.insert);
                }
            }
        });

        if let Some(spec) = STUDIO_FORMULA_FUNCS.iter().find(|f| f.name == state.mv_selected_fn) {
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.divider))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(spec.signature).size(11.5).strong().color(tokens.text_strong));
                    ui.label(egui::RichText::new(spec.summary).size(10.5).color(tokens.text_muted));
                });
        }
    } else if let Some(var) = mv.variables.get(active_col) {
        // Role & Levels sub-tab
        let cur_role = var.role;
        ui.label(egui::RichText::new(format!("{} ({})", var.title, role_label(cur_role))).size(12.0).strong().color(tokens.text_strong));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (slug, lbl, r_kind) in [
                ("continuous", "Continuous", VariableRole::Continuous),
                ("categorical", "Categorical", VariableRole::Categorical),
                ("label", "Label", VariableRole::Label),
            ] {
                let wid = format!("inspector.mv.role.{slug}");
                let r = studio_chip(
                    ui,
                    tokens,
                    lbl,
                    "Assign variable semantic role",
                    88.0,
                    cur_role == r_kind,
                    ctx_in.enabled && cur_role != VariableRole::Calculated,
                    None,
                    meta,
                    &wid,
                    "button",
                    Some("table.setVariableRole"),
                    (cur_role == VariableRole::Calculated).then_some("calculated columns lock Calculated role"),
                );
                if r {
                    intents.push_engine("table.setVariableRole", json!({ "sheet_id": ds.id.as_str(), "col": active_col, "role": slug }));
                }
            }
        });

        if cur_role == VariableRole::Categorical {
            ui.add_space(6.0);
            widgets::subheader(ui, tr(locale, "mv.vars.order_mode"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (order_slug, lbl, active) in [
                    ("entry_order", "Entry", matches!(var.level_order.order, LevelOrderKind::EntryOrder)),
                    ("alphabetical", "A-Z", matches!(var.level_order.order, LevelOrderKind::Alphabetical)),
                    ("frequency_descending", "Freq ↓", matches!(var.level_order.order, LevelOrderKind::FrequencyDescending)),
                    ("frequency_ascending", "Freq ↑", matches!(var.level_order.order, LevelOrderKind::FrequencyAscending)),
                ] {
                    let wid = format!("inspector.mv.order.{order_slug}");
                    let r = studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Set categorical level ordering",
                        74.0,
                        active,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("table.setCategoricalOrder"),
                        None,
                    );
                    if r {
                        intents.push_engine(
                            "table.setCategoricalOrder",
                            json!({ "sheet_id": ds.id.as_str(), "col": active_col, "order": order_slug, "reversed": var.level_order.reversed }),
                        );
                    }
                }
            });

            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut state.mv_recode_from).desired_width(ui.available_width()).hint_text("Recode from level"));
            ui.add(egui::TextEdit::singleline(&mut state.mv_recode_to).desired_width(ui.available_width()).hint_text("Recode to level"));
            let can_recode = ctx_in.enabled && !state.mv_recode_from.trim().is_empty() && !state.mv_recode_to.trim().is_empty();
            if studio_full_button(
                ui,
                tokens,
                "Recode Level",
                "Apply categorical level recoding (table.recodeLevels)",
                false,
                can_recode,
                false,
                meta,
                "inspector.mv.recode_apply",
                Some("table.recodeLevels"),
                (!can_recode).then_some("enter source and target level names"),
            ) {
                let mut map = serde_json::Map::new();
                map.insert(state.mv_recode_from.trim().to_owned(), json!(state.mv_recode_to.trim()));
                intents.push_engine("table.recodeLevels", json!({ "sheet_id": ds.id.as_str(), "col": active_col, "mappings": map }));
            }
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        "Manage All Variables…",
        "Open full Variable Manager & Formula Bar",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.mv.manage_vars",
        Some("ui.table.openFormulaBar"),
        None,
    ) {
        intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": active_col }));
    }
}

fn find_downstream_graph_for_data(doc: &ProjectDocument, ds_id: &SheetId) -> Option<SheetId> {
    doc.graph_sheets.iter().find(|g| &g.spec.primary_source_sheet_id == ds_id).map(|g| g.id.clone())
}

fn render_table_selection_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    ds: Option<&DataSheet>,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let doc = app.document();
    let Some(ds) = ds else {
        widgets::dim_label(ui, tr(tokens.locale, "inspector.no_active_sheet"));
        if studio_full_button(
            ui,
            tokens,
            "Open Data Table…",
            "Create or select a sheet from Navigator",
            false,
            ctx_in.enabled && !doc.data_sheets.is_empty(),
            false,
            meta,
            "inspector.table.select_first",
            Some("sheet.select"),
            doc.data_sheets.is_empty().then_some("no data sheets available"),
        ) && let Some(first) = doc.data_sheets.first()
        {
            intents.push_engine("sheet.select", json!({ "sheet_id": first.id.as_str() }));
        }
        return;
    };

    let arch_label = format!("{:?}", ds.kind());
    ui.label(
        egui::RichText::new(format!("{}  •  {}×{}", arch_label, ds.archetype.row_count(), ds.archetype.column_count()))
            .size(11.5)
            .strong()
            .color(tokens.text_strong),
    );
    if ds.frozen {
        ui.label(egui::RichText::new("Frozen (read-only)").size(11.0).color(tokens.accent_strong));
    }

    ui.add_space(4.0);
    widgets::subheader(ui, "Display Decimals");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (opt, lbl) in [(None, "Auto"), (Some(0), "0"), (Some(1), "1"), (Some(2), "2"), (Some(3), "3"), (Some(4), "4")] {
            let wid = match opt {
                None => String::from("inspector.table.decimals.auto"),
                Some(d) => format!("inspector.table.decimals.{d}"),
            };
            if studio_chip(
                ui,
                tokens,
                lbl,
                "Display decimal places",
                46.0,
                state.table_decimals == opt,
                ctx_in.enabled,
                None,
                meta,
                &wid,
                "button",
                None,
                None,
            ) {
                state.table_decimals = opt;
            }
        }
    });

    ui.add_space(6.0);
    widgets::subheader(ui, "Selection & Graph Subset");
    let bounds = app.grid.tool.selection_bounds();
    let sel_text = if let Some((a, b)) = bounds {
        let r0 = a.row.min(b.row);
        let r1 = a.row.max(b.row);
        let c0 = a.col.min(b.col);
        let c1 = a.col.max(b.col);
        format!("Rows {}..{}  •  Cols {}..{}", r0 + 1, r1 + 1, c0 + 1, c1 + 1)
    } else {
        String::from("No cells selected")
    };
    state.selected_subset_rows_summary = sel_text.clone();
    widgets::dim_label(ui, &sel_text);

    let has_bounds = bounds.is_some() && !ds.frozen;
    if studio_full_button(
        ui,
        tokens,
        "Exclude Selected Subset",
        "Exclude selected rows/columns from graphs and analyses (table.excludeCells)",
        false,
        ctx_in.enabled && has_bounds,
        false,
        meta,
        "inspector.table.exclude_subset",
        Some("table.excludeCells"),
        ds.frozen.then_some("sheet is frozen").or_else(|| (!has_bounds).then_some("select cells first")),
    ) && let Some((a, b)) = bounds
    {
        intents.push_engine(
            "table.excludeCells",
            json!({
                "sheet_id": ds.id.as_str(),
                "row_start": a.row.min(b.row),
                "row_end": a.row.max(b.row),
                "col_start": a.col.min(b.col),
                "col_end": a.col.max(b.col),
                "subcol_start": a.subcol.min(b.subcol),
                "subcol_end": a.subcol.max(b.subcol),
                "excluded": true,
            }),
        );
    }

    if studio_full_button(
        ui,
        tokens,
        "Include Selected Subset",
        "Clear exclusion mask on selected cells (table.excludeCells)",
        false,
        ctx_in.enabled && has_bounds,
        false,
        meta,
        "inspector.table.include_subset",
        Some("table.excludeCells"),
        ds.frozen.then_some("sheet is frozen").or_else(|| (!has_bounds).then_some("select cells first")),
    ) && let Some((a, b)) = bounds
    {
        intents.push_engine(
            "table.excludeCells",
            json!({
                "sheet_id": ds.id.as_str(),
                "row_start": a.row.min(b.row),
                "row_end": a.row.max(b.row),
                "col_start": a.col.min(b.col),
                "col_end": a.col.max(b.col),
                "subcol_start": a.subcol.min(b.subcol),
                "subcol_end": a.subcol.max(b.subcol),
                "excluded": false,
            }),
        );
    }

    let downstream_graph = find_downstream_graph_for_data(doc, &ds.id);
    if studio_full_button(
        ui,
        tokens,
        "Style Subset in Graph…",
        "Override marker color/size for selected rows on downstream graph",
        false,
        ctx_in.enabled && downstream_graph.is_some(),
        false,
        meta,
        "inspector.table.style_subset",
        Some("ui.formatPoints.open"),
        downstream_graph.is_none().then_some("no downstream graph linked to this table"),
    ) && let Some(gid) = downstream_graph
    {
        intents.push_ui("ui.formatPoints.open", json!({ "sheet_id": gid.as_str() }));
    }

    if !ds.linked_regions.is_empty() {
        ui.add_space(6.0);
        widgets::subheader(ui, "Linked Data Regions");
        for (idx, reg) in ds.linked_regions.iter().enumerate() {
            let up_id = reg.spec.source.upstream_sheet_id().cloned();
            let wid = format!("inspector.table.link.{idx}");
            if studio_full_button(
                ui,
                tokens,
                &reg.provenance_summary,
                "Jump to upstream linked source sheet",
                false,
                ctx_in.enabled && up_id.is_some(),
                false,
                meta,
                &wid,
                up_id.as_ref().map(|_| "sheet.select"),
                up_id.is_none().then_some("external or clipboard provenance"),
            ) && let Some(sid) = up_id
            {
                intents.push_engine("sheet.select", json!({ "sheet_id": sid.as_str() }));
            }
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        "Convert Table Archetype…",
        "Open Table Archetype Converter",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.table.format_table",
        Some("ui.table.convertArchetype.open"),
        None,
    ) {
        intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": ds.id.as_str() }));
    }
}

fn constant_kind_code(val: &ConstantValue) -> u8 {
    match val {
        ConstantValue::Number(_) | ConstantValue::Empty => 0,
        ConstantValue::Text(_) => 1,
        ConstantValue::Date(_) => 2,
    }
}

fn render_metadata_hooks_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    info: &InfoSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> usize {
    let tokens = ctx_in.tokens;
    let doc = app.document();

    widgets::subheader(ui, "Constants & Hooks");
    if state.info_selected_constant.as_ref().is_none_or(|name| info.get_constant(name).is_none()) {
        state.info_selected_constant = info.constants.first().map(|c| c.name.clone());
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let max_w = ui.available_width();
        for (idx, c) in info.constants.iter().enumerate() {
            let selected = state.info_selected_constant.as_deref() == Some(c.name.as_str());
            let lbl = format!("{} = {}", c.name, c.value.display_text());
            let wid = format!("inspector.info.const.{idx}");
            if studio_chip(ui, tokens, &lbl, &lbl, max_w, selected, ctx_in.enabled, None, meta, &wid, "button", None, None) {
                state.info_selected_constant = Some(c.name.clone());
                state.info_synced_key = None;
            }
        }
    });

    let sel_key = state.info_selected_constant.clone().unwrap_or_default();
    if state.info_synced_key.as_ref().is_none_or(|(sid, nm)| sid != &info.id || nm != &sel_key) {
        state.info_synced_key = Some((info.id.clone(), sel_key.clone()));
        if let Some(c) = info.get_constant(&sel_key) {
            state.info_name_draft = c.name.clone();
            state.info_value_draft = c.value.display_text();
            state.info_units_draft = c.units.clone().unwrap_or_default();
            state.info_kind_draft = constant_kind_code(&c.value);
        } else {
            state.info_name_draft.clear();
            state.info_value_draft.clear();
            state.info_units_draft.clear();
            state.info_kind_draft = 0;
        }
    }

    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(&mut state.info_name_draft).desired_width(ui.available_width()).hint_text("Constant name"));
    ui.add(
        egui::TextEdit::singleline(&mut state.info_value_draft).desired_width(ui.available_width()).hint_text("Value (number / text / YYYY-MM-DD)"),
    );
    ui.add(egui::TextEdit::singleline(&mut state.info_units_draft).desired_width(ui.available_width()).hint_text("Units (optional)"));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (idx, kind_lbl) in [(0u8, "Num"), (1, "Text"), (2, "Date")] {
            let wid = format!("inspector.info.kind.{idx}");
            if studio_chip(
                ui,
                tokens,
                kind_lbl,
                "Constant value kind",
                52.0,
                state.info_kind_draft == idx,
                ctx_in.enabled,
                None,
                meta,
                &wid,
                "button",
                None,
                None,
            ) {
                state.info_kind_draft = idx;
            }
        }
    });

    let name_ok = !state.info_name_draft.trim().is_empty();
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        "Save Constant",
        "Store or update constant on InfoSheet (info.setConstant)",
        false,
        ctx_in.enabled && name_ok,
        true,
        meta,
        "inspector.info.save_const",
        Some("info.setConstant"),
        (!name_ok).then_some("constant name cannot be empty"),
    ) {
        let kind_str = match state.info_kind_draft {
            1 => "text",
            2 => "date",
            _ => "number",
        };
        let raw_val = state.info_value_draft.trim();
        let val_json = match state.info_kind_draft {
            0 => raw_val.parse::<f64>().ok().filter(|v| v.is_finite()).map_or_else(|| json!(raw_val), |n| json!(n)),
            _ => json!(raw_val),
        };
        let units_val = if state.info_units_draft.trim().is_empty() { serde_json::Value::Null } else { json!(state.info_units_draft.trim()) };
        intents.push_engine(
            "info.setConstant",
            json!({
                "sheet_id": info.id.as_str(),
                "name": state.info_name_draft.trim(),
                "value": val_json,
                "units": units_val,
                "kind": kind_str,
            }),
        );
    }

    if let Some(existing_name) = state.info_selected_constant.clone()
        && info.get_constant(&existing_name).is_some()
        && studio_full_button(
            ui,
            tokens,
            "Remove Constant",
            "Delete selected constant (info.removeConstant)",
            false,
            ctx_in.enabled,
            false,
            meta,
            "inspector.info.remove_const",
            Some("info.removeConstant"),
            None,
        )
    {
        intents.push_engine("info.removeConstant", json!({ "sheet_id": info.id.as_str(), "name": existing_name }));
    }

    ui.add_space(6.0);
    widgets::subheader(ui, "Referenced By");
    let refs = collect_info_downstream_references(doc, info);
    let ref_count = refs.len();
    if refs.is_empty() {
        widgets::dim_label(ui, "No downstream sheets hook this InfoSheet yet.");
    } else {
        for (idx, item) in refs.iter().enumerate() {
            let lbl = format!("[{}] {} — {}", item.section_tag, item.target_title, item.binding_summary);
            let wid = format!("inspector.info.ref.{idx}");
            if studio_full_button(
                ui,
                tokens,
                &lbl,
                "Jump to downstream referencing sheet",
                false,
                ctx_in.enabled,
                false,
                meta,
                &wid,
                Some("sheet.select"),
                None,
            ) {
                intents.push_engine("sheet.select", json!({ "sheet_id": item.target_sheet_id.as_str() }));
            }
        }
    }

    if let Some(first_graph) = doc.graph_sheets.first() {
        ui.add_space(6.0);
        widgets::divider(ui);
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            "Insert Model / Hook Box…",
            "Insert hooked equation or callout onto active graph",
            false,
            ctx_in.enabled,
            false,
            meta,
            "inspector.info.open_hook_browser",
            Some("ui.graph.insertModelEquation"),
            None,
        ) {
            intents.push_ui("ui.graph.insertModelEquation", json!({ "sheet_id": first_graph.id.as_str() }));
        }
    }

    ref_count
}

fn checklist_badge(status: ChecklistStatus, tokens: &Tokens) -> (&'static str, Color32) {
    match status {
        ChecklistStatus::Pass => ("PASS", tokens.link_green),
        ChecklistStatus::Warn => ("WARN", tokens.accent_strong),
        ChecklistStatus::Violated => ("VIOL", tokens.link_red),
        ChecklistStatus::Info => ("INFO", tokens.link_blue),
    }
}

fn resolve_methods_paragraph(doc: &ProjectDocument, app: &UiApp, rs: &ResultSheet, state: &mut TrailingStudioEguiState) -> String {
    let fmt_str = if state.methods_format.trim().is_empty() { "markdown" } else { state.methods_format.trim() };
    let cache_hit = state.methods_cache_key.as_ref().is_some_and(|(sid, seq, fmt)| sid == &rs.id && *seq == app.mutation_seq() && fmt == fmt_str);
    if cache_hit && !state.methods_cached_text.is_empty() {
        return state.methods_cached_text.clone();
    }

    let report_fmt = match fmt_str {
        "plain" => ReportFormat::PlainText,
        "latex" => ReportFormat::Latex,
        _ => ReportFormat::Markdown,
    };
    let generated = generate_methods_report(doc, &rs.id, report_fmt).unwrap_or_else(|_| {
        app.canvas_dialogs
            .result_view
            .last_methods_report
            .clone()
            .unwrap_or_else(|| format!("Analysis `{}` on source `{}`.", rs.title(), rs.analysis.source_sheet_id.as_str()))
    });
    state.methods_cache_key = Some((rs.id.clone(), app.mutation_seq(), String::from(fmt_str)));
    state.methods_cached_text = generated.clone();
    generated
}

fn render_results_interpret_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    rs: &ResultSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let doc = app.document();

    let (status_lbl, status_clr) = match &rs.status {
        ResultStatus::UpToDate => ("Up to date", tokens.link_green),
        ResultStatus::Dirty => ("Recalculating", tokens.accent_strong),
        ResultStatus::Failed { message, .. } => {
            ui.add(egui::Label::new(egui::RichText::new(format!("Failed: {message}")).size(11.0).color(tokens.link_red)).wrap());
            ("Failed", tokens.link_red)
        }
    };
    let violated_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Violated).count();
    let warn_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Warn).count();
    let pass_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Pass).count();

    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(status_lbl).size(11.0).strong().color(status_clr)).wrap());
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("• {} pass / {} warn / {} viol", pass_n, warn_n, violated_n)).size(11.0).color(tokens.text_muted),
            )
            .wrap(),
        );
    });

    ui.add_space(4.0);
    let cur_tab_idx = match state.results_tab {
        ResultsStudioTab::Assumptions => 0,
        ResultsStudioTab::Guidance => 1,
        ResultsStudioTab::Methods => 2,
    };
    let tabs = [
        ("Assumptions", "inspector.results.tab.assumptions", tr(locale, "results.assumptions"), ResultsStudioTab::Assumptions),
        ("Guidance", "inspector.results.tab.guidance", tr(locale, "results.narrative"), ResultsStudioTab::Guidance),
        ("Methods", "inspector.results.tab.methods", tr(locale, "results.methods"), ResultsStudioTab::Methods),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let per_w = ((ui.available_width() - 8.0) / 3.0).max(52.0);
        for (idx, (_, wid, lbl, tab_val)) in tabs.into_iter().enumerate() {
            if studio_chip(ui, tokens, lbl, lbl, per_w, cur_tab_idx == idx, ctx_in.enabled, None, meta, wid, "tab", None, None) {
                state.results_tab = tab_val;
            }
        }
    });

    ui.add_space(4.0);
    match state.results_tab {
        ResultsStudioTab::Assumptions => {
            ui.add(egui::Label::new(egui::RichText::new(tr(locale, "results.diagnostics_only")).size(10.5).color(tokens.text_muted)).wrap());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if studio_chip(
                    ui,
                    tokens,
                    tr(locale, "results.filter_all"),
                    "Show all assumption checks",
                    90.0,
                    !state.results_severity_flagged_only,
                    ctx_in.enabled,
                    None,
                    meta,
                    "inspector.results.filter_all",
                    "button",
                    None,
                    None,
                ) {
                    state.results_severity_flagged_only = false;
                }
                if studio_chip(
                    ui,
                    tokens,
                    tr(locale, "results.filter_flagged"),
                    "Show warnings and violations only",
                    90.0,
                    state.results_severity_flagged_only,
                    ctx_in.enabled,
                    None,
                    meta,
                    "inspector.results.filter_flagged",
                    "button",
                    None,
                    None,
                ) {
                    state.results_severity_flagged_only = true;
                }
            });
            ui.add_space(4.0);
            let mut shown = 0usize;
            for item in &rs.checklist {
                if state.results_severity_flagged_only && !matches!(item.status, ChecklistStatus::Warn | ChecklistStatus::Violated) {
                    continue;
                }
                shown += 1;
                let (tag, clr) = checklist_badge(item.status, tokens);
                let safe_text = sanitize_diagnostics_only_text(item);
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.divider))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.horizontal_wrapped(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(tag).size(10.0).strong().color(clr)));
                            ui.add(egui::Label::new(egui::RichText::new(&item.assumption).size(11.0).strong().color(tokens.text_strong)).wrap());
                        });
                        if let Some(metric) = &item.diagnostic_metric {
                            ui.add(egui::Label::new(egui::RichText::new(metric).size(10.5).color(tokens.text_primary)).wrap());
                        }
                        ui.add(egui::Label::new(egui::RichText::new(safe_text).size(10.5).color(tokens.text_muted)).wrap());
                    });
                ui.add_space(3.0);
            }
            if shown == 0 {
                widgets::dim_label(ui, tr(locale, "results.no_checklist"));
            }
        }
        ResultsStudioTab::Guidance => {
            ui.add(egui::Label::new(egui::RichText::new(tr(locale, "results.diagnostics_only")).size(10.5).color(tokens.text_muted)).wrap());
            ui.add_space(4.0);
            let src_title = doc_sheet_title(doc, &rs.analysis.source_sheet_id).unwrap_or(rs.analysis.source_sheet_id.as_str());
            ui.add(egui::Label::new(egui::RichText::new(format!("Analysis: {}", rs.title())).size(11.5).strong().color(tokens.text_strong)).wrap());
            widgets::dim_label(ui, &format!("Source table: {src_title} • Pairwise comparisons: {}", rs.pairwise_comparisons.len()));
            if !rs.scalars.is_empty() {
                ui.add_space(4.0);
                widgets::subheader(ui, "Key Diagnostic Scalars");
                for scalar in rs.scalars.iter().take(6) {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&scalar.key).size(11.0).color(tokens.text_muted)).wrap());
                        ui.add(
                            egui::Label::new(egui::RichText::new(format!("{:.4}", scalar.value)).size(11.0).strong().color(tokens.text_primary))
                                .wrap(),
                        );
                    });
                }
            }
        }
        ResultsStudioTab::Methods => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (slug, lbl) in [("plain", "Txt"), ("markdown", "MD"), ("latex", "TeX")] {
                    let wid = format!("inspector.methods.fmt.{slug}");
                    let sel = state.methods_format == slug;
                    if studio_chip(ui, tokens, lbl, "Methods export format", 48.0, sel, ctx_in.enabled, None, meta, &wid, "button", None, None) {
                        state.methods_format = String::from(slug);
                    }
                }
            });
            ui.add_space(4.0);
            let report = resolve_methods_paragraph(doc, app, rs, state);
            if studio_full_button(
                ui,
                tokens,
                tr(locale, "results.copy_methods"),
                "Copy publication methods paragraph to clipboard",
                false,
                ctx_in.enabled,
                true,
                meta,
                "inspector.copy_methods",
                Some("ui.result.copyMethodsReport"),
                None,
            ) {
                state.last_copied_methods = true;
                state.last_copied_methods_text = report.clone();
                ui.ctx().copy_text(state.last_copied_methods_text.clone());
                intents.push_ui("ui.result.copyMethodsReport", json!({ "result_sheet_id": rs.id.as_str(), "format": state.methods_format }));
            }
            if state.last_copied_methods {
                ui.label(egui::RichText::new("Methods paragraph copied").size(10.5).color(tokens.link_green));
            }
            ui.add_space(4.0);
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.divider))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("inspector.methods.preview_scroll").max_height(180.0).show(ui, |ui| {
                        let mut preview = report;
                        ui.add(egui::TextEdit::multiline(&mut preview).desired_width(ui.available_width()).desired_rows(6).interactive(false));
                    });
                });
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        tr(locale, "results.parameters"),
        "Edit upstream analysis parameters (Cmd/Ctrl+T)",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.results.edit_params",
        Some("ui.editParams.open"),
        None,
    ) {
        intents.push_ui("ui.editParams.open", json!({ "result_sheet_id": rs.id.as_str() }));
    }
}

fn series_primary_color(series: &SeriesSpec) -> Rgba8 {
    match &series.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.fill_color
            } else if let Some(l) = line {
                l.color
            } else {
                Rgba8::BLACK
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => *fill_color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color,
        GraphicalDataObject::PartsOfWhole { .. } => Rgba8::BLACK,
    }
}

fn set_series_primary_color(series: &mut SeriesSpec, color: Rgba8) {
    match &mut series.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
            if let Some(l) = line {
                l.color = color;
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = color.with_alpha(200);
            *border_color = color;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = color.with_alpha(180);
            *stroke_color = color;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = color;
            marker.stroke_color = color;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = color;
            if let Some(m) = marker {
                m.fill_color = color;
                m.stroke_color = color;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color = color,
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn set_series_marker_shape(series: &mut SeriesSpec, shape: MarkerShape) -> bool {
    let apply_marker = |opt: &mut MarkerStyle| {
        opt.shape = shape;
    };
    match &mut series.gdo {
        GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } | GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => {
            apply_marker(m);
            true
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            apply_marker(marker);
            true
        }
        _ => false,
    }
}

fn render_graph_inspector_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    gs: &GraphSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let doc = app.document();

    let tab_defs = [
        ("Data", "inspector.graph.tab.data", GraphStudioTab::Data),
        ("Axes", "inspector.graph.tab.axes", GraphStudioTab::Axes),
        ("Leg", "inspector.graph.tab.legends", GraphStudioTab::Legends),
        ("Annot", "inspector.graph.tab.annotations", GraphStudioTab::Annotations),
        ("Canvas", "inspector.graph.tab.canvas", GraphStudioTab::Canvas),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let per_w = ((ui.available_width() - 12.0) / 5.0).max(34.0);
        for (lbl, wid, tab_kind) in tab_defs {
            if studio_chip(ui, tokens, lbl, lbl, per_w, state.graph_tab == tab_kind, ctx_in.enabled, None, meta, wid, "tab", None, None) {
                state.graph_tab = tab_kind;
            }
        }
    });
    ui.add_space(4.0);

    let series_count = gs.spec.series.len();
    if series_count > 0 && state.graph_series_index >= series_count {
        state.graph_series_index = 0;
    }
    let sel_idx = state.graph_series_index;

    match state.graph_tab {
        GraphStudioTab::Data => {
            widgets::subheader(ui, "Series & Error Bars");
            if series_count > 0 {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for (idx, s) in gs.spec.series.iter().enumerate() {
                        let lbl = s.legend_label.clone().unwrap_or_else(|| format!("Series {}", idx + 1));
                        let rgba = series_primary_color(s);
                        let swatch = Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, 255);
                        let wid = format!("inspector.graph.series.{idx}");
                        if studio_chip(ui, tokens, &lbl, &lbl, 110.0, idx == sel_idx, ctx_in.enabled, Some(swatch), meta, &wid, "button", None, None)
                        {
                            state.graph_series_index = idx;
                        }
                    }
                });
            }

            ui.add_space(4.0);
            widgets::subheader(ui, "Palette Preset");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (pal, lbl) in [
                    (PaletteId::OkabeIto, "Okabe-Ito"),
                    (PaletteId::PaulTolBright, "Tol Bright"),
                    (PaletteId::PaulTolMuted, "Tol Muted"),
                    (PaletteId::MonochromePrint, "Mono"),
                ] {
                    let wid = format!("inspector.graph.palette.{pal:?}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        pal.label(),
                        86.0,
                        gs.spec.palette == pal,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("graph.setSpec"),
                        None,
                    ) {
                        let mut next_series = gs.spec.series.clone();
                        for (i, s) in next_series.iter_mut().enumerate() {
                            set_series_primary_color(s, pal.color_at(i));
                        }
                        intents
                            .push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "palette": pal, "series": next_series } }));
                    }
                }
            });

            if let Some(series) = gs.spec.series.get(sel_idx) {
                ui.add_space(4.0);
                widgets::subheader(ui, "Symbol Shape");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for (shape, lbl) in
                        [(MarkerShape::Circle, "●"), (MarkerShape::Square, "■"), (MarkerShape::TriangleUp, "▲"), (MarkerShape::Diamond, "◆")]
                    {
                        let wid = format!("inspector.graph.shape.{shape:?}");
                        if studio_chip(
                            ui,
                            tokens,
                            lbl,
                            "Set series marker shape",
                            34.0,
                            false,
                            ctx_in.enabled,
                            None,
                            meta,
                            &wid,
                            "button",
                            Some("graph.setSeries"),
                            None,
                        ) {
                            let mut cloned = series.clone();
                            if set_series_marker_shape(&mut cloned, shape) {
                                intents.push_engine(
                                    "graph.setSeries",
                                    json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": cloned.gdo }),
                                );
                            }
                        }
                    }
                });

                ui.add_space(4.0);
                widgets::subheader(ui, "Y Error Bars");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    let cur_metric = series.y_error_bars.map(|e| e.metric);
                    for (metric, lbl) in [
                        (None, "Off"),
                        (Some(ErrorBarMetric::Sd), "SD"),
                        (Some(ErrorBarMetric::Sem), "SEM"),
                        (Some(ErrorBarMetric::ConfidenceInterval95), "95% CI"),
                        (Some(ErrorBarMetric::Range), "Range"),
                    ] {
                        let wid = format!("inspector.graph.eb.{lbl}");
                        if studio_chip(
                            ui,
                            tokens,
                            lbl,
                            "Error bar dispersion metric",
                            62.0,
                            cur_metric == metric,
                            ctx_in.enabled,
                            None,
                            meta,
                            &wid,
                            "button",
                            Some("graph.setSeries"),
                            None,
                        ) {
                            let val = match metric {
                                None => serde_json::Value::Null,
                                Some(m) => json!(ErrorBarSpec {
                                    metric: m,
                                    direction: series.y_error_bars.map_or(ErrorBarDirection::Both, |e| e.direction),
                                    ..series.y_error_bars.unwrap_or_default()
                                }),
                            };
                            intents
                                .push_engine("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": val }));
                        }
                    }
                });
            }

            if gs.spec.kind == curvebook_doc::GraphKind::Grouped {
                ui.add_space(4.0);
                widgets::subheader(ui, "Grouped Arrangement");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for (arr, lbl) in [
                        (GroupedArrangement::Interleaved, "Interleaved"),
                        (GroupedArrangement::Separated, "Separated"),
                        (GroupedArrangement::Stacked, "Stacked"),
                        (GroupedArrangement::Superimposed, "Superimposed"),
                    ] {
                        let wid = format!("inspector.graph.arr.{arr:?}");
                        if studio_chip(
                            ui,
                            tokens,
                            lbl,
                            "Grouped bar/point cluster arrangement",
                            92.0,
                            gs.spec.grouped_arrangement == arr,
                            ctx_in.enabled,
                            None,
                            meta,
                            &wid,
                            "button",
                            Some("graph.setSpec"),
                            None,
                        ) {
                            intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "grouped_arrangement": arr } }));
                        }
                    }
                });
            }

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                "More… Format Graph",
                "Open Format Graph dialog",
                false,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.more_format",
                Some("ui.graph.format.open"),
                None,
            ) {
                intents.push_ui("ui.graph.format.open", json!({ "sheet_id": gs.id.as_str() }));
            }
            if studio_full_button(
                ui,
                tokens,
                "Bubble & MV Encoding…",
                "Open Multivariate Bubble Plot settings",
                false,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.more_bubble",
                Some("ui.graph.bubblePlot.open"),
                None,
            ) {
                intents.push_ui("ui.graph.bubblePlot.open", json!({ "sheet_id": gs.id.as_str() }));
            }
            if studio_full_button(
                ui,
                tokens,
                "Apply Style From…",
                "Clone graph styling from another sheet",
                false,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.apply_style",
                Some("ui.applyStyleFrom.open"),
                None,
            ) {
                intents.push_ui("ui.applyStyleFrom.open", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
        GraphStudioTab::Axes => {
            widgets::subheader(ui, "Axis & Frame");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (idx, lbl) in [(0u8, "X"), (1, "Y"), (2, "Y2")] {
                    let wid = format!("inspector.graph.axes.target.{lbl}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Select target axis",
                        54.0,
                        state.graph_axis_target == idx,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        None,
                        None,
                    ) {
                        state.graph_axis_target = idx;
                    }
                }
            });

            let target_axis: &AxisSpec = match state.graph_axis_target {
                0 => &gs.spec.x_axis,
                2 => gs.spec.y2_axis.as_ref().unwrap_or(&gs.spec.y_axis),
                _ => &gs.spec.y_axis,
            };
            let cur_scale = target_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
            let axis_key = match state.graph_axis_target {
                0 => "x_axis",
                2 => "y2_axis",
                _ => "y_axis",
            };

            ui.add_space(4.0);
            widgets::dim_label(ui, "Scale Transform");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (sc, lbl) in [(AxisScale::Linear, "Linear"), (AxisScale::Log10, "Log10"), (AxisScale::Log2, "Log2"), (AxisScale::Ln, "Ln")] {
                    let wid = format!("inspector.graph.scale.{lbl}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Set axis scale transform",
                        56.0,
                        cur_scale == sc,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("graph.setAxes"),
                        None,
                    ) {
                        let mut next_axis = target_axis.clone();
                        for seg in &mut next_axis.segments {
                            seg.scale = sc;
                        }
                        let mut payload = serde_json::Map::new();
                        payload.insert(String::from("sheet_id"), json!(gs.id.as_str()));
                        payload.insert(String::from(axis_key), json!(next_axis));
                        intents.push_engine("graph.setAxes", serde_json::Value::Object(payload));
                    }
                }
            });

            ui.add_space(4.0);
            widgets::dim_label(ui, "Frame Preset");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (fp, lbl) in [
                    (FramePreset::OpenL, "Open L"),
                    (FramePreset::OffsetRangeTufte, "Tufte"),
                    (FramePreset::PlainBoxed, "Boxed"),
                    (FramePreset::DualYSecondaryRight, "Dual Y"),
                ] {
                    let wid = format!("inspector.graph.frame.{fp:?}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Set frame spine arrangement",
                        68.0,
                        gs.spec.frame_preset == fp,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("graph.setAxes"),
                        None,
                    ) {
                        intents.push_engine("graph.setAxes", json!({ "sheet_id": gs.id.as_str(), "frame_preset": fp }));
                    }
                }
            });

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                "More… Format Axes",
                "Open Format Axes dialog",
                false,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.more_axes",
                Some("ui.graph.axes.open"),
                None,
            ) {
                intents.push_ui("ui.graph.axes.open", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
        GraphStudioTab::Legends => {
            widgets::subheader(ui, "Legend Configuration");
            let mut legend = gs.spec.legend;
            if studio_full_button(
                ui,
                tokens,
                if legend.visible { "Legend: Visible" } else { "Legend: Hidden" },
                "Toggle legend visibility on graph",
                legend.visible,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.legend_visible",
                Some("graph.setSpec"),
                None,
            ) {
                legend.visible = !legend.visible;
                intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "legend": legend } }));
            }
            if studio_full_button(
                ui,
                tokens,
                if legend.bordered { "Border Box: On" } else { "Border Box: Off" },
                "Toggle legend frame border",
                legend.bordered,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.legend_border",
                Some("graph.setSpec"),
                None,
            ) {
                legend.bordered = !legend.bordered;
                intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "legend": legend } }));
            }

            ui.add_space(6.0);
            widgets::subheader(ui, "Pairwise Significance & CLD");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (mode, lbl) in [
                    (PairwiseDisplayMode::BracketsOnly, "Brackets"),
                    (PairwiseDisplayMode::CompactLetterDisplayOnly, "CLD"),
                    (PairwiseDisplayMode::BracketsAndCompactLetterDisplay, "Both"),
                ] {
                    let wid = format!("inspector.graph.pw.{lbl}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Set pairwise significance display mode",
                        68.0,
                        gs.spec.pairwise_display_mode == mode,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("graph.setSpec"),
                        None,
                    ) {
                        intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "pairwise_display_mode": mode } }));
                    }
                }
            });

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                "More… Pairwise & CLD",
                "Open Pairwise Significance & CLD dialog",
                false,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.more_pairwise",
                Some("ui.graph.pairwise.open"),
                None,
            ) {
                intents.push_ui("ui.graph.pairwise.open", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
        GraphStudioTab::Annotations => {
            widgets::subheader(ui, &format!("Annotations ({})", gs.spec.annotations.len()));
            if gs.spec.annotations.is_empty() {
                widgets::dim_label(ui, "No annotations on this graph yet.");
            } else {
                for (idx, ann) in gs.spec.annotations.iter().enumerate() {
                    let summary = match ann {
                        GraphAnnotation::TextBox { markdown, .. } => format!("Text: {markdown}"),
                        GraphAnnotation::LineOrArrow { arrowhead, .. } => {
                            if *arrowhead {
                                String::from("Arrow")
                            } else {
                                String::from("Line")
                            }
                        }
                        GraphAnnotation::Shape { kind, .. } => format!("Shape: {kind:?}"),
                        GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } => format!("Table: {}", result_sheet_id.as_str()),
                        GraphAnnotation::ModelEquation { result_sheet_id, .. } => format!("Equation: {}", result_sheet_id.as_str()),
                    };
                    let wid = format!("inspector.graph.annot.remove.{idx}");
                    if studio_full_button(
                        ui,
                        tokens,
                        &format!("✕ {summary}"),
                        "Remove annotation",
                        false,
                        ctx_in.enabled,
                        false,
                        meta,
                        &wid,
                        Some("graph.setSpec"),
                        None,
                    ) {
                        let mut next_annots = gs.spec.annotations.clone();
                        next_annots.remove(idx);
                        intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "annotations": next_annots } }));
                    }
                }
            }
            ui.add_space(6.0);
            if studio_full_button(
                ui,
                tokens,
                "+ Add Model Equation…",
                "Insert live regression equation callout",
                false,
                ctx_in.enabled && !doc.result_sheets.is_empty(),
                false,
                meta,
                "inspector.graph.add_equation",
                Some("ui.graph.insertModelEquation"),
                doc.result_sheets.is_empty().then_some("no result sheets available"),
            ) {
                intents.push_ui("ui.graph.insertModelEquation", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
        GraphStudioTab::Canvas => {
            widgets::subheader(ui, "Canvas & Publication Presets");
            widgets::dim_label(ui, &format!("{:.1} × {:.1} pt", gs.spec.canvas_size_pt.width, gs.spec.canvas_size_pt.height));

            ui.add_space(4.0);
            let paper_on = gs.spec.paper_fill.is_some();
            if studio_full_button(
                ui,
                tokens,
                if paper_on { "Background: White Paper" } else { "Background: Transparent" },
                "Toggle solid white paper fill vs transparent background",
                paper_on,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.canvas_bg",
                Some("graph.setSpec"),
                None,
            ) {
                let next_fill = if paper_on { serde_json::Value::Null } else { json!(Rgba8::WHITE) };
                intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "paper_fill": next_fill } }));
            }

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (preset, lbl) in
                    [("SingleColumn88mm", "88 mm"), ("ColumnAndHalf130mm", "130 mm"), ("DoubleColumn180mm", "180 mm"), ("Slide16x9", "16:9 Slide")]
                {
                    let wid = format!("inspector.graph.canvas_preset.{preset}");
                    if studio_chip(
                        ui,
                        tokens,
                        lbl,
                        "Apply publication canvas preset",
                        72.0,
                        false,
                        ctx_in.enabled,
                        None,
                        meta,
                        &wid,
                        "button",
                        Some("graph.applyCanvasPreset"),
                        None,
                    ) {
                        intents.push_engine("graph.applyCanvasPreset", json!({ "sheet_id": gs.id.as_str(), "preset": preset }));
                    }
                }
            });

            ui.add_space(4.0);
            let is_gray = app.preview_filter == CanvasPreviewFilter::GrayscalePrint;
            if studio_full_button(
                ui,
                tokens,
                if is_gray { "Grayscale Print Preview: On" } else { "Grayscale Print Preview: Off" },
                "Toggle grayscale publication readability preview",
                is_gray,
                ctx_in.enabled,
                false,
                meta,
                "inspector.graph.grayscale_toggle",
                Some("ui.previewFilter.set"),
                None,
            ) {
                intents.push_set_preview_filter(if is_gray { CanvasPreviewFilter::Normal } else { CanvasPreviewFilter::GrayscalePrint });
            }

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                "More… Export Figure",
                "Open Figure Export dialog (PDF / SVG / PNG)",
                false,
                ctx_in.enabled,
                true,
                meta,
                "inspector.graph.export_fig",
                Some("ui.export.figure.open"),
                None,
            ) {
                intents.push_ui("ui.export.figure.open", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
    }
}

fn ensure_drill_points_cache<'a>(app: &UiApp, gs: &GraphSheet, state: &'a mut TrailingStudioEguiState) -> &'a [DrillBackPointEntry] {
    let seq = app.mutation_seq();
    let hit = state.drill_cache_key.as_ref().is_some_and(|(sid, s)| sid == &gs.id && *s == seq);
    if !hit {
        state.drill_cache_key = Some((gs.id.clone(), seq));
        state.drill_cached_points = app
            .session()
            .render_sheet_scene(&gs.id)
            .map(|scene| {
                scene
                    .plotted_points
                    .into_iter()
                    .map(|p| DrillBackPointEntry {
                        row: p.source_cell.row,
                        col: p.source_cell.col,
                        subcol: p.source_cell.subcol,
                        series_label: p.series_label,
                        value_summary: p.value_summary,
                        x_pt_tenths: (p.center_pt.x * 10.0).round() as i64,
                        y_pt_tenths: (p.center_pt.y * 10.0).round() as i64,
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    &state.drill_cached_points
}

fn render_data_point_drill_back_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    gs: &GraphSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let doc = app.document();

    if studio_full_button(
        ui,
        tokens,
        "← Back to Graph Inspector",
        "Clear brushed point selection",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.drill_back.back",
        None,
        None,
    ) {
        intents.push_clear_brush();
    }

    let Some(brushed) = app.brushed_cell.clone() else {
        widgets::dim_label(ui, "Click any plotted point to inspect its source cell.");
        return;
    };

    let points = ensure_drill_points_cache(app, gs, state);
    let matched_pt = points.iter().find(|p| p.row == brushed.row && p.col == brushed.col && p.subcol == brushed.subcol);

    let src_ds = doc.data_sheet(&brushed.sheet_id);
    let src_title = src_ds.map_or(brushed.sheet_id.as_str(), |d| d.title.as_str());
    let cell_opt = src_ds.and_then(|d| d.cell(curvebook_doc::CellCoord::new(brushed.row, brushed.col, brushed.subcol)));
    let currently_excluded = cell_opt.is_some_and(|c| c.excluded);
    let raw_val = cell_opt.map_or(String::from("—"), |c| match &c.value {
        curvebook_doc::CellValue::Empty => String::from("(empty)"),
        curvebook_doc::CellValue::Number(v) => format!("{v:.6}"),
        curvebook_doc::CellValue::Text(t) => t.clone(),
    });

    ui.add_space(4.0);
    egui::Frame::NONE.fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.divider)).corner_radius(5.0).inner_margin(egui::Margin::same(8)).show(
        ui,
        |ui| {
            ui.label(egui::RichText::new(src_title).size(12.0).strong().color(tokens.text_strong));
            ui.label(
                egui::RichText::new(format!("Row {}  •  Col {}  •  Sub {}", brushed.row + 1, brushed.col + 1, brushed.subcol + 1))
                    .size(11.0)
                    .color(tokens.text_primary),
            );
            ui.label(egui::RichText::new(format!("Value: {raw_val}")).size(11.0).color(tokens.text_primary));
            if let Some(pt) = matched_pt {
                ui.label(egui::RichText::new(format!("Series: {}", pt.series_label)).size(11.0).color(tokens.text_muted));
                ui.label(egui::RichText::new(pt.value_summary.clone()).size(11.0).color(tokens.text_primary));
                ui.label(
                    egui::RichText::new(format!("Canvas: ({:.1}, {:.1}) pt", (pt.x_pt_tenths as f64) * 0.1, (pt.y_pt_tenths as f64) * 0.1))
                        .size(10.5)
                        .color(tokens.text_muted),
                );
            }
            if currently_excluded {
                ui.label(egui::RichText::new("Status: Excluded from analyses").size(11.0).strong().color(tokens.link_red));
            }
        },
    );

    ui.add_space(6.0);
    let ds_frozen = src_ds.is_some_and(|d| d.frozen);
    let exclude_lbl = if currently_excluded { "Include Point" } else { "Exclude Point" };
    if studio_full_button(
        ui,
        tokens,
        exclude_lbl,
        "Toggle exclusion flag for this datum cell (table.excludeCells)",
        currently_excluded,
        ctx_in.enabled && !ds_frozen,
        false,
        meta,
        "inspector.drill_back.toggle_exclude",
        Some("table.excludeCells"),
        ds_frozen.then_some("source sheet is frozen"),
    ) {
        intents.push_engine(
            "table.excludeCells",
            json!({
                "sheet_id": brushed.sheet_id.as_str(),
                "row": brushed.row,
                "col": brushed.col,
                "subcol": brushed.subcol,
                "excluded": !currently_excluded,
            }),
        );
    }

    if studio_full_button(
        ui,
        tokens,
        "Format Selected Point…",
        "Customize individual point color, shape, and size",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.drill_back.format_point",
        Some("ui.formatPoints.open"),
        None,
    ) {
        intents.push_ui(
            "ui.formatPoints.open",
            json!({
                "sheet_id": gs.id.as_str(),
                "points": [[brushed.col, brushed.row]],
            }),
        );
    }

    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        tr(tokens.locale, "inspector.drill_back.go"),
        "Navigate to source data table and select exact cell",
        false,
        ctx_in.enabled,
        true,
        meta,
        "inspector.drill_back.jump",
        Some("sheet.select"),
        None,
    ) {
        intents.push_engine("sheet.select", json!({ "sheet_id": brushed.sheet_id.as_str() }));
        intents.push_select_grid_cell(GridCellCoord::new(brushed.row, brushed.col, brushed.subcol));
    }
}

fn render_page_layout_composer_studio(
    ui: &mut egui::Ui,
    ls: &LayoutSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let spec = &ls.spec;

    ui.label(egui::RichText::new(format!("{:.1} × {:.1} {:?}", spec.width, spec.height, spec.unit)).size(11.5).strong().color(tokens.text_strong));

    ui.add_space(4.0);
    widgets::subheader(ui, "Journal Column Presets");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let is_85 = (spec.width - 85.0).abs() < 0.5;
        let is_114 = (spec.width - 114.0).abs() < 0.5;
        let is_174 = (spec.width - 174.0).abs() < 0.5;

        if studio_chip(
            ui,
            tokens,
            "85 mm",
            "Single-column journal width (85 mm)",
            68.0,
            is_85,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.preset.85mm",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "85mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            "114 mm",
            "1.5-column journal width (114 mm)",
            68.0,
            is_114,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.preset.114mm",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "width": 114.0, "height": 90.0, "unit": "mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            "174 mm",
            "Double-column journal width (174 mm)",
            68.0,
            is_174,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.preset.174mm",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "174mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            "A4",
            "ISO A4 portrait (210 × 297 mm)",
            48.0,
            false,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.preset.a4",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "a4_portrait" }));
        }
        if studio_chip(
            ui,
            tokens,
            "Letter",
            "US Letter portrait (8.5 × 11 in)",
            58.0,
            false,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.preset.letter",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "letter_portrait" }));
        }
    });

    ui.add_space(4.0);
    widgets::subheader(ui, "Orientation & Margins");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let is_port = spec.orientation == PageOrientation::Portrait;
        if studio_chip(
            ui,
            tokens,
            "Portrait",
            "Portrait page orientation",
            74.0,
            is_port,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.orient.portrait",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "orientation": "portrait" }));
        }
        if studio_chip(
            ui,
            tokens,
            "Landscape",
            "Landscape page orientation",
            84.0,
            !is_port,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.orient.landscape",
            "button",
            Some("layout.setSpec"),
            None,
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "orientation": "landscape" }));
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new(tr(locale, "inspector.layout.margins")).size(11.0).color(tokens.text_muted));
        if studio_chip(
            ui,
            tokens,
            "-",
            "Decrease page margin (mm)",
            28.0,
            false,
            ctx_in.enabled && state.layout_margin_mm > 2,
            None,
            meta,
            "inspector.layout.margin.dec",
            "button",
            None,
            None,
        ) {
            state.layout_margin_mm = state.layout_margin_mm.saturating_sub(2).max(2);
        }
        ui.label(egui::RichText::new(format!("{} mm", state.layout_margin_mm)).size(11.0).strong().color(tokens.text_primary));
        if studio_chip(
            ui,
            tokens,
            "+",
            "Increase page margin (mm)",
            28.0,
            false,
            ctx_in.enabled && state.layout_margin_mm < 40,
            None,
            meta,
            "inspector.layout.margin.inc",
            "button",
            None,
            None,
        ) {
            state.layout_margin_mm = (state.layout_margin_mm + 2).min(40);
        }
    });

    ui.add_space(4.0);
    widgets::subheader(ui, "DPI & Snap");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for dpi in [300u16, 600, 1200] {
            let wid = format!("inspector.layout.dpi.{dpi}");
            if studio_chip(
                ui,
                tokens,
                &format!("{dpi} DPI"),
                "Target rasterization / print DPI",
                64.0,
                state.layout_dpi == dpi,
                ctx_in.enabled,
                None,
                meta,
                &wid,
                "button",
                None,
                None,
            ) {
                state.layout_dpi = dpi;
            }
        }
        let snap_lbl = if state.snap_enabled { "Snap: On" } else { "Snap: Off" };
        if studio_chip(
            ui,
            tokens,
            snap_lbl,
            "Toggle panel snap-to-grid guides",
            68.0,
            state.snap_enabled,
            ctx_in.enabled,
            None,
            meta,
            "inspector.layout.snap",
            "button",
            None,
            None,
        ) {
            state.snap_enabled = !state.snap_enabled;
        }
    });

    ui.add_space(4.0);
    widgets::subheader(ui, tr(locale, "inspector.layout.grid"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (rows, cols, lbl) in [(1u8, 1u8, "1×1"), (1, 2, "1×2"), (2, 1, "2×1"), (2, 2, "2×2"), (2, 3, "2×3")] {
            let wid = format!("inspector.layout.grid.{rows}x{cols}");
            let sel = spec.grid_template == Some((rows, cols));
            if studio_chip(
                ui,
                tokens,
                lbl,
                "Panel arrangement grid template",
                46.0,
                sel,
                ctx_in.enabled,
                None,
                meta,
                &wid,
                "button",
                Some("layout.setSpec"),
                None,
            ) {
                intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "grid_template": [rows, cols] }));
            }
        }
    });

    ui.add_space(4.0);
    widgets::subheader(ui, "Panel Alignment & Badges");
    if studio_full_button(
        ui,
        tokens,
        tr(locale, "inspector.layout.align_origins"),
        "Align adjacent plot origins across panels",
        spec.align_plot_origins,
        ctx_in.enabled,
        false,
        meta,
        "inspector.layout.align_origins",
        Some("layout.alignOrigins"),
        None,
    ) {
        intents
            .push_engine("layout.alignOrigins", json!({ "sheet_id": ls.id.as_str(), "align": !spec.align_plot_origins, "normalize_bounds": true }));
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (style, slug, lbl) in [
            (PanelBadgeStyle::UpperCaseLetters, "uppercase", "A, B"),
            (PanelBadgeStyle::LowerCaseLetters, "lowercase", "a, b"),
            (PanelBadgeStyle::None, "none", "None"),
        ] {
            let wid = format!("inspector.layout.badge.{slug}");
            if studio_chip(
                ui,
                tokens,
                lbl,
                "Panel badge lettering",
                54.0,
                spec.panel_badges == style,
                ctx_in.enabled,
                None,
                meta,
                &wid,
                "button",
                Some("layout.setSpec"),
                None,
            ) {
                intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "panel_badges": slug }));
            }
        }
    });

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        "More… Layout Composer",
        "Open full Page Layout Composer dialog",
        false,
        ctx_in.enabled,
        false,
        meta,
        "inspector.layout.more_composer",
        Some("ui.layout.composer.open"),
        None,
    ) {
        intents.push_ui("ui.layout.composer.open", json!({ "sheet_id": ls.id.as_str() }));
    }
    if studio_full_button(
        ui,
        tokens,
        "Export Figure…",
        "Export layout page to PDF, SVG, or PNG",
        false,
        ctx_in.enabled,
        true,
        meta,
        "inspector.layout.export_fig",
        Some("ui.export.figure.open"),
        None,
    ) {
        intents.push_ui("ui.export.figure.open", json!({ "sheet_id": ls.id.as_str(), "dpi": state.layout_dpi }));
    }
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
                let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), egui::FontId::proportional(13.0), ctx_in.tokens.text_strong);
                job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
                job.wrap.max_width = title_budget;
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let elided = galley.elided;
                let (t_rect, t_resp) = ui.allocate_exact_size(Vec2::new(title_budget, 22.0), Sense::hover());
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
                        if is_rtl_locale(ctx_in.tokens.locale) { "«" } else { "»" },
                        "Collapse Studio (Cmd/Ctrl+Shift+I)",
                        24.0,
                        false,
                        ctx_in.enabled,
                        None,
                        meta,
                        "inspector.collapse_rail",
                        "button",
                        Some("ui.inspector.toggle"),
                        None,
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
                                render_page_layout_composer_studio(ui, ls, state, ctx_in, meta, intents);
                            }
                        }
                    }
                });
            });
        })
        .response
        .rect;

    report.dock_rect_pt = Some(resp);
    report
}
