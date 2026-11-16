//! Core virtualized data-grid view builder (`R10.8`, `R10.22`).

use curvebook_doc::{DataSheet, TableArchetype};
use curvebook_palette::Rgba8;
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::data_column_title;
use crate::grid_view::resolve_flat_coord;
use crate::interaction::{GridHitGeometry, UiEventOutcome};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::widgets::{FittedTextSpec, paint_text_fitted, widget_state};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::{grid_cell_meta, labeled_meta};
use super::mv_formula::{build_formula_bar_row, emit_mv_column_fx_button};

/// Transient view state owned by the core data-grid shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridCoreViewState {
    pub row_titles_collapsed: bool,
    pub row_titles_width_px: u32,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `grid_core` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut GridCoreViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the virtualized data table grid view for `ds` inside `rect`.
pub fn build_data_grid_view(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, rect: LayoutRectPx) {
    let sheet_rect = LayoutRectPx::new(rect.x + 6, rect.y + 6, rect.width.saturating_sub(12), rect.height.saturating_sub(12));
    if sheet_rect.width < 40 || sheet_rect.height < 40 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, sheet_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, sheet_rect, builder.palette.border, 1);
    builder.push_clip(sheet_rect);

    let show_formula = matches!(ds.archetype, TableArchetype::MultipleVariables(_)) || builder.app.grid.formula_bar.active;
    let formula_h = build_formula_bar_row(builder, ds, sheet_rect, show_formula);

    let frame = builder.app.grid.tool.format_visible_window(ds);
    let hdr_y = sheet_rect.y + 4 + i32::try_from(formula_h).unwrap_or(0);
    let row_h = 20u32;
    let body_y = hdr_y + i32::try_from(row_h + 2).unwrap_or(22);
    let gutter_w = 38u32;
    let (x_subcols, x_col_w) = match &ds.archetype {
        TableArchetype::Xy(xy) => (xy.x_mode.subcolumn_count().max(1), 64u32),
        TableArchetype::Survival(_) => (1, 64u32),
        _ => (0, 0),
    };
    let x_total_w = u32::try_from(x_subcols).unwrap_or(0).saturating_mul(x_col_w);
    let avail_cols_w = sheet_rect.width.saturating_sub(gutter_w + x_total_w + 14);
    let total_frame_cols = frame.window.end_col.saturating_sub(frame.window.start_col).max(1) as u32;
    let col_w = if total_frame_cols > 6 { (avail_cols_w / total_frame_cols).clamp(52, 76) } else { 76u32 };
    let max_cols = (avail_cols_w / col_w).max(1) as usize;
    let avail_body_h = u32::try_from((sheet_rect.bottom() - body_y - 12).max(0)).unwrap_or(0);
    let max_rows = (avail_body_h / row_h) as usize;
    let visible_rows = max_rows.min(frame.window.end_row.saturating_sub(frame.window.start_row));
    let end_row = frame.window.start_row.saturating_add(visible_rows);

    let mut visible_cols = Vec::new();
    for flat_col in (frame.window.start_col..frame.window.end_col).take(max_cols) {
        let (col, subcol) = resolve_flat_coord(&ds.archetype, flat_col).unwrap_or((flat_col, 0));
        visible_cols.push((flat_col, GridCellCoord::new(0, col, subcol)));
    }

    let geom = GridHitGeometry {
        sheet_id: ds.id.clone(),
        sheet_rect: sheet_rect.to_workspace_rect(),
        hdr_y: u32::try_from(hdr_y.max(0)).unwrap_or(0),
        body_y: u32::try_from(body_y.max(0)).unwrap_or(0),
        row_h,
        gutter_w,
        col_w,
        x_subcols,
        x_col_w,
        start_row: frame.window.start_row,
        end_row,
        start_col: frame.window.start_col,
        visible_cols: visible_cols.clone(),
        is_rtl: builder.tree.is_rtl,
    };

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let header_style = UiTextStyle::ui_bold(size_px);
    let cell_style = UiTextStyle::grid_tabular(size_px);

    // X/Time headers (if XY or Survival)
    for subcol in 0..x_subcols {
        if let Some(xr) = geom.x_header_rect(subcol) {
            let x_rect = LayoutRectPx::from_workspace_rect(xr);
            let x_lbl = if x_subcols == 1 { String::from("X") } else { format!("X{}", subcol + 1) };
            let x_wid = builder.push_node(NodeSpec {
                id: WidgetId::from(format!("grid.x_hdr.{subcol}")),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: x_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, x_lbl.clone(), FocusScope::Workspace),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            builder.paint_fill(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.header);
            builder.paint_stroke(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.border, 1);
            paint_text_fitted(
                builder,
                &x_wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: x_rect,
                    pad_x: 4,
                    text: &x_lbl,
                    style: &header_style,
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
        }
    }

    // Y/Data Column headers
    let is_mv = matches!(&ds.archetype, TableArchetype::MultipleVariables(_));
    for (vis_idx, &(flat_col, coord)) in visible_cols.iter().enumerate() {
        let h_rect = LayoutRectPx::from_workspace_rect(geom.column_header_rect(vis_idx as u32));
        let title = data_column_title(ds, coord.col).map_or_else(|| format!("Col {}", coord.col + 1), str::to_owned);
        let fx_w = if is_mv && coord.subcol == 0 && h_rect.width >= 38 { 18u32 } else { 0 };
        let lbl_rect = LayoutRectPx::new(h_rect.x, h_rect.y, h_rect.width.saturating_sub(fx_w), h_rect.height);
        let h_id = format!("grid.col_hdr.{flat_col}");
        let h_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(h_id),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: lbl_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, title.clone(), FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.paint_fill(ZLayer::Workspace, Some(&h_wid), lbl_rect, builder.palette.header);
        builder.paint_stroke(ZLayer::Workspace, Some(&h_wid), lbl_rect, builder.palette.border, 1);
        paint_text_fitted(
            builder,
            &h_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: lbl_rect,
                pad_x: 4,
                text: &title,
                style: &header_style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        emit_mv_column_fx_button(builder, ds, coord.col, lbl_rect, h_rect, fx_w);
    }

    // Virtualized visible cells only (`<= 16.67 ms` on 1M-cell grid!)
    for (r_idx, row_abs) in (frame.window.start_row..end_row).enumerate() {
        let g_rect = LayoutRectPx::from_workspace_rect(geom.row_gutter_rect(r_idx as u32));
        let row_label = format!("{}", row_abs + 1);
        builder.paint_fill(ZLayer::Workspace, None, g_rect, builder.palette.panel);
        let g_wid = WidgetId::from(format!("grid.gutter.{row_abs}"));
        let mut g_a11y = AccessibilityMeta::new(WidgetRole::Label, row_label.clone(), FocusScope::Workspace);
        g_a11y.value_text = Some(row_label.clone());
        builder.push_node(NodeSpec {
            id: g_wid.clone(),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: g_rect,
            a11y: g_a11y,
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        paint_text_fitted(
            builder,
            &g_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: g_rect,
                pad_x: 3,
                text: &row_label,
                style: &cell_style,
                color: builder.palette.muted,
                scroll_container: None,
            },
        );

        for subcol in 0..x_subcols {
            if let Some(xr) = geom.x_cell_rect(row_abs, subcol) {
                let x_rect = LayoutRectPx::from_workspace_rect(xr);
                let x_text = x_cell_display(ds, row_abs, subcol, &builder.app.locale_preset);
                let x_id = format!("grid.xcell.{row_abs}.{subcol}");
                let x_wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(x_id),
                    parent: None,
                    z_layer: ZLayer::Workspace,
                    rect_px: x_rect,
                    a11y: labeled_meta(WidgetRole::GridCell, format!("R{}X{}", row_abs + 1, subcol + 1), FocusScope::Workspace, "ArrowKeys"),
                    state: WidgetVisualState::default(),
                    action: WidgetAction::SelectGridCell { row: row_abs as u32, col: u32::MAX, subcol: subcol as u16 },
                });
                builder.paint_fill(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.surface);
                builder.paint_stroke(ZLayer::Workspace, Some(&x_wid), x_rect, builder.palette.border, 1);
                if !x_text.is_empty() {
                    paint_text_fitted(
                        builder,
                        &x_wid,
                        FittedTextSpec {
                            z_layer: ZLayer::Workspace,
                            rect: x_rect,
                            pad_x: 3,
                            text: &x_text,
                            style: &cell_style,
                            color: builder.palette.text,
                            scroll_container: None,
                        },
                    );
                }
            }
        }

        for &(flat_col, base_coord) in &visible_cols {
            let Some(cr) = geom.cell_rect(row_abs, flat_col) else {
                continue;
            };
            let cell_rect = LayoutRectPx::from_workspace_rect(cr);
            let coord = GridCellCoord::new(row_abs, base_coord.col, base_coord.subcol);
            let cell_view = frame.cells.iter().find(|c| c.coord.row == row_abs && c.flat_col == flat_col);
            let text = cell_view.map(|c| c.display_text.as_str()).unwrap_or("");
            let excluded = cell_view.is_some_and(|c| c.excluded);
            let selected = cell_view.is_some_and(|c| c.is_selected);

            let cid = format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol);
            let a11y = grid_cell_meta(format!("R{}C{}", coord.row + 1, coord.col + 1), selected, (!text.is_empty()).then_some(text));

            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(cid.clone()),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: cell_rect,
                a11y,
                state: widget_state(builder, &cid, ControlEnablement::enabled()),
                action: WidgetAction::SelectGridCell { row: coord.row as u32, col: coord.col as u32, subcol: coord.subcol as u16 },
            });
            let bg = if selected { Rgba8::rgba(59, 130, 246, 48) } else { builder.palette.surface };
            builder.paint_fill(ZLayer::Workspace, Some(&wid), cell_rect, bg);
            builder.paint_stroke(ZLayer::Workspace, Some(&wid), cell_rect, if selected { builder.palette.accent } else { builder.palette.border }, 1);
            if !text.is_empty() {
                let ink = if excluded { builder.palette.excluded } else { builder.palette.text };
                let style = if excluded { cell_style.clone().italic() } else { cell_style.clone() };
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec { z_layer: ZLayer::Workspace, rect: cell_rect, pad_x: 4, text, style: &style, color: ink, scroll_container: None },
                );
                if excluded {
                    let mid_y = cell_rect.center()[1];
                    builder.paint_line(ZLayer::Workspace, Some(&wid), [cell_rect.x + 3, mid_y], [cell_rect.right() - 3, mid_y], ink, 1);
                }
            }
        }
    }

    builder.tree.grid_geometry = Some(geom);
    builder.pop_clip();
}

/// Format an X/time cell value according to `preset`.
#[must_use]
pub fn x_cell_display(ds: &DataSheet, row: usize, subcol: usize, preset: &crate::i18n::LocalePreset) -> String {
    ds.x_or_time_cell(row, subcol).map(|c| crate::grid_view::format_cell_display(c, preset)).unwrap_or_default()
}
