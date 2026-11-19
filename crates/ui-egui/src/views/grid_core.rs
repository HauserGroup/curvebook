//! Core virtualized data-grid view builder and interaction shard (`R10.8`, `R10.12`, `R10.22`).
//!
//! Renders only the visible `(row, col)` window so `1M`-cell tables stay well within the
//! `16.67 ms` frame budget, while providing 3-tier column headers (identifier/super-span
//! band, editable dataset/column/X/row title row, replicate/summary subcolumn headers),
//! a collapsible & resizable `Row Titles` column next to the `#` row-number gutter,
//! 2-axis `ScrollAxes::Both` scrolling, keyboard navigation & clear targets, colour-coded
//! `LinkedDataRegion` provenance borders with `Go To Source` / `Unlink` popover and
//! read-only typing guard, and excluded cell italic/strikethrough/note-tag styling with
//! hover provenance tooltips.

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{CellCoord, DataSheet, LinkBorderColor, LinkedDataRegion, SheetId, TableArchetype, XColumnMode};
use curvebook_palette::Rgba8;
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::a11y::{grid_cell_meta, labeled_meta, selectable_meta};
use super::mv_formula;
use crate::UiApp;
use crate::compositor::WorkspacePixelRect;
use crate::grid_view::{
    MoveDirection, data_set_subcolumn_count, format_cell_display, provenance_border_rgba, resolve_flat_coord, total_flat_data_subcolumns,
};
use crate::i18n::{LocalePreset, mirror_x_for_rtl, normalize_locale, try_tr};
use crate::interaction::{GridHitGeometry, KeyModifiers, UiEventOutcome};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldEditState, TextFieldSpec, emit_button, emit_scrollbars_for_meta, emit_text_field, paint_text_fitted,
    widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle};

const DEFAULT_ROW_HEIGHT_PX: u32 = 24;
const ROW_NUM_WIDTH_PX: u32 = 48;
const DEFAULT_ROW_TITLES_WIDTH_PX: u32 = 112;
const COLLAPSED_ROW_TITLES_WIDTH_PX: u32 = 34;
const MIN_ROW_TITLES_WIDTH_PX: u32 = 56;
const MAX_ROW_TITLES_WIDTH_PX: u32 = 360;
const DEFAULT_X_COL_WIDTH_PX: u32 = 96;
const DEFAULT_DATA_COL_WIDTH_PX: u32 = 96;
const MIN_COL_WIDTH_PX: u32 = 44;
const MAX_COL_WIDTH_PX: u32 = 420;
const TIER1_HEADER_H_PX: u32 = 20;
const TIER2_HEADER_H_PX: u32 = 24;
const TIER3_HEADER_H_PX: u32 = 20;
const LINK_STRIP_H_PX: u32 = 26;
const RESIZE_HANDLE_W_PX: u32 = 5;
const CORNER_PAD_PX: u32 = 2;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scale_u32(base: u32, scale: f64) -> u32 {
    let s = if scale.is_finite() && scale > 0.5 { scale.clamp(0.5, 4.0) } else { 1.0 };
    ((f64::from(base) * s).round() as u32).max(1)
}

fn scale_pt(base: f64, scale: f64) -> f64 {
    let s = if scale.is_finite() && scale > 0.5 { scale.clamp(0.5, 4.0) } else { 1.0 };
    (base * s).clamp(9.0, 28.0)
}

/// Target metadata for the inline title editor overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GridTitleTarget {
    /// X / Time column title.
    XColumn,
    /// Top-level data column / group title (`col` index).
    DataColumn { col: usize },
    /// Nested subcolumn title (`col`, `subcol`).
    Subcolumn { col: usize, subcol: usize },
    /// Individual row title (`row`).
    RowTitle { row: usize },
}

/// Active inline header or row-title editing state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridTitleEditState {
    pub sheet_id: String,
    pub target: GridTitleTarget,
    pub draft: String,
}

/// Active popover inspecting a [`LinkedDataRegion`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridLinkPopoverState {
    pub sheet_id: String,
    pub region_id: String,
}

/// Non-modal read-only prompt surfaced when a user attempts to edit inside a linked region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridLinkBlockPrompt {
    pub sheet_id: String,
    pub region_id: String,
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

/// Transient view state owned by the core data-grid shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridCoreViewState {
    pub row_titles_collapsed: bool,
    pub row_titles_width_px: u32,
    #[serde(default)]
    pub column_widths_px: BTreeMap<String, u32>,
    #[serde(default)]
    pub x_col_widths_px: BTreeMap<usize, u32>,
    #[serde(default)]
    pub editing_title: Option<GridTitleEditState>,
    #[serde(default)]
    pub active_link_popover: Option<GridLinkPopoverState>,
    #[serde(default)]
    pub link_edit_warning: Option<GridLinkBlockPrompt>,
    #[serde(default)]
    pub hovered_cell: Option<GridCellCoord>,
    #[serde(default)]
    pub last_nav_summary: Option<String>,
}

impl GridCoreViewState {
    /// Return the effective pixel width of the `Row Titles` column.
    #[must_use]
    pub fn effective_row_titles_width(&self) -> u32 {
        if self.row_titles_collapsed {
            COLLAPSED_ROW_TITLES_WIDTH_PX
        } else if self.row_titles_width_px == 0 {
            DEFAULT_ROW_TITLES_WIDTH_PX
        } else {
            self.row_titles_width_px.clamp(MIN_ROW_TITLES_WIDTH_PX, MAX_ROW_TITLES_WIDTH_PX)
        }
    }

    /// Return the effective pixel width of an X subcolumn.
    #[must_use]
    pub fn effective_x_col_width(&self, subcol: usize) -> u32 {
        self.x_col_widths_px.get(&subcol).copied().unwrap_or(DEFAULT_X_COL_WIDTH_PX).clamp(MIN_COL_WIDTH_PX, MAX_COL_WIDTH_PX)
    }

    /// Return the effective pixel width of data subcolumn `(col, subcol)`.
    #[must_use]
    pub fn effective_data_col_width(&self, col: usize, subcol: usize) -> u32 {
        let key = format!("{col}:{subcol}");
        self.column_widths_px.get(&key).copied().unwrap_or(DEFAULT_DATA_COL_WIDTH_PX).clamp(MIN_COL_WIDTH_PX, MAX_COL_WIDTH_PX)
    }

    /// Set the explicit pixel width of `Row Titles` (`56..=360`) and ensure it is expanded.
    pub fn set_row_titles_width(&mut self, width_px: u32) {
        self.row_titles_collapsed = false;
        self.row_titles_width_px = width_px.clamp(MIN_ROW_TITLES_WIDTH_PX, MAX_ROW_TITLES_WIDTH_PX);
    }

    /// Set the explicit pixel width of data subcolumn `(col, subcol)` (`44..=420`).
    pub fn set_data_col_width(&mut self, col: usize, subcol: usize, width_px: u32) {
        self.column_widths_px.insert(format!("{col}:{subcol}"), width_px.clamp(MIN_COL_WIDTH_PX, MAX_COL_WIDTH_PX));
    }

    /// Set the explicit pixel width of X subcolumn `subcol` (`44..=420`).
    pub fn set_x_col_width(&mut self, subcol: usize, width_px: u32) {
        self.x_col_widths_px.insert(subcol, width_px.clamp(MIN_COL_WIDTH_PX, MAX_COL_WIDTH_PX));
    }

    fn has_uniform_column_widths(&self) -> bool {
        self.column_widths_px.is_empty() && self.x_col_widths_px.is_empty()
    }
}

/// Resolve a `w5.grid_core.*` localization key without requiring static registration in `i18n.rs`.
#[must_use]
pub fn grid_tr(locale: &str, key: &str, fallback: &'static str) -> &'static str {
    let canonical = normalize_locale(locale);
    try_tr(canonical, key).or_else(|| try_tr("en", key)).unwrap_or(fallback)
}

/// Format an X/time cell value according to `preset`.
#[must_use]
pub fn x_cell_display(ds: &DataSheet, row: usize, subcol: usize, preset: &LocalePreset) -> String {
    if let Some(cell) = ds.x_or_time_cell(row, subcol) {
        return format_cell_display(cell, preset);
    }
    if let TableArchetype::Xy(xy) = &ds.archetype
        && let Some(val) = xy.x_value_at(row)
    {
        let cell = curvebook_doc::Cell::number(val);
        return format_cell_display(&cell, preset);
    }
    String::new()
}

fn active_data_sheet(app: &UiApp) -> Option<&DataSheet> {
    let doc = app.document();
    let sid = doc.active_sheet_id.as_ref()?;
    doc.data_sheet(sid)
}

/// Return the [`LinkedDataRegion`] containing domain cell `(row, col)`, if any.
#[must_use]
pub fn linked_region_for_cell(ds: &DataSheet, row: usize, col: usize) -> Option<&LinkedDataRegion> {
    ds.linked_regions.iter().find(|region| {
        row >= region.target_range.start.row
            && row <= region.target_range.end.row
            && col >= region.target_range.start.col
            && col <= region.target_range.end.col
    })
}

/// Return `true` and open the read-only linked-region prompt when `(row, col, subcol)` is linked.
pub fn check_edit_blocked_by_link(ds: &DataSheet, state: &mut GridCoreViewState, row: usize, col: usize, subcol: usize) -> bool {
    let Some(region) = linked_region_for_cell(ds, row, col) else {
        return false;
    };
    let sheet_id = ds.id.to_string();
    state.active_link_popover = Some(GridLinkPopoverState { sheet_id: sheet_id.clone(), region_id: region.id.clone() });
    state.link_edit_warning = Some(GridLinkBlockPrompt { sheet_id, region_id: region.id.clone(), row, col, subcol });
    true
}

/// Handle grid keyboard navigation, range extension, boundary jumping, Delete/Backspace clear,
/// linked-cell edit blocking, and inline title editor Enter/Escape confirmation.
///
/// # Errors
/// Returns an error string when an engine command fails.
pub fn handle_grid_key_down(app: &mut UiApp, state: &mut GridCoreViewState, key: &str, mods: KeyModifiers) -> Result<UiEventOutcome, String> {
    if state.editing_title.is_some() {
        let is_enter = key.eq_ignore_ascii_case("Enter") || key.eq_ignore_ascii_case("Return");
        let is_esc = key.eq_ignore_ascii_case("Escape") || key.eq_ignore_ascii_case("Esc");
        if is_enter || is_esc {
            let cmd_id = if is_enter { Some(commit_title_edit(app, state)?) } else { None };
            if is_esc {
                state.editing_title = None;
            }
            app.interaction.text_field_buffers.remove("grid.title_editor.input");
            if app.focus_widget_id.as_deref() == Some("grid.title_editor.input") {
                app.focus_widget_id = None;
            }
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from(if is_enter { "grid_core.title_commit" } else { "grid_core.title_cancel" }),
                command_id: cmd_id.flatten(),
                command_result: None,
            });
        }
    }

    let jump = mods.meta || mods.ctrl;
    let action = match key {
        "ArrowUp" | "Up" => Some(GridNavEvent::Arrow { dir: MoveDirection::Up, extend: mods.shift, jump }),
        "ArrowDown" | "Down" => Some(GridNavEvent::Arrow { dir: MoveDirection::Down, extend: mods.shift, jump }),
        "ArrowLeft" | "Left" => Some(GridNavEvent::Arrow { dir: MoveDirection::Left, extend: mods.shift, jump }),
        "ArrowRight" | "Right" => Some(GridNavEvent::Arrow { dir: MoveDirection::Right, extend: mods.shift, jump }),
        "Tab" => Some(GridNavEvent::Tab { backward: mods.shift }),
        "Enter" | "Return" => Some(GridNavEvent::Enter { backward: mods.shift }),
        "Delete" | "Backspace" => Some(GridNavEvent::ClearSelection),
        _ => None,
    };

    let Some(nav) = action else {
        return Ok(UiEventOutcome::default());
    };
    apply_grid_nav_event(app, state, nav)
}

/// Block direct typing when the active grid cell sits inside a [`LinkedDataRegion`]
/// and surface the interactive `Go To Source` / `Unlink` prompt instead.
pub fn handle_grid_text_input(app: &UiApp, state: &mut GridCoreViewState, text: &str) -> UiEventOutcome {
    if text.is_empty() || state.editing_title.is_some() {
        return UiEventOutcome::default();
    }
    let Some(ds) = active_data_sheet(app) else {
        return UiEventOutcome::default();
    };
    if ds.frozen {
        return UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..UiEventOutcome::default() };
    }
    let Some(active) = app.grid.tool.active else {
        return UiEventOutcome::default();
    };
    if check_edit_blocked_by_link(ds, state, active.row, active.col, active.subcol) {
        return UiEventOutcome { handled: true, action: String::from("grid_core.link_read_only_prompt"), ..UiEventOutcome::default() };
    }
    UiEventOutcome::default()
}

/// Block inline cell double-click editing when `coord` sits inside a [`LinkedDataRegion`] or a frozen sheet.
pub fn handle_grid_double_click(app: &UiApp, state: &mut GridCoreViewState, coord: GridCellCoord) -> UiEventOutcome {
    let Some(ds) = active_data_sheet(app) else {
        return UiEventOutcome::default();
    };
    if ds.frozen {
        return UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..UiEventOutcome::default() };
    }
    if check_edit_blocked_by_link(ds, state, coord.row, coord.col, coord.subcol) {
        return UiEventOutcome { handled: true, action: String::from("grid_core.link_read_only_prompt"), ..UiEventOutcome::default() };
    }
    UiEventOutcome::default()
}

/// Update hovered cell coordinate and `GridViewState::hover_exclusion_tooltip` provenance text.
pub fn handle_grid_pointer_hover(app: &mut UiApp, state: &mut GridCoreViewState, coord: Option<GridCellCoord>) {
    state.hovered_cell = coord;
    let doc = app.session.document();
    let Some(ds) = doc.active_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)) else {
        app.grid.hover_exclusion_tooltip = None;
        return;
    };
    app.grid.update_hover_exclusion_tooltip(ds, coord);
}

#[derive(Debug, Clone, Copy)]
enum GridNavEvent {
    Arrow { dir: MoveDirection, extend: bool, jump: bool },
    Tab { backward: bool },
    Enter { backward: bool },
    ClearSelection,
}

