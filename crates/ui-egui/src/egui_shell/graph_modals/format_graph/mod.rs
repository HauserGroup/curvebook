//! 3-Schema `Format Graph` modal dispatcher and shared legend controls (`gui-layout-spec.md`
//! §8.11C).

mod heatmap;
pub(super) mod mv;
pub(super) mod series;

use egui::Ui;

use crate::dialogs_canvas::FormatGraphState;
use crate::egui_shell::UiIntent;
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

    let mut draft = gctx.app.canvas_dialogs.format_graph.clone();
    let mut dirty = false;

    match (schema, tab) {
        (FormatGraphSchema::SeriesBased, FormatGraphTab::Appearance) => render_fg_series_appearance(ui, state, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::SeriesAndArrangement) => {
            render_fg_series_arrangement(ui, state, gctx, &mut draft, &mut dirty);
        }
        (FormatGraphSchema::SeriesBased, FormatGraphTab::GraphSettings) => render_fg_settings(ui, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::ValueAnnotations) => render_fg_annotations(ui, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::AxisVariables) => render_fg_mv_axis_variables(ui, state, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::DataObjects) => render_fg_mv_data_objects(ui, state, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::Legends) => render_fg_legends(ui, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::Heatmap, FormatGraphTab::ColorMapping) => render_fg_heatmap_color_mapping(ui, state, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapLabels) => render_fg_heatmap_labels(ui, state, gctx, &mut draft, &mut dirty),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapBorders) => render_fg_heatmap_borders(ui, state, gctx, &mut draft, &mut dirty),
        _ => render_fg_series_appearance(ui, state, gctx, &mut draft, &mut dirty),
    }

    if dirty {
        gctx.intents.push(UiIntent::UpdateFormatGraphState(Box::new(draft)));
    }
}

pub(super) fn render_legend_toggles(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut FormatGraphState, dirty: &mut bool) {
    let mut show_legend = draft.show_legend;
    let vis_lbl = tr(gctx.locale, "fg.legend");
    let vis_chk = ui.checkbox(&mut show_legend, vis_lbl);
    record_checkbox(ui, &vis_chk, gctx.meta, "format_graph.show_legend", vis_lbl, show_legend);
    if vis_chk.changed() {
        draft.show_legend = show_legend;
        *dirty = true;
    }

    let mut bordered = draft.legend_bordered;
    let brd_lbl = "Bordered Legend Box";
    let brd_chk = ui.checkbox(&mut bordered, brd_lbl);
    record_checkbox(ui, &brd_chk, gctx.meta, "format_graph.legend_bordered", brd_lbl, bordered);
    if brd_chk.changed() {
        draft.legend_bordered = bordered;
        *dirty = true;
    }
}
