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

use std::collections::BTreeMap;

use curvebook_doc::{CellCoord, DataSheet, LinkedDataRegion, SheetCellRange, SheetId, TableArchetype};
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::{GridClipboardTarget, MoveDirection, data_set_subcolumn_count, format_cell_display};
use crate::i18n::{LocalePreset, normalize_locale, try_tr};
use crate::interaction::{KeyModifiers, TextFieldEditState, UiEventOutcome};

const DEFAULT_ROW_TITLES_WIDTH_PX: u32 = 112;
const COLLAPSED_ROW_TITLES_WIDTH_PX: u32 = 34;
const MIN_ROW_TITLES_WIDTH_PX: u32 = 56;
const MAX_ROW_TITLES_WIDTH_PX: u32 = 360;
const DEFAULT_X_COL_WIDTH_PX: u32 = 96;
const DEFAULT_DATA_COL_WIDTH_PX: u32 = 96;
const MIN_COL_WIDTH_PX: u32 = 44;
const MAX_COL_WIDTH_PX: u32 = 420;

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
            ClearSelectionPlan::LinkBlocked => {
                outcome.action = String::from("grid_core.link_read_only_prompt");
                state.last_nav_summary = Some(String::from("edit:clear"));
            }
            ClearSelectionPlan::NothingToClear => {
                outcome.action = String::from("grid_core.clear_selection_empty");
                state.last_nav_summary = Some(String::from("edit:clear"));
            }
            ClearSelectionPlan::Execute { requests } => {
                let mut last_out = None;
                for req in requests {
                    last_out = Some(app.execute_command("table.clearCells", req).map_err(|e| e.to_string())?);
                }
                outcome.action = String::from("grid_core.clear_selection");
                if let Some(out) = last_out {
                    outcome.command_id = Some(out.command_id);
                    outcome.command_result = Some(out.payload);
                }
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
                    app.grid.extend_shift_range(ds, cur);
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
    Execute { requests: Vec<serde_json::Value> },
}

fn first_overlapping_linked_cell(ds: &DataSheet, range: SheetCellRange) -> Option<(usize, usize, usize, &LinkedDataRegion)> {
    let r_lo = range.start.row.min(range.end.row);
    let r_hi = range.start.row.max(range.end.row);
    let cs_lo = (range.start.col, range.start.subcol).min((range.end.col, range.end.subcol));
    let cs_hi = (range.start.col, range.start.subcol).max((range.end.col, range.end.subcol));
    for region in &ds.linked_regions {
        let rr_lo = region.target_range.start.row.min(region.target_range.end.row);
        let rr_hi = region.target_range.start.row.max(region.target_range.end.row);
        let rcs_lo =
            (region.target_range.start.col, region.target_range.start.subcol).min((region.target_range.end.col, region.target_range.end.subcol));
        let rcs_hi =
            (region.target_range.start.col, region.target_range.start.subcol).max((region.target_range.end.col, region.target_range.end.subcol));
        if r_lo <= rr_hi && rr_lo <= r_hi && cs_lo <= rcs_hi && rcs_lo <= cs_hi {
            let hit_row = r_lo.max(rr_lo);
            let hit_cs = cs_lo.max(rcs_lo);
            return Some((hit_row, hit_cs.0, hit_cs.1, region));
        }
    }
    None
}

fn prepare_clear_selected_cells(ds: &DataSheet, grid: &crate::grid_view::GridViewState, state: &mut GridCoreViewState) -> ClearSelectionPlan {
    if ds.frozen {
        return ClearSelectionPlan::Frozen;
    }
    let target = match grid.clipboard_target(ds) {
        GridClipboardTarget::WholeSheetFallback => match grid.tool.active {
            Some(c) if c.row < ds.archetype.row_count() && c.col < ds.archetype.column_count() => {
                GridClipboardTarget::Cells(vec![CellCoord::new(c.row, c.col, c.subcol)])
            }
            _ => return ClearSelectionPlan::NothingToClear,
        },
        other => other,
    };
    match &target {
        GridClipboardTarget::Range(range) => {
            if let Some((hit_row, hit_col, hit_subcol, region)) = first_overlapping_linked_cell(ds, *range) {
                let sheet_id = ds.id.to_string();
                state.active_link_popover = Some(GridLinkPopoverState { sheet_id: sheet_id.clone(), region_id: region.id.clone() });
                state.link_edit_warning =
                    Some(GridLinkBlockPrompt { sheet_id, region_id: region.id.clone(), row: hit_row, col: hit_col, subcol: hit_subcol });
                return ClearSelectionPlan::LinkBlocked;
            }
        }
        GridClipboardTarget::Cells(coords) => {
            for coord in coords {
                if check_edit_blocked_by_link(ds, state, coord.row, coord.col, coord.subcol) {
                    return ClearSelectionPlan::LinkBlocked;
                }
            }
        }
        GridClipboardTarget::WholeSheetFallback => return ClearSelectionPlan::NothingToClear,
    }
    let requests = target.clear_cells_requests(ds);
    if requests.is_empty() {
        return ClearSelectionPlan::NothingToClear;
    }
    ClearSelectionPlan::Execute { requests }
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
        let out = app
            .execute_command(
                "table.clearCells",
                json!({
                    "sheet_id": sid,
                    "cells": [],
                    "unlink": true,
                    "region_id": region_id,
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