fn apply_grid_nav_event(app: &mut UiApp, state: &mut GridCoreViewState, event: GridNavEvent) -> Result<UiEventOutcome, String> {
    let mut outcome = UiEventOutcome { handled: true, action: String::from("grid_core.nav"), ..UiEventOutcome::default() };

    if matches!(event, GridNavEvent::ClearSelection) {
        let prepared = {
            let doc = app.session.document();
            let Some(ds) = doc.active_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)) else {
                return Ok(UiEventOutcome::default());
            };
            prepare_clear_selected_cells(ds, &app.grid, state)
        };
        match prepared {
            ClearSelectionPlan::Frozen => {
                outcome.action = String::from("grid_core.frozen_read_only");
                state.last_nav_summary = Some(String::from("edit:clear_blocked_frozen"));
            }
            ClearSelectionPlan::LinkBlocked | ClearSelectionPlan::NothingToClear => {
                outcome.action = String::from("grid_core.link_read_only_prompt");
                state.last_nav_summary = Some(String::from("edit:clear"));
            }
            ClearSelectionPlan::Execute { sheet_id, cells_json } => {
                let out = app.execute_command("table.clearCells", json!({ "sheet_id": sheet_id, "cells": cells_json })).map_err(|e| e.to_string())?;
                outcome.action = String::from("grid_core.clear_selection");
                outcome.command_id = Some(out.command_id);
                outcome.command_result = Some(out.payload);
                state.last_nav_summary = Some(String::from("edit:clear"));
            }
        }
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(outcome);
    }

    // Borrow `app.session` and `&mut app.grid` as disjoint fields so 1M-cell navigation never clones `DataSheet` or `ProjectDocument`.
    {
        let doc = app.session.document();
        let Some(ds) = doc.active_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)) else {
            return Ok(UiEventOutcome::default());
        };
        match event {
            GridNavEvent::Arrow { dir, extend, jump } => {
                app.grid.navigate(ds, dir, extend, jump);
                let cur = app.grid.tool.active.unwrap_or_default();
                if extend {
                    sync_extended_selection(&mut app.grid, ds);
                } else {
                    app.grid.select_single_sync(cur);
                }
                let summary = format!("arrow:{dir:?}:extend={extend}:jump={jump}->({},{},{})", cur.row, cur.col, cur.subcol);
                outcome.action = format!("grid_core.{summary}");
                state.last_nav_summary = Some(summary);
            }
            GridNavEvent::Tab { backward } => {
                step_tab_cell(&mut app.grid, ds, backward);
                let cur = app.grid.tool.active.unwrap_or_default();
                app.grid.select_single_sync(cur);
                let summary = format!("tab:backward={backward}->({},{},{})", cur.row, cur.col, cur.subcol);
                outcome.action = format!("grid_core.{summary}");
                state.last_nav_summary = Some(summary);
            }
            GridNavEvent::Enter { backward } => {
                let dir = if backward { MoveDirection::Up } else { MoveDirection::Down };
                app.grid.navigate(ds, dir, false, false);
                let cur = app.grid.tool.active.unwrap_or_default();
                app.grid.select_single_sync(cur);
                let summary = format!("enter:backward={backward}->({},{},{})", cur.row, cur.col, cur.subcol);
                outcome.action = format!("grid_core.{summary}");
                state.last_nav_summary = Some(summary);
            }
            GridNavEvent::ClearSelection => {}
        }
        app.grid.refresh_selection_stats(doc);
    }

    Ok(outcome)
}

enum ClearSelectionPlan {
    Frozen,
    LinkBlocked,
    NothingToClear,
    Execute { sheet_id: String, cells_json: Vec<serde_json::Value> },
}

fn prepare_clear_selected_cells(ds: &DataSheet, grid: &crate::grid_view::GridViewState, state: &mut GridCoreViewState) -> ClearSelectionPlan {
    if ds.frozen {
        return ClearSelectionPlan::Frozen;
    }
    let mut coords = grid.all_selected_coords(ds);
    if coords.is_empty()
        && let Some(active) = grid.tool.active
    {
        coords.push(active);
    }
    if coords.is_empty() {
        return ClearSelectionPlan::NothingToClear;
    }
    for coord in &coords {
        if check_edit_blocked_by_link(ds, state, coord.row, coord.col, coord.subcol) {
            return ClearSelectionPlan::LinkBlocked;
        }
    }
    let cells_json: Vec<serde_json::Value> = coords
        .iter()
        .map(|c| {
            json!({
                "row": c.row,
                "col": c.col,
                "subcol": c.subcol,
            })
        })
        .collect();
    ClearSelectionPlan::Execute { sheet_id: ds.id.to_string(), cells_json }
}

fn sync_extended_selection(grid: &mut crate::grid_view::GridViewState, ds: &DataSheet) {
    let Some((min_c, max_c)) = grid.tool.selection_bounds() else {
        return;
    };
    let total_flat = total_flat_data_subcolumns(&ds.archetype);
    let max_row = max_c.row.min(ds.archetype.row_count().saturating_sub(1));
    let mut coords = Vec::new();
    let mut rows = Vec::new();
    for row in min_c.row..=max_row {
        rows.push(row);
        for flat in 0..total_flat {
            if let Some((col, subcol)) = resolve_flat_coord(&ds.archetype, flat)
                && (col, subcol) >= (min_c.col, min_c.subcol)
                && (col, subcol) <= (max_c.col, max_c.subcol)
            {
                coords.push(GridCellCoord::new(row, col, subcol));
            }
        }
    }
    grid.selected_coords = coords;
    grid.selected_rows = rows;
}

fn step_tab_cell(grid: &mut crate::grid_view::GridViewState, ds: &DataSheet, backward: bool) {
    let before = grid.tool.active.unwrap_or_default();
    let dir = if backward { MoveDirection::Left } else { MoveDirection::Right };
    grid.navigate(ds, dir, false, false);
    if grid.tool.active.unwrap_or_default() == before {
        let total_rows = ds.archetype.row_count();
        let total_cols = ds.archetype.column_count();
        if total_rows == 0 || total_cols == 0 {
            return;
        }
        if !backward && before.row + 1 < total_rows {
            grid.select_single_sync(GridCellCoord::new(before.row + 1, 0, 0));
        } else if backward && before.row > 0 {
            let last_col = total_cols.saturating_sub(1);
            let last_sub = data_set_subcolumn_count(&ds.archetype, last_col).saturating_sub(1);
            grid.select_single_sync(GridCellCoord::new(before.row - 1, last_col, last_sub));
        }
    }
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `grid_core` domain.
///
/// # Errors
/// Returns an error string when an underlying engine command fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let clean_payload = payload.strip_prefix("secondary.").unwrap_or(payload);
    if clean_payload == "smoke" {
        return Ok(UiEventOutcome::default());
    }

    if let Some(outcome) = handle_row_titles_action(app, state, clean_payload) {
        return Ok(outcome);
    }
    if let Some(outcome) = handle_column_resize_action(state, clean_payload) {
        return Ok(outcome);
    }
    if let Some(outcome) = handle_title_editor_action(app, state, clean_payload)? {
        return Ok(outcome);
    }
    if let Some(outcome) = handle_link_action(app, state, clean_payload)? {
        return Ok(outcome);
    }
    if let Some(outcome) = handle_nav_custom_action(app, state, clean_payload)? {
        return Ok(outcome);
    }
    if let Some(outcome) = handle_cell_action(app, state, clean_payload) {
        return Ok(outcome);
    }

    Ok(UiEventOutcome::default())
}

fn handle_row_titles_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Option<UiEventOutcome> {
    match payload {
        "row_titles:toggle" => {
            state.row_titles_collapsed = !state.row_titles_collapsed;
            app.grid.row_titles_collapsed_override = Some(state.row_titles_collapsed);
            Some(UiEventOutcome { handled: true, action: String::from("grid_core.row_titles:toggle"), ..UiEventOutcome::default() })
        }
        "row_titles:collapse" => {
            state.row_titles_collapsed = true;
            app.grid.row_titles_collapsed_override = Some(true);
            Some(UiEventOutcome { handled: true, action: String::from("grid_core.row_titles:collapse"), ..UiEventOutcome::default() })
        }
        "row_titles:expand" => {
            state.row_titles_collapsed = false;
            app.grid.row_titles_collapsed_override = Some(false);
            Some(UiEventOutcome { handled: true, action: String::from("grid_core.row_titles:expand"), ..UiEventOutcome::default() })
        }
        "row_titles:resize_step" => {
            let cur =
                if state.row_titles_collapsed || state.row_titles_width_px == 0 { DEFAULT_ROW_TITLES_WIDTH_PX } else { state.row_titles_width_px };
            let next = match cur {
                w if w < 144 => 144,
                w if w < 176 => 176,
                w if w < 208 => 208,
                _ => 96,
            };
            state.set_row_titles_width(next);
            app.grid.row_titles_collapsed_override = Some(false);
            Some(UiEventOutcome { handled: true, action: String::from("grid_core.row_titles:resize_step"), ..UiEventOutcome::default() })
        }
        _ => {
            if let Some(raw) = payload.strip_prefix("row_titles:width:")
                && let Ok(px) = raw.parse::<u32>()
            {
                state.set_row_titles_width(px);
                app.grid.row_titles_collapsed_override = Some(false);
                return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
            }
            if let Some(raw) = payload.strip_prefix("row_titles:resize_delta:")
                && let Ok(dx) = raw.parse::<i32>()
            {
                let base = i32::try_from(state.effective_row_titles_width()).unwrap_or(112);
                let next = (base + dx).max(i32::try_from(MIN_ROW_TITLES_WIDTH_PX).unwrap_or(56));
                state.set_row_titles_width(u32::try_from(next).unwrap_or(MIN_ROW_TITLES_WIDTH_PX));
                app.grid.row_titles_collapsed_override = Some(false);
                return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
            }
            None
        }
    }
}

fn handle_column_resize_action(state: &mut GridCoreViewState, payload: &str) -> Option<UiEventOutcome> {
    if let Some(rest) = payload.strip_prefix("col:width:") {
        let mut parts = rest.split(':');
        if let (Some(c), Some(s), Some(p)) = (parts.next(), parts.next(), parts.next())
            && let (Ok(col), Ok(subcol), Ok(px)) = (c.parse::<usize>(), s.parse::<usize>(), p.parse::<u32>())
        {
            state.set_data_col_width(col, subcol, px);
            return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
        }
    }
    if let Some(rest) = payload.strip_prefix("col:resize_delta:") {
        let mut parts = rest.split(':');
        if let (Some(c), Some(s), Some(d)) = (parts.next(), parts.next(), parts.next())
            && let (Ok(col), Ok(subcol), Ok(dx)) = (c.parse::<usize>(), s.parse::<usize>(), d.parse::<i32>())
        {
            let cur = i32::try_from(state.effective_data_col_width(col, subcol)).unwrap_or(88);
            let next = (cur + dx).max(i32::try_from(MIN_COL_WIDTH_PX).unwrap_or(44));
            state.set_data_col_width(col, subcol, u32::try_from(next).unwrap_or(MIN_COL_WIDTH_PX));
            return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
        }
    }
    if let Some(rest) = payload.strip_prefix("col:resize_step:") {
        let mut parts = rest.split(':');
        if let (Some(c), Some(s)) = (parts.next(), parts.next())
            && let (Ok(col), Ok(subcol)) = (c.parse::<usize>(), s.parse::<usize>())
        {
            let cur = state.effective_data_col_width(col, subcol);
            let next = match cur {
                w if w < 120 => 120,
                w if w < 152 => 152,
                w if w < 184 => 184,
                _ => 72,
            };
            state.set_data_col_width(col, subcol, next);
            return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
        }
    }
    if let Some(rest) = payload.strip_prefix("xcol:width:") {
        let mut parts = rest.split(':');
        if let (Some(s), Some(p)) = (parts.next(), parts.next())
            && let (Ok(subcol), Ok(px)) = (s.parse::<usize>(), p.parse::<u32>())
        {
            state.set_x_col_width(subcol, px);
            return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
        }
    }
    if let Some(rest) = payload.strip_prefix("xcol:resize_delta:") {
        let mut parts = rest.split(':');
        if let (Some(s), Some(d)) = (parts.next(), parts.next())
            && let (Ok(subcol), Ok(dx)) = (s.parse::<usize>(), d.parse::<i32>())
        {
            let cur = i32::try_from(state.effective_x_col_width(subcol)).unwrap_or(96);
            let next = (cur + dx).max(i32::try_from(MIN_COL_WIDTH_PX).unwrap_or(44));
            state.set_x_col_width(subcol, u32::try_from(next).unwrap_or(MIN_COL_WIDTH_PX));
            return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
        }
    }
    if let Some(raw) = payload.strip_prefix("xcol:resize_step:")
        && let Ok(subcol) = raw.parse::<usize>()
    {
        let cur = state.effective_x_col_width(subcol);
        let next = match cur {
            w if w < 128 => 128,
            w if w < 160 => 160,
            _ => 80,
        };
        state.set_x_col_width(subcol, next);
        return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
    }
    None
}

