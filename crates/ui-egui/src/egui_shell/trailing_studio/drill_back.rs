//! Data Point Drill-Back studio surface.

use curvebook_doc::GraphSheet;
use curvebook_tools::GridCellCoord;
use egui::Stroke;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, widgets};
use crate::i18n::tr;

use super::helpers::{StudioButtonSpec, studio_full_button};
use super::{DrillBackPointEntry, TrailingStudioContext, TrailingStudioEguiState};

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
            if currently_excluded {
                ui.label(egui::RichText::new("Status: Excluded from analyses").size(small).strong().color(tokens.link_red));
            }
        });

    ui.add_space(6.0);
    let ds_frozen = src_ds.is_some_and(|d| d.frozen);
    let exclude_lbl = if currently_excluded { "Include Point" } else { "Exclude Point" };
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: exclude_lbl,
            tooltip: "Toggle exclusion flag for this datum cell (table.excludeCells)",
            selected: currently_excluded,
            enabled: ctx_in.enabled && !ds_frozen,
            primary: false,
            widget_id: "inspector.drill_back.toggle_exclude",
            command_id: Some("table.excludeCells"),
            disabled_reason: ds_frozen.then_some("source sheet is frozen"),
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

    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Format Selected Point…",
            tooltip: "Customize individual point color, shape, and size",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.drill_back.format_point",
            command_id: Some("ui.formatPoints.open"),
            disabled_reason: None,
        },
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
        meta,
        &StudioButtonSpec {
            label: tr(tokens.locale, "inspector.drill_back.go"),
            tooltip: "Navigate to source data table and select exact cell",
            selected: false,
            enabled: ctx_in.enabled,
            primary: true,
            widget_id: "inspector.drill_back.jump",
            command_id: Some("sheet.select"),
            disabled_reason: None,
        },
    ) {
        intents.push_engine("sheet.select", json!({ "sheet_id": brushed.sheet_id.as_str() }));
        intents.push_select_grid_cell(GridCellCoord::new(brushed.row, brushed.col, brushed.subcol));
    }
}
