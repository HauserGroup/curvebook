//! Data Point Drill-Back studio surface.

use curvebook_doc::GraphSheet;
use curvebook_tools::GridCellCoord;
use egui::Stroke;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, UiIntent, widgets};
use crate::i18n::tr;

use super::helpers::{StudioButtonSpec, studio_full_button};
use super::{DrillBackPointEntry, TrailingStudioContext, TrailingStudioEguiState};

fn ensure_drill_points_cache<'a>(
    ctx: &egui::Context,
    app: &UiApp,
    gs: &GraphSheet,
    state: &'a mut TrailingStudioEguiState,
) -> &'a [DrillBackPointEntry] {
    let seq = app.mutation_seq();
    let hit = state.drill_cache_key.as_ref().is_some_and(|(sid, s)| sid == &gs.id && *s == seq);
    if !hit {
        state.drill_cache_key = Some((gs.id.clone(), seq));
        state.drill_cached_points = crate::egui_shell::canvas_view::resolve_cached_scene(ctx, app, &gs.id, "canvas")
            .map(|scene| {
                scene
                    .plotted_points
                    .iter()
                    .map(|p| DrillBackPointEntry {
                        source_sheet_id: p.source_cell.sheet_id.clone(),
                        row: p.source_cell.row,
                        col: p.source_cell.col,
                        subcol: p.source_cell.subcol,
                        series_label: p.series_label.clone(),
                        value_summary: p.value_summary.clone(),
                        x_pt_tenths: (p.center_pt.x * 10.0).round() as i64,
                        y_pt_tenths: (p.center_pt.y * 10.0).round() as i64,
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    &state.drill_cached_points
}

pub(super) fn render_data_point_drill_back_studio(
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
        meta,
        &StudioButtonSpec {
            label: "← Back to Graph Inspector",
            tooltip: "Clear brushed point selection",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.drill_back.back",
            command_id: None,
            disabled_reason: None,
        },
    ) {
        intents.push(UiIntent::ClearBrush);
    }

    let Some(brushed) = app.brushed_cell.clone() else {
        widgets::dim_label(ui, "Click any plotted point to inspect its source cell.");
        return;
    };

    let points = ensure_drill_points_cache(ui.ctx(), app, gs, state);
    let matched_pt =
        points.iter().find(|p| p.source_sheet_id == brushed.sheet_id && p.row == brushed.row && p.col == brushed.col && p.subcol == brushed.subcol);
    let brushed_on_graph = matched_pt.is_some()
        || gs.spec.primary_source_sheet_id == brushed.sheet_id
        || gs.spec.series.iter().any(|s| s.source_sheet_id == brushed.sheet_id);

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
    egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, tokens.divider))
        .corner_radius(f32::from(tokens.metrics.radius_md))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            let small = tokens.metrics.font_ui_small_pt;
            ui.label(egui::RichText::new(src_title).size(12.0).strong().color(tokens.text_strong));
            ui.label(
                egui::RichText::new(format!("Row {}  •  Col {}  •  Sub {}", brushed.row + 1, brushed.col + 1, brushed.subcol + 1))
                    .size(small)
                    .color(tokens.text_primary),
            );
            ui.label(egui::RichText::new(format!("Value: {raw_val}")).size(small).color(tokens.text_primary));
            if let Some(pt) = matched_pt {
                ui.label(egui::RichText::new(format!("Series: {}", pt.series_label)).size(small).color(tokens.text_muted));
                ui.label(egui::RichText::new(pt.value_summary.clone()).size(small).color(tokens.text_primary));
                ui.label(
                    egui::RichText::new(format!("Canvas: ({:.1}, {:.1}) pt", (pt.x_pt_tenths as f64) * 0.1, (pt.y_pt_tenths as f64) * 0.1))
                        .size(small)
                        .color(tokens.text_muted),
                );
            }
            if !brushed_on_graph {
                ui.label(egui::RichText::new("Not plotted on this graph").size(small).color(tokens.text_muted));
            }
            if currently_excluded {
                ui.label(egui::RichText::new("Status: Excluded from analyses").size(small).strong().color(tokens.link_red));
            }
        });

    ui.add_space(6.0);
    let src_exists = src_ds.is_some();
    let ds_frozen = src_ds.is_some_and(|d| d.frozen);
    let exclude_lbl = if currently_excluded { "Include Point" } else { "Exclude Point" };
    let exclude_enabled = ctx_in.enabled && src_exists && !ds_frozen;
    let exclude_disabled_reason = if !src_exists {
        Some("source sheet no longer exists")
    } else if ds_frozen {
        Some("source sheet is frozen")
    } else {
        None
    };
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: exclude_lbl,
            tooltip: "Toggle exclusion flag for this datum cell (table.excludeCells)",
            selected: currently_excluded,
            enabled: exclude_enabled,
            primary: false,
            widget_id: "inspector.drill_back.toggle_exclude",
            command_id: Some("table.excludeCells"),
            disabled_reason: exclude_disabled_reason,
        },
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

    let resolved_series_idx = brushed_on_graph
        .then(|| {
            gs.spec.series.iter().position(|s| s.source_sheet_id == brushed.sheet_id && s.data_set_index == brushed.col).or_else(|| {
                gs.spec.series.iter().position(|s| {
                    s.data_set_index == brushed.col
                        && doc.result_sheet(&s.source_sheet_id).is_some_and(|rs| rs.analysis.source_sheet_id == brushed.sheet_id)
                })
            })
        })
        .flatten();
    let format_enabled = ctx_in.enabled && resolved_series_idx.is_some();
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Format Selected Point",
            tooltip: "Customize individual point color, shape, and size",
            selected: false,
            enabled: format_enabled,
            primary: false,
            widget_id: "inspector.drill_back.format_point",
            command_id: Some("ui.formatPoints.open"),
            disabled_reason: (ctx_in.enabled && resolved_series_idx.is_none()).then_some("brushed cell is not a plotted series on this graph"),
        },
    ) && let Some(series_idx) = resolved_series_idx
    {
        let subcol_count = src_ds.map_or(1, |d| crate::grid_view::data_set_subcolumn_count(&d.archetype, brushed.col));
        let mut pt_obj = json!({
            "series_index": series_idx,
            "point_index": brushed.row,
        });
        if subcol_count > 1
            && let Some(map) = pt_obj.as_object_mut()
        {
            map.insert(String::from("subcol"), json!(brushed.subcol));
        }
        intents.push_ui(
            "ui.formatPoints.open",
            json!({
                "sheet_id": gs.id.as_str(),
                "points": [pt_obj],
            }),
        );
    }

    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: tr(tokens.locale, "inspector.drill_back.go"),
            tooltip: "Navigate to source data table and select exact cell",
            selected: false,
            enabled: ctx_in.enabled && src_exists,
            primary: true,
            widget_id: "inspector.drill_back.jump",
            command_id: Some("sheet.select"),
            disabled_reason: (!src_exists).then_some("source sheet no longer exists"),
        },
    ) {
        intents.push_engine("sheet.select", json!({ "sheet_id": brushed.sheet_id.as_str() }));
        intents.push(UiIntent::SelectGridCell {
            sheet_id: Some(brushed.sheet_id.clone()),
            coord: GridCellCoord::new(brushed.row, brushed.col, brushed.subcol),
        });
    }
}