fn handle_title_editor_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Result<Option<UiEventOutcome>, String> {
    if payload == "title:cancel" {
        state.editing_title = None;
        app.interaction.text_field_buffers.remove("grid.title_editor.input");
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("grid_core.title:cancel"), ..UiEventOutcome::default() }));
    }
    if payload == "title:commit" {
        let cmd_id = commit_title_edit(app, state)?;
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("grid_core.title:commit"), command_id: cmd_id, command_result: None }));
    }
    if payload == "title:begin:x" {
        let Some((sid, draft)) = active_data_sheet(app)
            .map(|ds| (ds.id.to_string(), archetype_x_title(&ds.archetype).map(ToOwned::to_owned).unwrap_or_else(|| String::from("X"))))
        else {
            return Ok(Some(UiEventOutcome::default()));
        };
        open_title_editor(app, state, &sid, GridTitleTarget::XColumn, draft);
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("grid_core.title:begin:x"), ..UiEventOutcome::default() }));
    }
    if let Some(raw) = payload.strip_prefix("title:begin:col:")
        && let Ok(col) = raw.parse::<usize>()
    {
        let Some((sid, draft)) = active_data_sheet(app).map(|ds| {
            let d =
                crate::dialogs_canvas::data_column_title(ds, col).map(ToOwned::to_owned).unwrap_or_else(|| curvebook_doc::default_column_label(col));
            (ds.id.to_string(), d)
        }) else {
            return Ok(Some(UiEventOutcome::default()));
        };
        open_title_editor(app, state, &sid, GridTitleTarget::DataColumn { col }, draft);
        return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
    }
    if let Some(rest) = payload.strip_prefix("title:begin:subcol:") {
        let mut parts = rest.split(':');
        if let (Some(c), Some(s)) = (parts.next(), parts.next())
            && let (Ok(col), Ok(subcol)) = (c.parse::<usize>(), s.parse::<usize>())
        {
            let Some((sid, draft)) = active_data_sheet(app).map(|ds| {
                let d = nested_subcolumn_title(ds, col, subcol).unwrap_or_else(|| subcolumn_header_label(ds, col, subcol, &app.locale_preset));
                (ds.id.to_string(), d)
            }) else {
                return Ok(Some(UiEventOutcome::default()));
            };
            open_title_editor(app, state, &sid, GridTitleTarget::Subcolumn { col, subcol }, draft);
            return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
        }
    }
    if let Some(raw) = payload.strip_prefix("title:begin:row:")
        && let Ok(row) = raw.parse::<usize>()
    {
        let Some((sid, draft)) =
            active_data_sheet(app).map(|ds| (ds.id.to_string(), archetype_row_title(&ds.archetype, row).unwrap_or_default().to_owned()))
        else {
            return Ok(Some(UiEventOutcome::default()));
        };
        open_title_editor(app, state, &sid, GridTitleTarget::RowTitle { row }, draft);
        return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
    }
    Ok(None)
}

fn open_title_editor(app: &mut UiApp, state: &mut GridCoreViewState, sheet_id: &str, target: GridTitleTarget, draft: String) {
    app.interaction.text_field_buffers.insert(String::from("grid.title_editor.input"), TextFieldEditState::new(draft.clone(), false));
    app.focus_widget_id = Some(String::from("grid.title_editor.input"));
    state.editing_title = Some(GridTitleEditState { sheet_id: sheet_id.to_owned(), target, draft });
}

fn commit_title_edit(app: &mut UiApp, state: &mut GridCoreViewState) -> Result<Option<String>, String> {
    let Some(edit) = state.editing_title.take() else {
        return Ok(None);
    };
    let text = app.interaction.text_field_buffers.remove("grid.title_editor.input").map_or(edit.draft, |b| b.text);
    let out = match edit.target {
        GridTitleTarget::XColumn => app.execute_command(
            "table.setTitles",
            json!({
                "sheet_id": edit.sheet_id,
                "x_title": text,
            }),
        ),
        GridTitleTarget::DataColumn { col } => app.execute_command(
            "table.setTitles",
            json!({
                "sheet_id": edit.sheet_id,
                "col": col,
                "title": text,
            }),
        ),
        GridTitleTarget::Subcolumn { col, subcol } => {
            let is_nested =
                app.document().data_sheet(&SheetId::from(edit.sheet_id.as_str())).is_some_and(|ds| matches!(ds.archetype, TableArchetype::Nested(_)));
            if is_nested {
                app.execute_command(
                    "table.setSubcolumnTitles",
                    json!({
                        "sheet_id": edit.sheet_id,
                        "col": col,
                        "subcol": subcol,
                        "title": text,
                    }),
                )
            } else {
                app.execute_command(
                    "table.setTitles",
                    json!({
                        "sheet_id": edit.sheet_id,
                        "col": col,
                        "title": text,
                    }),
                )
            }
        }
        GridTitleTarget::RowTitle { row } => app.execute_command(
            "table.setRowTitles",
            json!({
                "sheet_id": edit.sheet_id,
                "row": row,
                "title": text,
            }),
        ),
    }
    .map_err(|e| e.to_string())?;
    Ok(Some(out.command_id))
}

fn handle_link_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Result<Option<UiEventOutcome>, String> {
    if payload == "link:close" || payload == "link:dismiss_prompt" {
        state.active_link_popover = None;
        state.link_edit_warning = None;
        return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
    }
    if let Some(region_id) = payload.strip_prefix("link:open:") {
        let Some(ds) = active_data_sheet(app) else {
            return Ok(Some(UiEventOutcome::default()));
        };
        if ds.linked_regions.iter().any(|r| r.id == region_id) {
            state.active_link_popover = Some(GridLinkPopoverState { sheet_id: ds.id.to_string(), region_id: region_id.to_owned() });
            return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
        }
    }
    if let Some(region_id) = payload.strip_prefix("link:goto:") {
        let Some(ds) = active_data_sheet(app) else {
            return Ok(Some(UiEventOutcome::default()));
        };
        let upstream = ds.linked_regions.iter().find(|r| r.id == region_id).and_then(|r| r.spec.source.upstream_sheet_id().cloned());
        state.active_link_popover = None;
        state.link_edit_warning = None;
        if let Some(target_sid) = upstream {
            let _ = app.execute_command(
                "sheet.select",
                json!({
                    "sheet_id": target_sid.to_string(),
                    "mode": "navigator",
                }),
            );
            return Ok(Some(UiEventOutcome {
                handled: true,
                action: format!("grid_core.link:goto:{target_sid}"),
                command_id: Some(String::from("sheet.select")),
                command_result: None,
            }));
        }
        return Ok(Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() }));
    }
    if let Some(region_id) = payload.strip_prefix("link:unlink:") {
        let Some(ds) = active_data_sheet(app) else {
            return Ok(Some(UiEventOutcome::default()));
        };
        let sid = ds.id.to_string();
        let _ = region_id;
        let out = app
            .execute_command(
                "table.clearCells",
                json!({
                    "sheet_id": sid,
                    "cells": [],
                    "unlink": true,
                }),
            )
            .map_err(|e| e.to_string())?;
        state.active_link_popover = None;
        state.link_edit_warning = None;
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: format!("grid_core.{payload}"),
            command_id: Some(out.command_id),
            command_result: Some(out.payload),
        }));
    }
    Ok(None)
}

fn handle_nav_custom_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Result<Option<UiEventOutcome>, String> {
    let nav = match payload {
        "nav:up" => Some(GridNavEvent::Arrow { dir: MoveDirection::Up, extend: false, jump: false }),
        "nav:down" => Some(GridNavEvent::Arrow { dir: MoveDirection::Down, extend: false, jump: false }),
        "nav:left" => Some(GridNavEvent::Arrow { dir: MoveDirection::Left, extend: false, jump: false }),
        "nav:right" => Some(GridNavEvent::Arrow { dir: MoveDirection::Right, extend: false, jump: false }),
        "nav:shift_up" => Some(GridNavEvent::Arrow { dir: MoveDirection::Up, extend: true, jump: false }),
        "nav:shift_down" => Some(GridNavEvent::Arrow { dir: MoveDirection::Down, extend: true, jump: false }),
        "nav:shift_left" => Some(GridNavEvent::Arrow { dir: MoveDirection::Left, extend: true, jump: false }),
        "nav:shift_right" => Some(GridNavEvent::Arrow { dir: MoveDirection::Right, extend: true, jump: false }),
        "nav:jump_up" => Some(GridNavEvent::Arrow { dir: MoveDirection::Up, extend: false, jump: true }),
        "nav:jump_down" => Some(GridNavEvent::Arrow { dir: MoveDirection::Down, extend: false, jump: true }),
        "nav:jump_left" => Some(GridNavEvent::Arrow { dir: MoveDirection::Left, extend: false, jump: true }),
        "nav:jump_right" => Some(GridNavEvent::Arrow { dir: MoveDirection::Right, extend: false, jump: true }),
        "nav:shift_jump_up" => Some(GridNavEvent::Arrow { dir: MoveDirection::Up, extend: true, jump: true }),
        "nav:shift_jump_down" => Some(GridNavEvent::Arrow { dir: MoveDirection::Down, extend: true, jump: true }),
        "nav:shift_jump_left" => Some(GridNavEvent::Arrow { dir: MoveDirection::Left, extend: true, jump: true }),
        "nav:shift_jump_right" => Some(GridNavEvent::Arrow { dir: MoveDirection::Right, extend: true, jump: true }),
        "nav:tab" => Some(GridNavEvent::Tab { backward: false }),
        "nav:shift_tab" => Some(GridNavEvent::Tab { backward: true }),
        "nav:enter" => Some(GridNavEvent::Enter { backward: false }),
        "nav:shift_enter" => Some(GridNavEvent::Enter { backward: true }),
        "edit:clear" | "edit:clear_selection" => Some(GridNavEvent::ClearSelection),
        _ => None,
    };
    let Some(ev) = nav else {
        return Ok(None);
    };
    apply_grid_nav_event(app, state, ev).map(Some)
}

fn handle_cell_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Option<UiEventOutcome> {
    if let Some(rest) = payload.strip_prefix("cell:hover:")
        && let Some(coord) = parse_triple_coord(rest)
    {
        handle_grid_pointer_hover(app, state, Some(coord));
        return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
    }
    if let Some(rest) = payload.strip_prefix("cell:focus:")
        && let Some(coord) = parse_triple_coord(rest)
    {
        app.grid.select_single_sync(coord);
        return Some(UiEventOutcome { handled: true, action: format!("grid_core.{payload}"), ..UiEventOutcome::default() });
    }
    None
}

fn parse_triple_coord(raw: &str) -> Option<GridCellCoord> {
    let mut parts = raw.split(':');
    let row = parts.next()?.parse::<usize>().ok()?;
    let col = parts.next()?.parse::<usize>().ok()?;
    let subcol = parts.next()?.parse::<usize>().ok()?;
    Some(GridCellCoord::new(row, col, subcol))
}

#[derive(Debug, Clone, Copy)]
struct ColSpanSlice {
    col: usize,
    x_px: i32,
    width_px: u32,
}

#[derive(Debug, Clone, Copy)]
struct VisibleSubcol {
    flat_col: usize,
    col: usize,
    subcol: usize,
    x_px: i32,
    width_px: u32,
}

struct GridLayoutMetrics {
    grid_rect: LayoutRectPx,
    gutter_x: i32,
    row_num_w: u32,
    rt_x: i32,
    rt_w: u32,
    cols_x: i32,
    cols_w: u32,
    header_y: i32,
    data_y: i32,
    body_h: u32,
    row_h: u32,
    tier1_h: u32,
    tier2_h: u32,
    tier3_h: u32,
    header_h: u32,
    resize_w: u32,
    scale: f64,
    x_subcol_count: usize,
    total_rows: usize,
    total_flat_cols: usize,
    scroll_x: f64,
    scroll_y: f64,
    is_rtl: bool,
    scroll_id: WidgetId,
}

fn rect_visible_in_clip(rect: LayoutRectPx, clip: LayoutRectPx) -> bool {
    let inter = rect.intersect(clip);
    inter.width >= 2 && inter.height >= 2
}

fn scoped_id(slot_tag: &str, id: &str) -> String {
    if slot_tag == "secondary" { format!("secondary.{id}") } else { id.to_owned() }
}

fn scoped_payload(slot_tag: &str, payload: &str) -> String {
    if slot_tag == "secondary" { format!("secondary.{payload}") } else { payload.to_owned() }
}

