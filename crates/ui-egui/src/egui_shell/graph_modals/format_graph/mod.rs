//! 3-Schema `Format Graph` modal dispatcher and shared legend controls (`gui-layout-spec.md`
//! §8.11C).

mod heatmap;
mod mv;
mod series;

use egui::Ui;

use crate::i18n::tr;

use super::helpers::{GraphModalCtx, record_checkbox};
use super::state::{FormatGraphSchema, FormatGraphTab, GraphModalEguiState};

use heatmap::{render_fg_heatmap_borders, render_fg_heatmap_color_mapping, render_fg_heatmap_labels};
use mv::{render_fg_legends, render_fg_mv_axis_variables, render_fg_mv_data_objects};
use series::{render_fg_annotations, render_fg_series_appearance, render_fg_series_arrangement, render_fg_settings};

pub(super) fn render_format_graph_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let schema = FormatGraphSchema::detect(gctx.app);
    let tab = schema.clamp_tab(state.format_graph_tab);
    state.format_graph_tab = tab;

    match (schema, tab) {
        (FormatGraphSchema::SeriesBased, FormatGraphTab::Appearance) => render_fg_series_appearance(ui, state, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::SeriesAndArrangement) => render_fg_series_arrangement(ui, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::GraphSettings) => render_fg_settings(ui, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::ValueAnnotations) => render_fg_annotations(ui, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::AxisVariables) => render_fg_mv_axis_variables(ui, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::DataObjects) => render_fg_mv_data_objects(ui, state, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::Legends) => render_fg_legends(ui, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::ColorMapping) => render_fg_heatmap_color_mapping(ui, state, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapLabels) => render_fg_heatmap_labels(ui, state, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapBorders) => render_fg_heatmap_borders(ui, state, gctx),
        _ => render_fg_series_appearance(ui, state, gctx),
    }
}

fn render_legend_toggles(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, fg: &crate::dialogs_canvas::FormatGraphState) {
    let mut show_legend = fg.show_legend;
    let vis_lbl = tr(gctx.locale, "fg.legend");
    let vis_chk = ui.checkbox(&mut show_legend, vis_lbl);
    record_checkbox(ui, &vis_chk, gctx.meta, "format_graph.show_legend", vis_lbl, show_legend);
    if vis_chk.changed() {
        let mut next = fg.clone();
        next.show_legend = show_legend;
        gctx.intents.push_update_format_graph_state(next);
    }

    let mut bordered = fg.legend_bordered;
    let brd_lbl = "Bordered Legend Box";
    let brd_chk = ui.checkbox(&mut bordered, brd_lbl);
    record_checkbox(ui, &brd_chk, gctx.meta, "format_graph.legend_bordered", brd_lbl, bordered);
    if brd_chk.changed() {
        let mut next = fg.clone();
        next.legend_bordered = bordered;
        gctx.intents.push_update_format_graph_state(next);
    }
}