/// Build the virtualized data table grid view for `ds` inside `rect`.
pub fn build_data_grid_view(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, rect: LayoutRectPx, slot_tag: &str) {
    if rect.width < 32 || rect.height < 32 {
        return;
    }

    let scale = builder.scale().clamp(0.5, 4.0);
    let state = &builder.app.view_shards.grid_core;
    let locale = builder.app.locale.as_str();
    let preset = &builder.app.locale_preset;
    let is_rtl = builder.tree.is_rtl;
    let mut cursor_y = rect.y;

    let mut formula_dock_active = false;

    // Preserve `fx` formula bar seam without duplicate IDs in dual split mode.
    let ui_scale = builder.scale();
    if slot_tag != "secondary" || builder.tree.node("formula_bar.input").is_none() {
        let show_formula = matches!(ds.archetype, TableArchetype::MultipleVariables(_)) || builder.app.grid.formula_bar.active;
        let consumed = mv_formula::build_formula_bar_row(builder, ds, rect, show_formula);
        cursor_y = cursor_y.saturating_add(i32::try_from(consumed).unwrap_or(0));
        formula_dock_active =
            consumed > 0 && rect.width >= scale_u32(240, ui_scale) && rect.height.saturating_sub(consumed + 4) >= scale_u32(120, ui_scale);
    }

    let dock_reserve = if formula_dock_active {
        let base = mv_formula::right_panel_desired_width_for_scale(builder.app, ds, rect, ui_scale);
        if base > 0 {
            let mv_state = &builder.app.view_shards.mv_formula;
            let min_panel = scale_u32(224, ui_scale);
            let panel_w = base.max(min_panel).min(rect.width.saturating_sub(12));
            let drawer_min = scale_u32(212, ui_scale);
            let drawer_max = scale_u32(276, ui_scale);
            let extra_drawer = if mv_state.right_panel_open && mv_state.variables_drawer_open && rect.width >= panel_w + min_panel {
                rect.width.saturating_sub(panel_w + 14).clamp(drawer_min, drawer_max) + 8
            } else {
                0
            };
            (panel_w + 8 + extra_drawer).min(rect.width.saturating_sub(scale_u32(120, ui_scale)))
        } else {
            0
        }
    } else {
        0
    };
    let grid_w = rect.width.saturating_sub(dock_reserve);
    let grid_x = if is_rtl { rect.x.saturating_add(i32::try_from(dock_reserve).unwrap_or(0)) } else { rect.x };

    // Linked-region provenance strip when the sheet carries active `LinkedDataRegion`s.
    let show_warning = state.link_edit_warning.as_ref().is_some_and(|w| w.sheet_id == ds.id.to_string());
    if !ds.linked_regions.is_empty() || show_warning {
        let rem_h = u32::try_from((rect.bottom() - cursor_y).max(0)).unwrap_or(0);
        let strip_h = scale_u32(LINK_STRIP_H_PX, scale).min(rem_h);
        if strip_h >= 18 {
            let strip_rect = LayoutRectPx::new(grid_x, cursor_y, grid_w, strip_h);
            emit_link_provenance_strip(builder, ds, state, strip_rect, slot_tag, locale, scale);
            cursor_y = cursor_y.saturating_add(i32::try_from(strip_h).unwrap_or(0));
        }
    }

    let tier1_h = scale_u32(TIER1_HEADER_H_PX, scale);
    let tier2_h = scale_u32(TIER2_HEADER_H_PX, scale);
    let tier3_h = scale_u32(TIER3_HEADER_H_PX, scale);
    let header_h = tier1_h + tier2_h + tier3_h;
    let row_h = scale_u32(DEFAULT_ROW_HEIGHT_PX, scale);
    let resize_w = scale_u32(RESIZE_HANDLE_W_PX, scale);

    let avail_h = u32::try_from((rect.bottom() - cursor_y).max(0)).unwrap_or(0);
    if avail_h < header_h + 8 || grid_w < 32 {
        return;
    }
    let grid_rect = LayoutRectPx::new(grid_x, cursor_y, grid_w, avail_h);

    let x_subcol_count = x_subcolumn_count(&ds.archetype);
    let total_rows = ds.archetype.row_count();
    let total_flat_cols = total_flat_data_subcolumns(&ds.archetype);

    let row_titles_collapsed = builder.app.grid.row_titles_collapsed_override.unwrap_or(state.row_titles_collapsed);
    let mut effective_state = state.clone();
    effective_state.row_titles_collapsed = row_titles_collapsed;

    let row_num_w = scale_u32(ROW_NUM_WIDTH_PX, scale).min(grid_rect.width / 3);
    let rt_w = scale_u32(effective_state.effective_row_titles_width(), scale).min(grid_rect.width.saturating_sub(row_num_w + scale_u32(32, scale)));
    let left_gutter_w = row_num_w.saturating_add(rt_w);

    let x_total_w = total_x_width(&effective_state, x_subcol_count, scale);
    let data_total_w = total_data_width(&ds.archetype, &effective_state, total_flat_cols, scale);
    let total_cols_w = x_total_w.saturating_add(data_total_w);
    let total_data_h = u32::try_from(total_rows).unwrap_or(u32::MAX / row_h.max(1)).saturating_mul(row_h);

    let sb = scrollbar_thickness_px(builder.tree.metrics.scale_factor).max(10);
    let (vscroll_reserve, hscroll_reserve) = compute_scrollbar_reserves(grid_rect, left_gutter_w, header_h, total_cols_w, total_data_h, sb);

    let content_w = left_gutter_w.saturating_add(total_cols_w).saturating_add(vscroll_reserve);
    let content_h = header_h.saturating_add(total_data_h).saturating_add(hscroll_reserve);

    let sheet_id_str = scoped_id(slot_tag, &format!("grid.sheet.{}", ds.id));
    let scroll_meta = ScrollMeta::new(
        sheet_id_str.clone(),
        grid_rect,
        [content_w.max(grid_rect.width), content_h.max(grid_rect.height)],
        builder.scroll_offset(&sheet_id_str),
        ScrollAxes::Both,
    );

    let inner_w = grid_rect.width.saturating_sub(vscroll_reserve);
    let inner_h = grid_rect.height.saturating_sub(hscroll_reserve);
    let cols_w = inner_w.saturating_sub(left_gutter_w);
    let body_h = inner_h.saturating_sub(header_h);
    if cols_w < 16 || body_h < 8 {
        emit_grid_background_and_container(builder, grid_rect, &sheet_id_str, &scroll_meta);
        emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(WidgetId::from(sheet_id_str.as_str())));
        return;
    }

    let content_origin_x = grid_rect.x.saturating_add(i32::try_from(if is_rtl { vscroll_reserve } else { 0 }).unwrap_or(0));
    let gutter_local_x = mirror_x_for_rtl(0, left_gutter_w, inner_w, is_rtl);
    let gutter_x = content_origin_x.saturating_add(i32::try_from(gutter_local_x).unwrap_or(0));
    let row_num_local_x = mirror_x_for_rtl(0, row_num_w, left_gutter_w, is_rtl);
    let rt_local_x = mirror_x_for_rtl(row_num_w, rt_w, left_gutter_w, is_rtl);
    let cols_local_x = mirror_x_for_rtl(left_gutter_w, cols_w, inner_w, is_rtl);
    let cols_x = content_origin_x.saturating_add(i32::try_from(cols_local_x).unwrap_or(0));

    let metrics = GridLayoutMetrics {
        grid_rect,
        gutter_x,
        row_num_w,
        rt_x: gutter_x.saturating_add(i32::try_from(rt_local_x).unwrap_or(0)),
        rt_w,
        cols_x,
        cols_w,
        header_y: grid_rect.y,
        data_y: grid_rect.y.saturating_add(i32::try_from(header_h).unwrap_or(0)),
        body_h,
        row_h,
        tier1_h,
        tier2_h,
        tier3_h,
        header_h,
        resize_w,
        scale,
        x_subcol_count,
        total_rows,
        total_flat_cols,
        scroll_x: scroll_meta.scroll_offset_px[0].max(0.0),
        scroll_y: scroll_meta.scroll_offset_px[1].max(0.0),
        is_rtl,
        scroll_id: WidgetId::from(sheet_id_str.as_str()),
    };

    builder.push_clip(grid_rect);

    emit_grid_background_and_container(builder, grid_rect, &sheet_id_str, &scroll_meta);

    let (vis_x_cols, vis_data_cols, vis_spans) = compute_visible_columns(ds, &effective_state, &metrics);
    let (start_row, end_row) = compute_visible_row_range(&metrics);

    emit_visible_cells_window(builder, ds, &metrics, preset, locale, start_row, end_row, &vis_x_cols, &vis_data_cols, slot_tag);

    emit_gutter_body_window(
        builder,
        ds,
        &effective_state,
        &metrics,
        gutter_x.saturating_add(i32::try_from(row_num_local_x).unwrap_or(0)),
        start_row,
        end_row,
        slot_tag,
        locale,
    );

    emit_column_headers_3tier(builder, ds, &metrics, preset, locale, &vis_x_cols, &vis_data_cols, &vis_spans, slot_tag);

    emit_gutter_header(builder, &effective_state, &metrics, gutter_x.saturating_add(i32::try_from(row_num_local_x).unwrap_or(0)), slot_tag, locale);

    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(metrics.scroll_id.clone()));

    // Strictly preserve R2-S10-09: secondary split pane must not overwrite primary hit geometry.
    if slot_tag != "secondary" || builder.tree.grid_geometry.is_none() {
        let mut geom_cols = Vec::with_capacity(vis_data_cols.len());
        let mut col_spans = Vec::with_capacity(vis_data_cols.len());
        let start_col = vis_data_cols.first().map_or(0, |v| v.flat_col);
        for d in &vis_data_cols {
            geom_cols.push((d.flat_col, GridCellCoord::new(0, d.col, d.subcol)));
            col_spans.push((u32::try_from(d.x_px.max(0)).unwrap_or(0), d.width_px));
        }
        let x_col_spans = vis_x_cols.iter().map(|x| (u32::try_from(x.x_px.max(0)).unwrap_or(0), x.width_px)).collect();
        let sy_rem = if metrics.scroll_y >= 0.0 && row_h > 0 { (metrics.scroll_y as u64) % u64::from(row_h) } else { 0 };
        let body_y_i = i64::from(metrics.data_y).saturating_sub(i64::try_from(sy_rem).unwrap_or(0));
        let geom = GridHitGeometry {
            sheet_id: ds.id.clone(),
            sheet_rect: WorkspacePixelRect {
                x: u32::try_from(grid_rect.x.max(0)).unwrap_or(0),
                y: u32::try_from(grid_rect.y.max(0)).unwrap_or(0),
                width: grid_rect.width,
                height: grid_rect.height,
            },
            hdr_y: u32::try_from(metrics.header_y.max(0)).unwrap_or(0),
            body_y: u32::try_from(body_y_i.max(0)).unwrap_or(0),
            row_h,
            gutter_w: left_gutter_w,
            col_w: vis_data_cols.first().map_or_else(|| scale_u32(DEFAULT_DATA_COL_WIDTH_PX, scale), |v| v.width_px),
            x_subcols: x_subcol_count,
            x_col_w: vis_x_cols.first().map_or_else(|| scale_u32(DEFAULT_X_COL_WIDTH_PX, scale), |v| v.width_px),
            start_row,
            end_row,
            start_col,
            visible_cols: geom_cols,
            x_col_spans,
            col_spans,
            is_rtl,
        };
        builder.tree.grid_geometry = Some(geom);
    }

    builder.pop_clip();

    if slot_tag != "secondary" || builder.tree.node("grid.title_editor.input").is_none() {
        emit_inline_title_editor_overlay(builder, ds, &effective_state, &metrics, locale);
        emit_linked_region_popover(builder, ds, &effective_state, &metrics, locale);
    }
}

fn x_subcolumn_count(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(_) | TableArchetype::Survival(_) => 1,
        _ => 0,
    }
}

fn total_x_width(state: &GridCoreViewState, x_subcol_count: usize, scale: f64) -> u32 {
    if state.x_col_widths_px.is_empty() {
        return u32::try_from(x_subcol_count).unwrap_or(0).saturating_mul(scale_u32(DEFAULT_X_COL_WIDTH_PX, scale));
    }
    let mut sum = 0u32;
    for s in 0..x_subcol_count {
        sum = sum.saturating_add(scale_u32(state.effective_x_col_width(s), scale));
    }
    sum
}

fn total_data_width(arch: &TableArchetype, state: &GridCoreViewState, total_flat_cols: usize, scale: f64) -> u32 {
    let def_w = scale_u32(DEFAULT_DATA_COL_WIDTH_PX, scale);
    if state.column_widths_px.is_empty() || total_flat_cols > 512 {
        return u32::try_from(total_flat_cols).unwrap_or(u32::MAX / def_w.max(1)).saturating_mul(def_w);
    }
    let mut sum = 0u32;
    for flat in 0..total_flat_cols {
        if let Some((col, sub)) = resolve_flat_coord(arch, flat) {
            sum = sum.saturating_add(scale_u32(state.effective_data_col_width(col, sub), scale));
        } else {
            sum = sum.saturating_add(def_w);
        }
    }
    sum
}

fn compute_scrollbar_reserves(
    grid_rect: LayoutRectPx,
    left_gutter_w: u32,
    header_h: u32,
    total_cols_w: u32,
    total_data_h: u32,
    sb: u32,
) -> (u32, u32) {
    let pad = CORNER_PAD_PX;
    let avail_cols0 = grid_rect.width.saturating_sub(left_gutter_w);
    let avail_body0 = grid_rect.height.saturating_sub(header_h);

    let mut need_v = total_data_h > avail_body0;
    let mut need_h = total_cols_w > avail_cols0;
    for _ in 0..2 {
        let vr = if need_v { sb + pad } else { 0 };
        let hr = if need_h { sb + pad } else { 0 };
        need_v = total_data_h > avail_body0.saturating_sub(hr);
        need_h = total_cols_w > avail_cols0.saturating_sub(vr);
    }
    (if need_v { sb + pad } else { 0 }, if need_h { sb + pad } else { 0 })
}

fn emit_grid_background_and_container(builder: &mut LayoutBuilder<'_>, grid_rect: LayoutRectPx, sheet_id_str: &str, scroll_meta: &ScrollMeta) {
    let bg = builder.palette.surface;
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(sheet_id_str),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: grid_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Data Grid", FocusScope::Workspace),
        state: widget_state(builder, sheet_id_str, ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), grid_rect, bg);
    builder.attach_scroll(&wid, scroll_meta.clone());
}

fn compute_visible_row_range(metrics: &GridLayoutMetrics) -> (usize, usize) {
    if metrics.total_rows == 0 || metrics.body_h == 0 {
        return (0, 0);
    }
    let sy = if metrics.scroll_y >= 0.0 { metrics.scroll_y as usize } else { 0 };
    let row_h = usize::try_from(metrics.row_h.max(1)).unwrap_or(24);
    let start = (sy / row_h).min(metrics.total_rows);
    let max_vis = usize::try_from(metrics.body_h / metrics.row_h.max(1)).unwrap_or(0) + 2;
    let end = start.saturating_add(max_vis).min(metrics.total_rows);
    (start, end)
}

fn compute_visible_columns(
    ds: &DataSheet,
    state: &GridCoreViewState,
    metrics: &GridLayoutMetrics,
) -> (Vec<VisibleSubcol>, Vec<VisibleSubcol>, Vec<ColSpanSlice>) {
    let mut vis_x = Vec::new();
    let mut vis_data = Vec::new();
    let mut spans = Vec::new();

    let scroll_x = if metrics.scroll_x >= 0.0 { metrics.scroll_x as u32 } else { 0 };
    let vis_end_x = scroll_x.saturating_add(metrics.cols_w);

    let mut content_cursor = 0u32;
    for s in 0..metrics.x_subcol_count {
        let w = scale_u32(state.effective_x_col_width(s), metrics.scale);
        let col_end = content_cursor.saturating_add(w);
        if col_end > scroll_x && content_cursor < vis_end_x {
            let screen_x = screen_col_x(metrics, content_cursor, w, scroll_x);
            vis_x.push(VisibleSubcol { flat_col: s, col: 0, subcol: s, x_px: screen_x, width_px: w });
        }
        content_cursor = col_end;
    }

    let def_w = scale_u32(DEFAULT_DATA_COL_WIDTH_PX, metrics.scale);
    if state.has_uniform_column_widths() && metrics.total_flat_cols > 64 {
        let x_area_end = content_cursor;
        let data_scroll = scroll_x.saturating_sub(x_area_end);
        let first_flat = usize::try_from(data_scroll / def_w.max(1)).unwrap_or(0);
        let count = usize::try_from(metrics.cols_w / def_w.max(1)).unwrap_or(0) + 3;
        let end_flat = first_flat.saturating_add(count).min(metrics.total_flat_cols);
        for flat in first_flat..end_flat {
            if let Some((col, sub)) = resolve_flat_coord(&ds.archetype, flat) {
                let c_start = x_area_end.saturating_add(u32::try_from(flat).unwrap_or(0).saturating_mul(def_w));
                let screen_x = screen_col_x(metrics, c_start, def_w, scroll_x);
                vis_data.push(VisibleSubcol { flat_col: flat, col, subcol: sub, x_px: screen_x, width_px: def_w });
            }
        }
        accumulate_spans_from_visible(&vis_data, &mut spans);
        return (vis_x, vis_data, spans);
    }

    let mut flat = 0usize;
    let col_count = ds.archetype.column_count();
    for col in 0..col_count {
        let sub_count = data_set_subcolumn_count(&ds.archetype, col).max(1);
        let mut span_has_vis = false;
        let mut min_vis_x = i32::MAX;
        let mut max_vis_end = i32::MIN;

        for sub in 0..sub_count {
            let w = scale_u32(state.effective_data_col_width(col, sub), metrics.scale);
            let col_end = content_cursor.saturating_add(w);
            if col_end > scroll_x && content_cursor < vis_end_x {
                let screen_x = screen_col_x(metrics, content_cursor, w, scroll_x);
                vis_data.push(VisibleSubcol { flat_col: flat, col, subcol: sub, x_px: screen_x, width_px: w });
                span_has_vis = true;
                min_vis_x = min_vis_x.min(screen_x);
                max_vis_end = max_vis_end.max(screen_x.saturating_add(i32::try_from(w).unwrap_or(0)));
            }
            content_cursor = col_end;
            flat += 1;
        }
        if span_has_vis {
            spans.push(ColSpanSlice { col, x_px: min_vis_x, width_px: u32::try_from((max_vis_end - min_vis_x).max(0)).unwrap_or(0) });
        }
        if content_cursor >= vis_end_x {
            break;
        }
    }

    (vis_x, vis_data, spans)
}

fn accumulate_spans_from_visible(vis_data: &[VisibleSubcol], spans: &mut Vec<ColSpanSlice>) {
    let mut cur: Option<ColSpanSlice> = None;
    for cell in vis_data {
        match &mut cur {
            Some(span) if span.col == cell.col => {
                let left = span.x_px.min(cell.x_px);
                let right = span
                    .x_px
                    .saturating_add(i32::try_from(span.width_px).unwrap_or(0))
                    .max(cell.x_px.saturating_add(i32::try_from(cell.width_px).unwrap_or(0)));
                span.x_px = left;
                span.width_px = u32::try_from((right - left).max(0)).unwrap_or(0);
            }
            _ => {
                if let Some(prev) = cur.take() {
                    spans.push(prev);
                }
                cur = Some(ColSpanSlice { col: cell.col, x_px: cell.x_px, width_px: cell.width_px });
            }
        }
    }
    if let Some(last) = cur {
        spans.push(last);
    }
}

fn screen_col_x(metrics: &GridLayoutMetrics, content_col_x: u32, col_w: u32, scroll_x: u32) -> i32 {
    if !metrics.is_rtl {
        metrics.cols_x.saturating_add(i32::try_from(content_col_x).unwrap_or(0) - i32::try_from(scroll_x).unwrap_or(0))
    } else {
        let rel_left = i32::try_from(content_col_x).unwrap_or(0) - i32::try_from(scroll_x).unwrap_or(0);
        let right_edge = metrics.cols_x.saturating_add(i32::try_from(metrics.cols_w).unwrap_or(0)) - rel_left;
        right_edge - i32::try_from(col_w).unwrap_or(0)
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_visible_cells_window(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    metrics: &GridLayoutMetrics,
    preset: &LocalePreset,
    locale: &str,
    start_row: usize,
    end_row: usize,
    vis_x_cols: &[VisibleSubcol],
    vis_data_cols: &[VisibleSubcol],
    slot_tag: &str,
) {
    let body_clip = LayoutRectPx::new(metrics.cols_x, metrics.data_y, metrics.cols_w, metrics.body_h);
    builder.push_clip(body_clip);

    let sy = if metrics.scroll_y >= 0.0 { metrics.scroll_y as i64 } else { 0 };
    let row_h_i = i64::from(metrics.row_h);
    let body_bottom = i64::from(metrics.data_y) + i64::from(metrics.body_h);

    let active = builder.app.grid.tool.active;
    let sel_bounds = builder.app.grid.tool.selection_bounds();
    let has_explicit_coords = !builder.app.grid.selected_coords.is_empty();
    let vis_selected_coords: BTreeSet<(usize, usize, usize)> =
        builder.app.grid.selected_coords.iter().filter(|c| c.row >= start_row && c.row < end_row).map(|c| (c.row, c.col, c.subcol)).collect();

    for row in start_row..end_row {
        let row_y_i = i64::from(metrics.data_y) + (row as i64) * row_h_i - sy;
        if row_y_i >= body_bottom {
            break;
        }
        let row_y = i32::try_from(row_y_i).unwrap_or(0);
        let is_alt = row % 2 == 1;

        for xcol in vis_x_cols {
            let cell_rect = LayoutRectPx::new(xcol.x_px, row_y, xcol.width_px, metrics.row_h);
            if !rect_visible_in_clip(cell_rect, body_clip) {
                continue;
            }
            let is_editing = builder.app.inline_editor.active
                && matches!(
                    &builder.app.inline_editor.target,
                    Some(crate::interaction::InlineEditorTarget::GridXCell { sheet_id, row: er, subcol: es })
                        if *sheet_id == ds.id && *er == row && *es == xcol.subcol
                );
            let display = if is_editing { builder.app.inline_editor.buffer.clone() } else { x_cell_display(ds, row, xcol.subcol, preset) };
            let is_numeric = !is_editing
                && (ds.x_or_time_cell(row, xcol.subcol).is_some_and(|c| matches!(c.value, curvebook_doc::CellValue::Number(_)))
                    || matches!(&ds.archetype, TableArchetype::Xy(xy) if xy.x_value_at(row).is_some()));
            let excluded = ds.x_or_time_cell(row, xcol.subcol).is_some_and(|c| c.excluded);
            let is_x_selected = builder.app.grid.active_x_cell.is_some_and(|(ax_row, ax_sub)| ax_row == row && ax_sub == xcol.subcol);

            emit_single_cell_node(
                builder,
                CellRenderArgs {
                    ds,
                    sheet_wid: &metrics.scroll_id,
                    row,
                    col: 0,
                    subcol: xcol.subcol,
                    is_x: true,
                    rect: cell_rect,
                    display,
                    is_numeric,
                    excluded,
                    is_alt,
                    is_selected: is_x_selected,
                    is_editing,
                    scale: metrics.scale,
                    locale,
                    slot_tag,
                },
            );
        }

        for dcol in vis_data_cols {
            let cell_rect = LayoutRectPx::new(dcol.x_px, row_y, dcol.width_px, metrics.row_h);
            if !rect_visible_in_clip(cell_rect, body_clip) {
                continue;
            }
            let coord = CellCoord::new(row, dcol.col, dcol.subcol);
            let cell_opt = ds.cell(coord);
            let is_editing = builder.app.inline_editor.active
                && matches!(
                    &builder.app.inline_editor.target,
                    Some(crate::interaction::InlineEditorTarget::GridCell { sheet_id, coord: ec })
                        if *sheet_id == ds.id && ec.row == row && ec.col == dcol.col && ec.subcol == dcol.subcol
                );
            let display = if is_editing {
                builder.app.inline_editor.buffer.clone()
            } else {
                cell_opt.map(|c| format_cell_display(c, preset)).unwrap_or_default()
            };
            let is_numeric = !is_editing && cell_opt.is_some_and(|c| matches!(c.value, curvebook_doc::CellValue::Number(_)));
            let excluded = cell_opt.is_some_and(|c| c.excluded);

            let is_active = active.is_some_and(|a| a.row == row && a.col == dcol.col && a.subcol == dcol.subcol);
            let is_selected = if has_explicit_coords {
                is_active || vis_selected_coords.contains(&(row, dcol.col, dcol.subcol))
            } else {
                let in_bounds = sel_bounds.is_some_and(|(min_c, max_c)| {
                    row >= min_c.row
                        && row <= max_c.row
                        && (dcol.col, dcol.subcol) >= (min_c.col, min_c.subcol)
                        && (dcol.col, dcol.subcol) <= (max_c.col, max_c.subcol)
                });
                is_active || in_bounds
            };

            emit_single_cell_node(
                builder,
                CellRenderArgs {
                    ds,
                    sheet_wid: &metrics.scroll_id,
                    row,
                    col: dcol.col,
                    subcol: dcol.subcol,
                    is_x: false,
                    rect: cell_rect,
                    display,
                    is_numeric,
                    excluded,
                    is_alt,
                    is_selected,
                    is_editing,
                    scale: metrics.scale,
                    locale,
                    slot_tag,
                },
            );
        }
    }

    builder.pop_clip();
}

struct CellRenderArgs<'a> {
    ds: &'a DataSheet,
    sheet_wid: &'a WidgetId,
    row: usize,
    col: usize,
    subcol: usize,
    is_x: bool,
    rect: LayoutRectPx,
    display: String,
    is_numeric: bool,
    excluded: bool,
    is_alt: bool,
    is_selected: bool,
    is_editing: bool,
    scale: f64,
    locale: &'a str,
    slot_tag: &'a str,
}

fn emit_single_cell_node(builder: &mut LayoutBuilder<'_>, args: CellRenderArgs<'_>) {
    let palette = builder.palette;
    let region = if args.is_x { None } else { linked_region_for_cell(args.ds, args.row, args.col) };
    let link_color = region.map(|r| provenance_border_rgba(r.border_color().unwrap_or(LinkBorderColor::Blue)));

    let alt_surface = tint_alt_row(palette.surface, palette.panel);
    let fill = if args.is_editing {
        palette.surface
    } else if args.is_selected {
        Rgba8::rgba(palette.accent.r, palette.accent.g, palette.accent.b, 36)
    } else if let Some(lc) = link_color {
        Rgba8::rgba(lc.r, lc.g, lc.b, 22)
    } else if args.is_alt {
        alt_surface
    } else {
        palette.surface
    };

    let wid_raw =
        if args.is_x { format!("grid.xcell.{}.{}", args.row, args.subcol) } else { format!("grid.cell.{}.{}.{}", args.row, args.col, args.subcol) };
    let wid_str = scoped_id(args.slot_tag, &wid_raw);

    let excl_tip = if args.excluded && !args.is_x {
        crate::grid_view::GridViewState::cell_exclusion_tooltip(args.ds, GridCellCoord::new(args.row, args.col, args.subcol))
            .or_else(|| Some(grid_tr(args.locale, "w5.grid_core.excluded_cell", "Excluded cell").to_owned()))
    } else {
        None
    };

    let cell_a11y_label = if args.is_x {
        format!("X r{} s{}", args.row + 1, args.subcol + 1)
    } else {
        format!("r{} c{} s{}", args.row + 1, args.col + 1, args.subcol + 1)
    };
    let val_preview = if let Some(tip) = &excl_tip { format!("{} ({tip})", args.display) } else { args.display.clone() };
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(wid_str.as_str()),
        parent: Some(args.sheet_wid.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: args.rect,
        a11y: grid_cell_meta(cell_a11y_label, args.is_selected || args.is_editing, Some(val_preview.as_str())),
        state: widget_state(builder, &wid_str, ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });

    builder.paint_fill(ZLayer::Workspace, Some(&wid), args.rect, fill);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), args.rect, Rgba8::rgba(palette.border.r, palette.border.g, palette.border.b, 95), 1);

    if let (Some(reg), Some(lc)) = (region, link_color) {
        paint_linked_region_edges(builder, &wid, args.rect, reg, args.row, args.col, lc);
    }

    if args.is_selected || args.is_editing {
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), args.rect, palette.accent, 2);
    }

    if args.is_editing || !args.display.is_empty() {
        let fg = if args.excluded && !args.is_editing { palette.excluded } else { palette.text };
        let label = if args.is_editing {
            format!("{}|", args.display)
        } else if args.excluded {
            format!("{} *", args.display)
        } else {
            args.display
        };
        let font_pt = scale_pt(12.0, args.scale);
        let mut style = if args.is_numeric { UiTextStyle::grid_tabular(font_pt) } else { UiTextStyle::ui_sans(font_pt) };
        if args.excluded && !args.is_editing {
            style = style.italic();
        }
        let vis_rect = args.rect.intersect(builder.active_clip_rect());
        let text_rect = compute_aligned_text_rect(builder, vis_rect, &label, &style, args.is_numeric, 4);
        let _ = paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: text_rect,
                pad_x: 4,
                text: &label,
                style: &style,
                color: fg,
                scroll_container: Some(args.sheet_wid.as_str()),
            },
        );
    }

    if args.excluded {
        let strike_y = args.rect.y + i32::try_from(args.rect.height / 2).unwrap_or(12);
        let x0 = args.rect.x + 4;
        let x1 = args.rect.right() - 4;
        if x1 > x0 {
            builder.paint_line(ZLayer::Workspace, Some(&wid), [x0, strike_y], [x1, strike_y], palette.excluded, 1);
        }
        let has_note = !args.is_x && args.ds.exclusion_note(CellCoord::new(args.row, args.col, args.subcol)).is_some();
        if has_note && args.rect.width >= 10 && args.rect.height >= 10 {
            let tag_rect = LayoutRectPx::new(args.rect.right() - 6, args.rect.y + 1, 5, 5);
            builder.paint_fill(ZLayer::Workspace, Some(&wid), tag_rect, palette.excluded);
        }
    }
}

fn tint_alt_row(surface: Rgba8, panel: Rgba8) -> Rgba8 {
    let blend = |a: u8, b: u8| -> u8 {
        let v = (u16::from(a) * 3 + u16::from(b)) / 4;
        v.min(255) as u8
    };
    Rgba8::rgba(blend(surface.r, panel.r), blend(surface.g, panel.g), blend(surface.b, panel.b), surface.a)
}

fn compute_aligned_text_rect(
    builder: &LayoutBuilder<'_>,
    rect: LayoutRectPx,
    text: &str,
    style: &UiTextStyle,
    right_align: bool,
    min_pad: u32,
) -> LayoutRectPx {
    if !right_align {
        return rect;
    }
    let measured = builder.measurer.measure(text, style, &builder.tree.locale);
    let text_w = if measured.width_px.is_finite() && measured.width_px > 0.0 { measured.width_px.ceil() as u32 } else { 0 };
    let needed = text_w.saturating_add(min_pad.saturating_mul(2)).saturating_add(2);
    if rect.width > needed {
        let shift = rect.width.saturating_sub(needed);
        LayoutRectPx::new(rect.x.saturating_add(i32::try_from(shift).unwrap_or(0)), rect.y, rect.width.saturating_sub(shift), rect.height)
    } else {
        rect
    }
}

fn paint_linked_region_edges(
    builder: &mut LayoutBuilder<'_>,
    wid: &WidgetId,
    rect: LayoutRectPx,
    reg: &LinkedDataRegion,
    row: usize,
    col: usize,
    color: Rgba8,
) {
    let r = &reg.target_range;
    if row == r.start.row {
        builder.paint_fill(ZLayer::Workspace, Some(wid), LayoutRectPx::new(rect.x, rect.y, rect.width, 2), color);
    }
    if row == r.end.row {
        builder.paint_fill(ZLayer::Workspace, Some(wid), LayoutRectPx::new(rect.x, rect.bottom() - 2, rect.width, 2), color);
    }
    if col == r.start.col {
        builder.paint_fill(ZLayer::Workspace, Some(wid), LayoutRectPx::new(rect.x, rect.y, 2, rect.height), color);
    }
    if col == r.end.col {
        builder.paint_fill(ZLayer::Workspace, Some(wid), LayoutRectPx::new(rect.right() - 2, rect.y, 2, rect.height), color);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_gutter_body_window(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    state: &GridCoreViewState,
    metrics: &GridLayoutMetrics,
    row_num_x: i32,
    start_row: usize,
    end_row: usize,
    slot_tag: &str,
    locale: &str,
) {
    let gutter_clip = LayoutRectPx::new(metrics.gutter_x, metrics.data_y, metrics.row_num_w.saturating_add(metrics.rt_w), metrics.body_h);
    builder.push_clip(gutter_clip);

    let sy = if metrics.scroll_y >= 0.0 { metrics.scroll_y as i64 } else { 0 };
    let row_h_i = i64::from(metrics.row_h);
    let body_bottom = i64::from(metrics.data_y) + i64::from(metrics.body_h);
    let palette = builder.palette;
    let rn_style = UiTextStyle::grid_tabular(scale_pt(11.0, metrics.scale));

    for row in start_row..end_row {
        let row_y_i = i64::from(metrics.data_y) + (row as i64) * row_h_i - sy;
        if row_y_i >= body_bottom {
            break;
        }
        let row_y = i32::try_from(row_y_i).unwrap_or(0);
        let rn_rect = LayoutRectPx::new(row_num_x, row_y, metrics.row_num_w, metrics.row_h);
        if rect_visible_in_clip(rn_rect, gutter_clip) {
            let rn_id = scoped_id(slot_tag, &format!("grid.row_num.{row}"));
            let rn_num = (row + 1).to_string();
            let rn_wid = builder.push_node(NodeSpec {
                id: WidgetId::from(rn_id.as_str()),
                parent: Some(metrics.scroll_id.clone()),
                z_layer: ZLayer::Workspace,
                rect_px: rn_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, rn_num.clone(), FocusScope::Workspace),
                state: widget_state(builder, &rn_id, ControlEnablement::enabled()),
                action: WidgetAction::NoOp,
            });
            builder.paint_fill(ZLayer::Workspace, Some(&rn_wid), rn_rect, palette.header);
            builder.paint_stroke(ZLayer::Workspace, Some(&rn_wid), rn_rect, palette.border, 1);
            if row_y >= metrics.data_y && i64::from(rn_rect.bottom()) <= body_bottom {
                let _ = paint_text_fitted(
                    builder,
                    &rn_wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: rn_rect,
                        pad_x: 4,
                        text: &rn_num,
                        style: &rn_style,
                        color: palette.muted,
                        scroll_container: Some(metrics.scroll_id.as_str()),
                    },
                );
            }
        }

        let rt_rect = LayoutRectPx::new(metrics.rt_x, row_y, metrics.rt_w, metrics.row_h);
        if state.row_titles_collapsed {
            if rect_visible_in_clip(rt_rect, gutter_clip) {
                let c_id = scoped_id(slot_tag, &format!("grid.row_title_collapsed.{row}"));
                let c_wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(c_id.as_str()),
                    parent: Some(metrics.scroll_id.clone()),
                    z_layer: ZLayer::Workspace,
                    rect_px: rt_rect,
                    a11y: AccessibilityMeta::new(WidgetRole::Label, "", FocusScope::Workspace),
                    state: widget_state(builder, &c_id, ControlEnablement::enabled()),
                    action: WidgetAction::NoOp,
                });
                builder.paint_fill(ZLayer::Workspace, Some(&c_wid), rt_rect, palette.header);
                builder.paint_stroke(ZLayer::Workspace, Some(&c_wid), rt_rect, palette.border, 1);
            }
        } else if rect_visible_in_clip(rt_rect, gutter_clip) {
            let title_text = archetype_row_title(&ds.archetype, row).unwrap_or("");
            let a11y_lbl = if title_text.is_empty() {
                format!("{} {}", grid_tr(locale, "w5.grid_core.row_titles", "Row Titles"), row + 1)
            } else {
                title_text.to_owned()
            };
            let rt_wid_str = scoped_id(slot_tag, &format!("grid.row_title.{row}"));
            let rt_wid = emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &rt_wid_str,
                    parent: Some(metrics.scroll_id.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: rt_rect,
                    label: title_text,
                    keyboard_path: "F2",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: if ds.frozen { ControlEnablement::disabled("Frozen") } else { ControlEnablement::enabled() },
                    action: WidgetAction::Custom {
                        domain: String::from("grid_core"),
                        payload: scoped_payload(slot_tag, &format!("title:begin:row:{row}")),
                    },
                },
            );
            // Replace empty visual label a11y with non-empty descriptive label when blank.
            if title_text.is_empty()
                && let Some(node) = builder.tree.nodes.iter_mut().rev().find(|n| n.id == rt_wid)
            {
                node.a11y.label = a11y_lbl;
            }
        }
    }

    builder.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn emit_column_headers_3tier(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    metrics: &GridLayoutMetrics,
    preset: &LocalePreset,
    locale: &str,
    vis_x_cols: &[VisibleSubcol],
    vis_data_cols: &[VisibleSubcol],
    vis_spans: &[ColSpanSlice],
    slot_tag: &str,
) {
    let is_rtl = builder.tree.is_rtl;
    let header_clip = LayoutRectPx::new(metrics.cols_x, metrics.header_y, metrics.cols_w, metrics.header_h);
    builder.push_clip(header_clip);

    let y1 = metrics.header_y;
    let y2 = y1.saturating_add(i32::try_from(metrics.tier1_h).unwrap_or(20));
    let y3 = y2.saturating_add(i32::try_from(metrics.tier2_h).unwrap_or(24));
    let palette = builder.palette;
    let hdr_style = UiTextStyle::ui_bold(scale_pt(11.0, metrics.scale));
    let sub_style = UiTextStyle::ui_sans(scale_pt(11.0, metrics.scale));
    let rw = metrics.resize_w;
    let handle_h = metrics.tier2_h.saturating_add(metrics.tier3_h);

    // Tier 1 + Tier 2 + Tier 3 for X column(s).
    for xcol in vis_x_cols {
        let r1 = LayoutRectPx::new(xcol.x_px, y1, xcol.width_px, metrics.tier1_h);
        let x_ident = grid_tr(locale, "w5.grid_core.x_col", "X");
        emit_header_band_cell(
            builder,
            &scoped_id(slot_tag, &format!("grid.col_super.x.{}", xcol.subcol)),
            r1,
            x_ident,
            palette.header,
            palette.muted,
            &hdr_style,
        );

        let r2_w = xcol.width_px.saturating_sub(rw + 1);
        let content_x = if is_rtl { xcol.x_px.saturating_add(i32::try_from(rw + 1).unwrap_or(0)) } else { xcol.x_px };
        let r2 = LayoutRectPx::new(content_x, y2, r2_w, metrics.tier2_h);
        if r2_w >= 8 && rect_visible_in_clip(r2, header_clip) {
            let x_title = x_column_header_title(ds, x_ident);
            let x_wid_str = scoped_id(slot_tag, &format!("grid.x_title.{}", xcol.subcol));
            let _ = emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &x_wid_str,
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect: r2,
                    label: &x_title,
                    keyboard_path: "F2",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: if ds.frozen { ControlEnablement::disabled("Frozen") } else { ControlEnablement::enabled() },
                    action: WidgetAction::Custom { domain: String::from("grid_core"), payload: scoped_payload(slot_tag, "title:begin:x") },
                },
            );
        }

        let resize_x = if is_rtl { xcol.x_px } else { xcol.x_px.saturating_add(i32::try_from(xcol.width_px.saturating_sub(rw)).unwrap_or(0)) };
        let resize_rect = LayoutRectPx::new(resize_x, y2, rw, handle_h);
        if rect_visible_in_clip(resize_rect, header_clip) {
            emit_col_resize_handle(
                builder,
                &scoped_id(slot_tag, &format!("grid.col_resize.x.{}", xcol.subcol)),
                resize_rect,
                scoped_payload(slot_tag, &format!("xcol:resize_step:{}", xcol.subcol)),
                locale,
            );
        }

        if r2_w >= 8 {
            let r3 = LayoutRectPx::new(content_x, y3, r2_w, metrics.tier3_h);
            let x_sub = x_subcolumn_label(ds, xcol.subcol);
            emit_header_band_cell(
                builder,
                &scoped_id(slot_tag, &format!("grid.subcol_hdr.x.{}", xcol.subcol)),
                r3,
                &x_sub,
                palette.panel,
                palette.muted,
                &sub_style,
            );
        }
    }

    // Tier 1 (super-column spans) + Tier 2 (editable dataset title + MV fx button).
    let is_mv = matches!(ds.archetype, TableArchetype::MultipleVariables(_));
    for span in vis_spans {
        let super_label = super_column_band_label(&ds.archetype, span.col);
        let r1 = LayoutRectPx::new(span.x_px, y1, span.width_px, metrics.tier1_h);
        emit_header_band_cell(
            builder,
            &scoped_id(slot_tag, &format!("grid.col_super.{}", span.col)),
            r1,
            &super_label,
            palette.header,
            palette.muted,
            &hdr_style,
        );

        let fx_w = if is_mv && span.width_px > rw + scale_u32(28, metrics.scale) { scale_u32(22, metrics.scale) } else { 0 };
        let title_w = span.width_px.saturating_sub(rw + 1 + fx_w);
        let col_title = crate::dialogs_canvas::data_column_title(ds, span.col)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| curvebook_doc::default_column_label(span.col));
        let title_x = if is_rtl { span.x_px.saturating_add(i32::try_from(rw + 1).unwrap_or(0)) } else { span.x_px };
        let r2_lbl = LayoutRectPx::new(title_x, y2, title_w, metrics.tier2_h);
        if title_w >= 8 && rect_visible_in_clip(r2_lbl, header_clip) {
            let wid_str = scoped_id(slot_tag, &format!("grid.col_title.{}", span.col));
            let _ = emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &wid_str,
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect: r2_lbl,
                    label: &col_title,
                    keyboard_path: "F2",
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: false,
                    shortcut: None,
                    enablement: if ds.frozen { ControlEnablement::disabled("Frozen") } else { ControlEnablement::enabled() },
                    action: WidgetAction::Custom {
                        domain: String::from("grid_core"),
                        payload: scoped_payload(slot_tag, &format!("title:begin:col:{}", span.col)),
                    },
                },
            );
        }

        if fx_w > 0 && (slot_tag != "secondary" || builder.tree.node(&format!("formula_bar.open_col.{}", span.col)).is_none()) {
            let full_hdr = LayoutRectPx::new(span.x_px, y2, span.width_px, metrics.tier2_h);
            if rect_visible_in_clip(full_hdr, header_clip) {
                mv_formula::emit_mv_column_fx_button(builder, ds, span.col, r2_lbl, full_hdr, fx_w);
            }
        }
    }

    // Tier 3 (replicate/subcolumn headers) + column-edge resize handles.
    for dcol in vis_data_cols {
        let sub_label = subcolumn_header_label(ds, dcol.col, dcol.subcol, preset);
        let btn_w = dcol.width_px.saturating_sub(rw + 1);
        let dcol_x = if is_rtl { dcol.x_px.saturating_add(i32::try_from(rw + 1).unwrap_or(0)) } else { dcol.x_px };
        if btn_w >= 8 {
            let r3 = LayoutRectPx::new(dcol_x, y3, btn_w, metrics.tier3_h);
            let sub_wid = scoped_id(slot_tag, &format!("grid.subcol_title.{}.{}", dcol.col, dcol.subcol));
            if matches!(ds.archetype, TableArchetype::Nested(_)) {
                if rect_visible_in_clip(r3, header_clip) {
                    let _ = emit_button(
                        builder,
                        ButtonWidgetSpec {
                            id: &sub_wid,
                            parent: None,
                            z_layer: ZLayer::Workspace,
                            rect: r3,
                            label: &sub_label,
                            keyboard_path: "F2",
                            focus_scope: FocusScope::Workspace,
                            tab_order: None,
                            primary: false,
                            selected: false,
                            shortcut: None,
                            enablement: if ds.frozen { ControlEnablement::disabled("Frozen") } else { ControlEnablement::enabled() },
                            action: WidgetAction::Custom {
                                domain: String::from("grid_core"),
                                payload: scoped_payload(slot_tag, &format!("title:begin:subcol:{}:{}", dcol.col, dcol.subcol)),
                            },
                        },
                    );
                }
            } else {
                emit_header_band_cell(builder, &sub_wid, r3, &sub_label, palette.panel, palette.muted, &sub_style);
            }
        }

        let sub_count = data_set_subcolumn_count(&ds.archetype, dcol.col).max(1);
        let is_group_end = dcol.subcol + 1 >= sub_count;
        let rx = if is_rtl { dcol.x_px } else { dcol.x_px.saturating_add(i32::try_from(dcol.width_px.saturating_sub(rw)).unwrap_or(0)) };
        let (ry, rh) = if is_group_end { (y2, handle_h) } else { (y3, metrics.tier3_h) };
        let resize_rect = LayoutRectPx::new(rx, ry, rw, rh);
        if rect_visible_in_clip(resize_rect, header_clip) {
            emit_col_resize_handle(
                builder,
                &scoped_id(slot_tag, &format!("grid.col_resize.{}.{}", dcol.col, dcol.subcol)),
                resize_rect,
                scoped_payload(slot_tag, &format!("col:resize_step:{}:{}", dcol.col, dcol.subcol)),
                locale,
            );
        }
    }

    builder.pop_clip();
}

fn emit_header_band_cell(builder: &mut LayoutBuilder<'_>, wid_str: &str, rect: LayoutRectPx, label: &str, bg: Rgba8, fg: Rgba8, style: &UiTextStyle) {
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(wid_str),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, label, FocusScope::Workspace),
        state: widget_state(builder, wid_str, ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), rect, builder.palette.border, 1);
    if rect.width >= 10 && !label.is_empty() {
        let _ = paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec { z_layer: ZLayer::Workspace, rect, pad_x: 4, text: label, style, color: fg, scroll_container: None },
        );
    }
}

fn emit_col_resize_handle(builder: &mut LayoutBuilder<'_>, wid_str: &str, rect: LayoutRectPx, payload: String, locale: &str) {
    let st = widget_state(builder, wid_str, ControlEnablement::enabled());
    let mut a11y = AccessibilityMeta::new(WidgetRole::Button, grid_tr(locale, "w5.grid_core.resize_column", "Resize column"), FocusScope::Workspace);
    a11y.keyboard_path = Some(String::from("Enter"));
    let color = if st.hovered || st.active { builder.palette.accent } else { builder.palette.border };
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(wid_str),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y,
        state: st,
        action: WidgetAction::Custom { domain: String::from("grid_core"), payload },
    });
    builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, color);
}

fn emit_gutter_header(
    builder: &mut LayoutBuilder<'_>,
    state: &GridCoreViewState,
    metrics: &GridLayoutMetrics,
    row_num_x: i32,
    slot_tag: &str,
    locale: &str,
) {
    let corner_clip = LayoutRectPx::new(metrics.gutter_x, metrics.header_y, metrics.row_num_w.saturating_add(metrics.rt_w), metrics.header_h);
    builder.push_clip(corner_clip);

    let rn_rect = LayoutRectPx::new(row_num_x, metrics.header_y, metrics.row_num_w, metrics.header_h);
    let hash_label = grid_tr(locale, "w5.grid_core.row_number_header", "#");
    let hdr_style = UiTextStyle::ui_bold(scale_pt(11.0, metrics.scale));
    emit_header_band_cell(
        builder,
        &scoped_id(slot_tag, "grid.corner.row_num"),
        rn_rect,
        hash_label,
        builder.palette.header,
        builder.palette.muted,
        &hdr_style,
    );

    let toggle_id = scoped_id(slot_tag, "grid.row_titles.toggle");
    if state.row_titles_collapsed {
        let rt_rect = LayoutRectPx::new(metrics.rt_x, metrics.header_y, metrics.rt_w, metrics.header_h);
        let expand_lbl = grid_tr(locale, "w5.grid_core.expand_row_titles", "Expand Row Titles");
        let short_lbl = grid_tr(locale, "w5.grid_core.row_titles_short", "RT");
        let t_wid = emit_button(
            builder,
            ButtonWidgetSpec {
                id: &toggle_id,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: rt_rect,
                label: short_lbl,
                keyboard_path: "Enter",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("grid_core"), payload: scoped_payload(slot_tag, "row_titles:toggle") },
            },
        );
        if let Some(node) = builder.tree.nodes.iter_mut().rev().find(|n| n.id == t_wid) {
            node.a11y.label = expand_lbl.to_owned();
            node.a11y.expanded = Some(false);
        }
    } else {
        let rw = metrics.resize_w;
        let btn_w = metrics.rt_w.saturating_sub(rw + 1);
        let rt_title = grid_tr(locale, "w5.grid_core.row_titles", "Row Titles");
        let collapse_lbl = grid_tr(locale, "w5.grid_core.collapse_row_titles", "Collapse Row Titles");

        let top_rect = LayoutRectPx::new(metrics.rt_x, metrics.header_y, btn_w, metrics.tier1_h);
        emit_header_band_cell(
            builder,
            &scoped_id(slot_tag, "grid.row_titles.band"),
            top_rect,
            rt_title,
            builder.palette.header,
            builder.palette.text,
            &hdr_style,
        );

        let toggle_rect = LayoutRectPx::new(
            metrics.rt_x,
            metrics.header_y.saturating_add(i32::try_from(metrics.tier1_h).unwrap_or(20)),
            btn_w,
            metrics.tier2_h.saturating_add(metrics.tier3_h),
        );
        let t_wid = emit_button(
            builder,
            ButtonWidgetSpec {
                id: &toggle_id,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: toggle_rect,
                label: "←",
                keyboard_path: "Enter",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("grid_core"), payload: scoped_payload(slot_tag, "row_titles:toggle") },
            },
        );
        if let Some(node) = builder.tree.nodes.iter_mut().rev().find(|n| n.id == t_wid) {
            node.a11y.label = collapse_lbl.to_owned();
            node.a11y.expanded = Some(true);
        }

        let resize_id = scoped_id(slot_tag, "grid.row_titles.resize");
        let rx = metrics.rt_x.saturating_add(i32::try_from(metrics.rt_w.saturating_sub(rw)).unwrap_or(0));
        let resize_rect = LayoutRectPx::new(rx, metrics.header_y, rw, metrics.header_h);
        let rst = widget_state(builder, &resize_id, ControlEnablement::enabled());
        let color = if rst.hovered || rst.active { builder.palette.accent } else { builder.palette.border };
        let mut r_a11y = AccessibilityMeta::new(
            WidgetRole::Button,
            grid_tr(locale, "w5.grid_core.resize_row_titles", "Resize Row Titles column"),
            FocusScope::Workspace,
        );
        r_a11y.keyboard_path = Some(String::from("Enter"));
        let r_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(resize_id.as_str()),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: resize_rect,
            a11y: r_a11y,
            state: rst,
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: scoped_payload(slot_tag, "row_titles:resize_step") },
        });
        builder.paint_fill(ZLayer::Workspace, Some(&r_wid), resize_rect, color);
    }

    builder.pop_clip();
}

fn emit_link_provenance_strip(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    state: &GridCoreViewState,
    rect: LayoutRectPx,
    slot_tag: &str,
    locale: &str,
    scale: f64,
) {
    let strip_id = scoped_id(slot_tag, "grid.link_strip");
    let strip_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(strip_id.as_str()),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: AccessibilityMeta::new(
            WidgetRole::Label,
            grid_tr(locale, "w5.grid_core.linked_region_title", "Linked Data Region"),
            FocusScope::Workspace,
        ),
        state: widget_state(builder, &strip_id, ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&strip_wid), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&strip_wid), rect, builder.palette.border, 1);

    let mut cur_x = rect.x + 6;
    let badge_lbl = grid_tr(locale, "w5.grid_core.linked_badge", "Linked");
    let chip_style = UiTextStyle::ui_sans(scale_pt(11.0, scale));
    let max_chip_w = scale_u32(240, scale);
    for (idx, reg) in ds.linked_regions.iter().enumerate() {
        let rem = u32::try_from((rect.right() - cur_x).max(0)).unwrap_or(0);
        if rem < 72 {
            break;
        }
        let chip_w = max_chip_w.min(rem.saturating_sub(4));
        let chip_rect = LayoutRectPx::new(cur_x, rect.y + 3, chip_w, rect.height.saturating_sub(6));
        let color = provenance_border_rgba(reg.border_color().unwrap_or(LinkBorderColor::Blue));
        let text =
            if reg.provenance_summary.is_empty() { format!("{badge_lbl} #{}", idx + 1) } else { format!("{badge_lbl}: {}", reg.provenance_summary) };
        let chip_id = scoped_id(slot_tag, &format!("grid.link_badge.{idx}"));
        let st = widget_state(builder, &chip_id, ControlEnablement::enabled());
        let selected = state.active_link_popover.as_ref().is_some_and(|p| p.region_id == reg.id);
        let chip_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(chip_id.as_str()),
            parent: Some(strip_wid.clone()),
            z_layer: ZLayer::Workspace,
            rect_px: chip_rect,
            a11y: selectable_meta(WidgetRole::Button, text.clone(), FocusScope::Workspace, "Enter", selected),
            state: st.clone(),
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: scoped_payload(slot_tag, &format!("link:open:{}", reg.id)) },
        });
        builder.paint_fill(ZLayer::Workspace, Some(&chip_wid), chip_rect, Rgba8::rgba(color.r, color.g, color.b, if st.hovered { 60 } else { 32 }));
        builder.paint_stroke(ZLayer::Workspace, Some(&chip_wid), chip_rect, color, 2);
        let _ = paint_text_fitted(
            builder,
            &chip_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: chip_rect,
                pad_x: 6,
                text: &text,
                style: &chip_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        cur_x += i32::try_from(chip_w).unwrap_or(0) + 6;
    }
}

fn emit_inline_title_editor_overlay(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    state: &GridCoreViewState,
    metrics: &GridLayoutMetrics,
    locale: &str,
) {
    let Some(edit) = &state.editing_title else {
        return;
    };
    if edit.sheet_id != ds.id.to_string() {
        return;
    }

    let scale = metrics.scale;
    let box_w = scale_u32(340, scale).min(metrics.grid_rect.width.saturating_sub(12));
    let box_h = scale_u32(38, scale);
    if box_w < 160 || metrics.grid_rect.height < box_h + 12 {
        return;
    }
    let ox = metrics.grid_rect.x + 8;
    let oy = metrics.grid_rect.y + i32::try_from(metrics.tier1_h).unwrap_or(20);
    let box_rect = LayoutRectPx::new(ox, oy, box_w, box_h);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("grid.title_editor.panel"),
        parent: None,
        z_layer: ZLayer::FloatingOverlay,
        rect_px: box_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, grid_tr(locale, "w5.grid_core.edit_title", "Edit Title"), FocusScope::Workspace),
        state: widget_state(builder, "grid.title_editor.panel", ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&panel_wid), box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&panel_wid), box_rect, builder.palette.accent, 2);

    let btn_w = scale_u32(64, scale);
    let gap = scale_u32(6, scale);
    let v_pad = scale_u32(6, scale);
    let inner_h = box_h.saturating_sub(v_pad * 2);
    let input_w = box_w.saturating_sub(btn_w * 2 + gap * 4);
    let gap_i = i32::try_from(gap).unwrap_or(6);
    let vpad_i = i32::try_from(v_pad).unwrap_or(6);
    let input_rect = LayoutRectPx::new(ox + gap_i, oy + vpad_i, input_w, inner_h);
    let apply_rect = LayoutRectPx::new(input_rect.right() + gap_i, oy + vpad_i, btn_w, inner_h);
    let cancel_rect = LayoutRectPx::new(apply_rect.right() + gap_i, oy + vpad_i, btn_w, inner_h);

    let _ = emit_text_field(
        builder,
        TextFieldSpec {
            id: "grid.title_editor.input",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: input_rect,
            label: grid_tr(locale, "w5.grid_core.dataset_title", "Dataset Title"),
            fallback_text: &edit.draft,
            keyboard_path: "F2",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let apply_lbl = grid_tr(locale, "w5.grid_core.apply", "Apply");
    let _ = emit_button(
        builder,
        ButtonWidgetSpec {
            id: "grid.title_editor.apply",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: apply_rect,
            label: apply_lbl,
            keyboard_path: "Enter",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: true,
            selected: false,
            shortcut: Some("Enter"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: String::from("title:commit") },
        },
    );

    let cancel_lbl = grid_tr(locale, "w5.grid_core.cancel", "Cancel");
    let _ = emit_button(
        builder,
        ButtonWidgetSpec {
            id: "grid.title_editor.cancel",
            parent: Some(panel_wid),
            z_layer: ZLayer::FloatingOverlay,
            rect: cancel_rect,
            label: cancel_lbl,
            keyboard_path: "Escape",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: Some("Esc"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: String::from("title:cancel") },
        },
    );
}

fn emit_linked_region_popover(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, state: &GridCoreViewState, metrics: &GridLayoutMetrics, locale: &str) {
    let Some(pop) = &state.active_link_popover else {
        return;
    };
    if pop.sheet_id != ds.id.to_string() {
        return;
    }
    let Some(reg) = ds.linked_regions.iter().find(|r| r.id == pop.region_id) else {
        return;
    };

    let scale = metrics.scale;
    let pop_w = scale_u32(430, scale).min(metrics.grid_rect.width.saturating_sub(16));
    let has_warn = state.link_edit_warning.as_ref().is_some_and(|w| w.region_id == reg.id);
    let pop_h = scale_u32(if has_warn { 112 } else { 88 }, scale);
    if pop_w < 200 || metrics.grid_rect.height < pop_h + 16 {
        return;
    }
    let px = metrics.grid_rect.x + i32::try_from(scale_u32(12, scale)).unwrap_or(12);
    let py = metrics.grid_rect.y + i32::try_from(scale_u32(40, scale)).unwrap_or(40);
    let pop_rect = LayoutRectPx::new(px, py, pop_w, pop_h);

    let border_col = provenance_border_rgba(reg.border_color().unwrap_or(LinkBorderColor::Blue));
    let title = grid_tr(locale, "w5.grid_core.linked_region_title", "Linked Data Region");
    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("grid.link_popover.panel"),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: pop_rect,
        a11y: labeled_meta(WidgetRole::Label, title, FocusScope::PopupMenu, "Escape"),
        state: widget_state(builder, "grid.link_popover.panel", ControlEnablement::enabled()),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::DropdownMenu, Some(&panel_wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&panel_wid), pop_rect, border_col, 2);

    let hdr_style = UiTextStyle::ui_bold(scale_pt(12.0, scale));
    let body_style = UiTextStyle::ui_sans(scale_pt(11.0, scale));
    let line_h1 = scale_u32(18, scale);
    let line_h2 = scale_u32(16, scale);
    let pad_h = i32::try_from(scale_u32(8, scale)).unwrap_or(8);
    let _ = paint_text_fitted(
        builder,
        &panel_wid,
        FittedTextSpec {
            z_layer: ZLayer::DropdownMenu,
            rect: LayoutRectPx::new(px + pad_h, py + i32::try_from(scale_u32(6, scale)).unwrap_or(6), pop_w - 16, line_h1),
            pad_x: 2,
            text: title,
            style: &hdr_style,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let summary = if reg.provenance_summary.is_empty() { reg.id.clone() } else { reg.provenance_summary.clone() };
    let _ = paint_text_fitted(
        builder,
        &panel_wid,
        FittedTextSpec {
            z_layer: ZLayer::DropdownMenu,
            rect: LayoutRectPx::new(px + pad_h, py + i32::try_from(scale_u32(26, scale)).unwrap_or(26), pop_w - 16, line_h2),
            pad_x: 2,
            text: &summary,
            style: &body_style,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    if has_warn {
        let msg = grid_tr(locale, "w5.grid_core.linked_read_only_prompt", "Linked region is read-only. Go to source or unlink to edit.");
        let _ = paint_text_fitted(
            builder,
            &panel_wid,
            FittedTextSpec {
                z_layer: ZLayer::DropdownMenu,
                rect: LayoutRectPx::new(px + pad_h, py + i32::try_from(scale_u32(46, scale)).unwrap_or(46), pop_w - 16, line_h1),
                pad_x: 2,
                text: msg,
                style: &body_style,
                color: builder.palette.link_red,
                scroll_container: None,
            },
        );
    }

    let btn_h = scale_u32(24, scale);
    let btn_y = pop_rect.bottom() - i32::try_from(scale_u32(32, scale)).unwrap_or(32);
    let avail_btns = pop_w.saturating_sub(32);
    let goto_w = (avail_btns * 36 / 100).max(56);
    let unlink_w = (avail_btns * 38 / 100).max(56);
    let close_w = avail_btns.saturating_sub(goto_w + unlink_w).max(48);
    let goto_rect = LayoutRectPx::new(px + 10, btn_y, goto_w, btn_h);
    let unlink_rect = LayoutRectPx::new(goto_rect.right() + 6, btn_y, unlink_w, btn_h);
    let close_rect = LayoutRectPx::new(unlink_rect.right() + 6, btn_y, close_w, btn_h);

    let goto_lbl = grid_tr(locale, "w5.grid_core.go_to_source", "Go To Source");
    let has_upstream = reg.spec.source.upstream_sheet_id().is_some();
    let _ = emit_button(
        builder,
        ButtonWidgetSpec {
            id: "grid.link_popover.goto",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect: goto_rect,
            label: goto_lbl,
            keyboard_path: "Enter",
            focus_scope: FocusScope::PopupMenu,
            tab_order: None,
            primary: true,
            selected: false,
            shortcut: None,
            enablement: if has_upstream { ControlEnablement::enabled() } else { ControlEnablement::disabled("External source") },
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: format!("link:goto:{}", reg.id) },
        },
    );

    let unlink_lbl = grid_tr(locale, "w5.grid_core.unlink", "Unlink");
    let _ = emit_button(
        builder,
        ButtonWidgetSpec {
            id: "grid.link_popover.unlink",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect: unlink_rect,
            label: unlink_lbl,
            keyboard_path: "Enter",
            focus_scope: FocusScope::PopupMenu,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: if ds.frozen { ControlEnablement::disabled("Frozen") } else { ControlEnablement::enabled() },
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: format!("link:unlink:{}", reg.id) },
        },
    );

    let close_lbl = grid_tr(locale, "w5.grid_core.close", "Close");
    let _ = emit_button(
        builder,
        ButtonWidgetSpec {
            id: "grid.link_popover.close",
            parent: Some(panel_wid),
            z_layer: ZLayer::DropdownMenu,
            rect: close_rect,
            label: close_lbl,
            keyboard_path: "Escape",
            focus_scope: FocusScope::PopupMenu,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: Some("Esc"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("grid_core"), payload: String::from("link:close") },
        },
    );
}

fn super_column_band_label(arch: &TableArchetype, col: usize) -> String {
    let alpha = curvebook_doc::default_column_label(col);
    match arch {
        TableArchetype::Grouped(_) | TableArchetype::Nested(_) => format!("Group {alpha}"),
        TableArchetype::Xy(_) if arch.column_count() == 1 => String::from("Y"),
        TableArchetype::Xy(_) => format!("Y ({alpha})"),
        _ => alpha,
    }
}

fn archetype_x_title(arch: &TableArchetype) -> Option<&str> {
    match arch {
        TableArchetype::Xy(xy) => Some(xy.x_title.as_str()),
        TableArchetype::Survival(surv) => Some(surv.time_title.as_str()),
        _ => None,
    }
}

fn archetype_row_title(arch: &TableArchetype, row: usize) -> Option<&str> {
    let slice: &[String] = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    slice.get(row).map(String::as_str)
}

fn x_column_header_title(ds: &DataSheet, default_x: &str) -> String {
    if let Some(title) = archetype_x_title(&ds.archetype)
        && !title.is_empty()
    {
        return title.to_owned();
    }
    default_x.to_owned()
}

fn x_subcolumn_label(ds: &DataSheet, subcol: usize) -> String {
    match &ds.archetype {
        TableArchetype::Xy(xy) => match &xy.x_mode {
            XColumnMode::DateOrElapsedTime { unit_label } if !unit_label.is_empty() => unit_label.clone(),
            XColumnMode::DateOrElapsedTime { .. } => String::from("Time"),
            XColumnMode::GeneratedSequence { .. } => String::from("Seq"),
            XColumnMode::Numbers { .. } => format!("X{}", subcol + 1),
        },
        TableArchetype::Survival(_) => String::from("Time"),
        _ => String::from("X"),
    }
}

fn nested_subcolumn_title(ds: &DataSheet, col: usize, subcol: usize) -> Option<String> {
    if let TableArchetype::Nested(nested) = &ds.archetype {
        return nested.groups.get(col).and_then(|g| g.subcolumn_titles.get(subcol)).cloned();
    }
    None
}

fn subcolumn_header_label(ds: &DataSheet, col: usize, subcol: usize, preset: &LocalePreset) -> String {
    let locale = preset.locale.as_str();
    match &ds.archetype {
        TableArchetype::Nested(nested) => nested
            .groups
            .get(col)
            .and_then(|g| g.subcolumn_titles.get(subcol))
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Sub {}", subcol + 1)),
        TableArchetype::Survival(_) => {
            if subcol == 0 {
                String::from("Event (0/1)")
            } else {
                format!("Sub {}", subcol + 1)
            }
        }
        TableArchetype::Xy(xy) => replicate_subcol_label(&xy.y_subcolumn_mode, data_set_subcolumn_count(&ds.archetype, col), subcol, locale),
        TableArchetype::Grouped(gr) => replicate_subcol_label(&gr.subcolumn_mode, data_set_subcolumn_count(&ds.archetype, col), subcol, locale),
        TableArchetype::MultipleVariables(mv) => mv.variables.get(col).map(|v| format!("{:?}", v.role)).unwrap_or_else(|| String::from("Var")),
        TableArchetype::Column(_) | TableArchetype::Contingency(_) | TableArchetype::PartsOfWhole(_) => {
            if subcol == 0 {
                String::from("Y")
            } else {
                format!("Y{}", subcol + 1)
            }
        }
    }
}

fn replicate_subcol_label(mode: &curvebook_doc::SubcolumnMode, sub_count: usize, subcol: usize, locale: &str) -> String {
    use curvebook_doc::SubcolumnMode;
    match mode {
        SubcolumnMode::MeanSdN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sd_col", "SD").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::MeanSemN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sem_col", "SEM").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::MeanCvN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => String::from("%CV"),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::MeanLowHigh => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => String::from("-Err"),
            2 => String::from("+Err"),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::UpperLowerLimits => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.min_col", "Min").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.max_col", "Max").to_owned(),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::MedianIqr => match subcol {
            0 => grid_tr(locale, "w5.grid_core.median_col", "Median").to_owned(),
            1 => String::from("Q1"),
            2 => String::from("Q3"),
            _ => format!("S{}", subcol + 1),
        },
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
            if sub_count <= 1 {
                String::from("Y")
            } else {
                format!("Y{}", subcol + 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, DataSheet, MultipleVariablesTable, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
    use curvebook_engine::Session;

    use crate::UiApp;
    use crate::human_driver::HumanPathDriver;
    use crate::interaction::ViewportMetrics;
    use crate::layout::WidgetId;

    fn build_large_mv_driver(cols: usize, rows: usize, locale: &str) -> Result<HumanPathDriver, String> {
        let mut mv = MultipleVariablesTable::new(cols, rows).map_err(|e| e.to_string())?;
        for r in 0..rows {
            mv.row_titles[r] = format!("Row {}", r + 1);
            for (c, var) in mv.variables.iter_mut().enumerate() {
                var.cells[r] = Cell::number((r as f64) * 10.0 + (c as f64));
            }
        }
        let ds = DataSheet::new("mv-big", "Big MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
        let mut doc = ProjectDocument::new("Large Grid Parity").map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.select_in_navigator(&SheetId::from("mv-big")).map_err(|e| e.to_string())?;
        let mut app = UiApp::from_session(Session::from_document(doc));
        let _ = app.set_locale(locale);
        Ok(HumanPathDriver::new(app, ViewportMetrics::new(1280.0, 800.0, 1.0)))
    }

    #[test]
    fn large_mv_bottom_partial_row_passes_parity_ltr_and_rtl() -> Result<(), String> {
        // 4 cols at 1280x800 has hscroll_reserve == 0 and >= 26 rows (row 25 clipped right above status.summary).
        for locale in ["en", "ar"] {
            let mut driver = build_large_mv_driver(4, 120, locale)?;
            for scroll_y in [0.0, 10.0, 18.0] {
                driver.app_mut().interaction.scroll_offsets.offsets.insert(String::from("grid.sheet.mv-big"), [0.0, scroll_y]);
                let report = driver.verify_parity();
                assert!(report.is_clean(), "{locale} scroll_y={scroll_y} parity violations: {:?}", report.violations);
            }
            if locale == "ar" {
                let tree = driver.layout_tree();
                let sheet = tree.node("grid.sheet.mv-big").ok_or_else(|| String::from("missing grid.sheet.mv-big"))?;
                let vscroll = tree.node("grid.sheet.mv-big.vscroll").ok_or_else(|| String::from("missing RTL vscroll"))?;
                assert_eq!(vscroll.bounds.rect_px.x, sheet.bounds.rect_px.x, "RTL vertical scrollbar must sit on left edge");
                if let Some(grid) = &tree.grid_geometry {
                    let gutter = grid.row_gutter_rect(0);
                    assert!(gutter.x >= grid.sheet_rect.right().saturating_sub(grid.gutter_w + 2), "RTL gutter must flush right");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn grid_cells_parent_and_fractional_wheel_scroll_align_hit_test() -> Result<(), String> {
        let mut driver = build_large_mv_driver(5, 130, "en")?;
        let tree0 = driver.layout_tree();
        let sheet_id = WidgetId::from("grid.sheet.mv-big");
        let c0 = tree0.node("grid.cell.0.0.0").ok_or_else(|| String::from("missing grid.cell.0.0.0"))?;
        assert_eq!(c0.parent.as_ref(), Some(&sheet_id), "grid.cell must parent to grid.sheet.<id>");

        // Non-multiple-of-24 vertical scroll offsets must align GridHitGeometry::hit_test_cell exactly.
        for scroll_y in [18.0_f64, 2730.0_f64] {
            driver.app_mut().interaction.scroll_offsets.offsets.insert(String::from("grid.sheet.mv-big"), [0.0, scroll_y]);
            let tree = driver.layout_tree();
            let geom = tree.grid_geometry.as_ref().ok_or_else(|| String::from("missing grid_geometry"))?;
            let mut checked = 0usize;
            for r in (geom.start_row + 1)..geom.end_row.saturating_sub(1) {
                let id = format!("grid.cell.{r}.0.0");
                if let Some(node) = tree.node(&id)
                    && node.bounds.visible_rect_px.height >= 12
                {
                    let [cx, cy] = node.bounds.visible_rect_px.center();
                    let hit = geom.hit_test_cell(cx as u32, cy as u32);
                    assert_eq!(hit.map(|(coord, _)| coord.row), Some(r), "scroll_y={scroll_y}: expected row {r}, got {hit:?}");
                    checked += 1;
                }
            }
            assert!(checked >= 5, "expected multiple visible cells verified at scroll_y={scroll_y}");
        }

        // XY X-cells must also parent to grid.sheet.<id> and hit-test cleanly at fractional scroll.
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 80).map_err(|e| e.to_string())?;
        for r in 0..80 {
            xy.x_cells[r][0] = Cell::number(r as f64);
        }
        let xy_ds = DataSheet::new("xy-big", "XY Big", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;
        let mut doc = ProjectDocument::new("XY Parent Test").map_err(|e| e.to_string())?;
        doc.add_data_sheet(xy_ds).map_err(|e| e.to_string())?;
        doc.select_in_navigator(&SheetId::from("xy-big")).map_err(|e| e.to_string())?;
        let mut xy_driver = HumanPathDriver::new(UiApp::from_session(Session::from_document(doc)), ViewportMetrics::new(1280.0, 800.0, 1.0));
        xy_driver.app_mut().interaction.scroll_offsets.offsets.insert(String::from("grid.sheet.xy-big"), [0.0, 318.0]);
        let xy_tree = xy_driver.layout_tree();
        let xcell = xy_tree.node("grid.xcell.14.0").ok_or_else(|| String::from("missing grid.xcell.14.0"))?;
        assert_eq!(xcell.parent.as_ref(), Some(&WidgetId::from("grid.sheet.xy-big")));
        let [xcx, xcy] = xcell.bounds.visible_rect_px.center();
        let xy_geom = xy_tree.grid_geometry.as_ref().ok_or_else(|| String::from("missing xy grid_geometry"))?;
        assert_eq!(xy_geom.hit_test_x_cell(xcx as u32, xcy as u32), Some((14, 0)));
        Ok(())
    }
}
