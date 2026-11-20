//! Graph/layout canvas view, rulers, command palette, and descriptor-driven modal helpers (`R10.3`, `R10.6`, `R10.15`).

use curvebook_chart::PointPt;
use curvebook_doc::{ErrorBarMetric, GraphKind, JournalWidthPreset, LayoutItem, PageOrientation, PanelBadgeStyle, RectPt, SheetId, SummaryMeasure};
use curvebook_render::TiffCompression;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{ExportFigureFormat, GRAPH_TYPE_OPTIONS};
use crate::dialogs_r9::RandomizationSchemeKind;
use crate::interaction::UiEventOutcome;

const fn default_true() -> bool {
    true
}

fn default_export_color_space() -> String {
    String::from("sRGB")
}

fn default_export_unit() -> String {
    String::from("px")
}

/// Transient view state owned by the graph / layout / miscellaneous dialogs shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogsGraphMiscViewState {
    #[serde(default)]
    pub active_section: String,
    #[serde(default)]
    pub zoom_menu_slot: Option<String>,
    #[serde(default = "default_true")]
    pub hook_expand_info: bool,
    #[serde(default = "default_true")]
    pub hook_expand_analysis: bool,
    #[serde(default = "default_true")]
    pub hook_expand_document: bool,
    #[serde(default)]
    pub graph_type_show_all: bool,
    #[serde(default)]
    pub export_transparent_bg: bool,
    #[serde(default = "default_export_color_space")]
    pub export_color_space: String,
    #[serde(default = "default_export_unit")]
    pub export_unit: String,
    #[serde(default)]
    pub sim_selected_col_idx: usize,
    #[serde(default)]
    pub layout_revision: u64,
}

impl Default for DialogsGraphMiscViewState {
    fn default() -> Self {
        Self {
            active_section: String::new(),
            zoom_menu_slot: None,
            hook_expand_info: true,
            hook_expand_analysis: true,
            hook_expand_document: true,
            graph_type_show_all: false,
            export_transparent_bg: false,
            export_color_space: default_export_color_space(),
            export_unit: default_export_unit(),
            sim_selected_col_idx: 0,
            layout_revision: 0,
        }
    }
}

fn handled_outcome(mutated: bool) -> Result<UiEventOutcome, String> {
    let act = if mutated { "dialogs_graph_misc.updated" } else { "dialogs_graph_misc.ui" };
    Ok(UiEventOutcome { handled: true, action: String::from(act), ..UiEventOutcome::default() })
}

fn buffer_text(app: &UiApp, widget_id: &str) -> Option<String> {
    app.interaction.text_field_buffers.get(widget_id).map(|b| b.text.clone())
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `dialogs_graph_misc` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let res = if let Some(res) = handle_zoom_pan_action(app, state, payload) {
        res
    } else if let Some(res) = handle_palette_action(app, payload) {
        res
    } else if let Some(res) = handle_graph_hook_export_action(app, state, payload) {
        res
    } else if let Some(res) = handle_remaining_dialog_action(app, state, payload) {
        res
    } else {
        Ok(UiEventOutcome::default())
    };
    if let Ok(ref out) = res
        && out.handled
    {
        // Bump `layout_revision` so view-state fingerprints invalidate cleanly after any `canvas_dialogs` mutation.
        state.layout_revision = state.layout_revision.wrapping_add(1);
    }
    res
}

fn apply_zoom_and_pan(app: &mut UiApp, zoom: Option<f64>, reset_pan: bool, pan_delta: Option<[f64; 2]>) {
    if let Some(z) = zoom {
        let clamped = z.clamp(0.25, 4.0);
        app.canvas_dialogs.canvas.zoom = clamped;
        app.canvas_dialogs.pan_zoom.zoom = clamped;
    }
    if reset_pan {
        app.canvas_dialogs.canvas.pan_pt = PointPt::new(0.0, 0.0);
        app.canvas_dialogs.pan_zoom.pan_pt = [0.0, 0.0];
    } else if let Some([dx, dy]) = pan_delta {
        let nx = (app.canvas_dialogs.canvas.pan_pt.x + dx).clamp(-1200.0, 1200.0);
        let ny = (app.canvas_dialogs.canvas.pan_pt.y + dy).clamp(-1200.0, 1200.0);
        app.canvas_dialogs.canvas.pan_pt = PointPt::new(nx, ny);
        app.canvas_dialogs.pan_zoom.pan_pt = [nx, ny];
    }
}

fn handle_zoom_pan_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(slot) = payload.strip_prefix("zoom:toggle_menu:") {
        state.zoom_menu_slot = if state.zoom_menu_slot.as_deref() == Some(slot) { None } else { Some(slot.to_owned()) };
        return Some(handled_outcome(false));
    }
    if let Some(rest) = payload.strip_prefix("zoom:set:")
        && let Some((_, preset)) = rest.split_once(':')
    {
        state.zoom_menu_slot = None;
        if preset == "fit" {
            apply_zoom_and_pan(app, Some(1.0), true, None);
            return Some(handled_outcome(true));
        }
        if let Ok(pct) = preset.parse::<f64>() {
            apply_zoom_and_pan(app, Some(pct / 100.0), false, None);
            return Some(handled_outcome(true));
        }
    }
    if let Some(rest) = payload.strip_prefix("zoom:step:")
        && let Some((_, dir)) = rest.split_once(':')
    {
        state.zoom_menu_slot = None;
        let delta = if dir == "in" { 0.25 } else { -0.25 };
        let next = app.canvas_dialogs.canvas.zoom + delta;
        apply_zoom_and_pan(app, Some(next), false, None);
        return Some(handled_outcome(true));
    }
    if let Some(rest) = payload.strip_prefix("pan:reset:") {
        let _ = rest;
        apply_zoom_and_pan(app, None, true, None);
        return Some(handled_outcome(true));
    }
    if let Some(rest) = payload.strip_prefix("pan:nudge:") {
        let mut parts = rest.split(':');
        let _slot = parts.next();
        if let (Some(dx_s), Some(dy_s)) = (parts.next(), parts.next())
            && let (Ok(dx), Ok(dy)) = (dx_s.parse::<f64>(), dy_s.parse::<f64>())
        {
            apply_zoom_and_pan(app, None, false, Some([dx, dy]));
            return Some(handled_outcome(true));
        }
    }
    if payload.starts_with("pan:scroll_h:") {
        let delta = if payload.ends_with(":dec") { -48.0 } else { 48.0 };
        apply_zoom_and_pan(app, None, false, Some([delta, 0.0]));
        return Some(handled_outcome(true));
    }
    if payload.starts_with("pan:scroll_v:") {
        let delta = if payload.ends_with(":dec") { -48.0 } else { 48.0 };
        apply_zoom_and_pan(app, None, false, Some([0.0, delta]));
        return Some(handled_outcome(true));
    }
    if let Some(panel_id) = payload.strip_prefix("chrome:select_panel:") {
        let next = if app.canvas_dialogs.canvas.selected_object_id.as_deref() == Some(panel_id) { None } else { Some(panel_id.to_owned()) };
        app.canvas_dialogs.canvas.selected_object_id = next;
        return Some(handled_outcome(true));
    }
    if let Some(s_idx) = payload.strip_prefix("chrome:select_series:") {
        let key = format!("series.{s_idx}");
        let next = if app.canvas_dialogs.canvas.selected_object_id.as_deref() == Some(key.as_str()) { None } else { Some(key) };
        app.canvas_dialogs.canvas.selected_object_id = next;
        return Some(handled_outcome(true));
    }
    None
}

fn handle_palette_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    let matches = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    match payload {
        "palette:prev" => {
            if !matches.is_empty() {
                let idx = app.canvas_dialogs.command_palette.selected_index.min(matches.len() - 1);
                app.canvas_dialogs.command_palette.selected_index = if idx == 0 { matches.len() - 1 } else { idx - 1 };
            }
            Some(handled_outcome(false))
        }
        "palette:next" => {
            if !matches.is_empty() {
                let idx = app.canvas_dialogs.command_palette.selected_index;
                app.canvas_dialogs.command_palette.selected_index = (idx + 1) % matches.len();
            }
            Some(handled_outcome(false))
        }
        "palette:run" => {
            let idx = app.canvas_dialogs.command_palette.selected_index;
            let chosen = matches.get(idx).or_else(|| matches.first()).map(|e| e.id.to_owned());
            if let Some(cmd_id) = chosen {
                app.canvas_dialogs.close_dialog();
                app.focus_widget_id = None;
                return Some(
                    app.execute_command(&cmd_id, json!({}))
                        .map(|_| UiEventOutcome {
                            handled: true,
                            action: format!("palette.run:{cmd_id}"),
                            command_id: Some(cmd_id),
                            ..UiEventOutcome::default()
                        })
                        .map_err(|err| err.to_string()),
                );
            }
            Some(handled_outcome(false))
        }
        _ => payload.strip_prefix("palette:select:").and_then(|raw| raw.parse::<usize>().ok()).map(|idx| {
            if idx < matches.len() {
                app.canvas_dialogs.command_palette.selected_index = idx;
            }
            handled_outcome(false)
        }),
    }
}

fn handle_graph_hook_export_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(opt_id) = payload.strip_prefix("graph_type:select:") {
        app.canvas_dialogs.change_graph_type.selected_option_id = opt_id.to_owned();
        return Some(handled_outcome(true));
    }
    if payload == "graph_type:toggle_all" {
        state.graph_type_show_all = !state.graph_type_show_all;
        return Some(handled_outcome(false));
    }
    if let Some(summary) = payload.strip_prefix("graph_type:summary:") {
        app.canvas_dialogs.change_graph_type.summary_measure = match summary {
            "median" => SummaryMeasure::Median,
            "geomean" => SummaryMeasure::GeometricMean,
            _ => SummaryMeasure::Mean,
        };
        return Some(handled_outcome(true));
    }
    if let Some(err_metric) = payload.strip_prefix("graph_type:errorbar:") {
        app.canvas_dialogs.change_graph_type.error_bar_metric = match err_metric {
            "sem" => ErrorBarMetric::Sem,
            "ci95" => ErrorBarMetric::ConfidenceInterval95,
            "iqr" => ErrorBarMetric::InterquartileRange,
            "range" => ErrorBarMetric::Range,
            _ => ErrorBarMetric::Sd,
        };
        return Some(handled_outcome(true));
    }
    if payload == "graph_type:apply" {
        if let Some(gid) = app.canvas_dialogs.change_graph_type.graph_sheet_id.clone() {
            let opt_id = app.canvas_dialogs.change_graph_type.selected_option_id.clone();
            let graph_kind = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == opt_id).map_or(GraphKind::Xy, |o| o.graph_kind);
            let res = app.execute_command("graph.setKind", json!({ "sheet_id": gid.as_str(), "kind": graph_kind }));
            return Some(match res {
                Ok(_) => handled_outcome(true),
                Err(err) => Err(err.to_string()),
            });
        }
        return Some(handled_outcome(false));
    }
    if let Some(cat) = payload.strip_prefix("hook:toggle_cat:") {
        match cat {
            "info" => state.hook_expand_info = !state.hook_expand_info,
            "analysis" => state.hook_expand_analysis = !state.hook_expand_analysis,
            "document" => state.hook_expand_document = !state.hook_expand_document,
            _ => {}
        }
        return Some(handled_outcome(false));
    }
    if payload == "hook:clear_search" {
        app.canvas_dialogs.hook_picker.search_query.clear();
        app.interaction.text_field_buffers.remove("hook.search_input");
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("hook:select:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(txt) = buffer_text(app, "hook.search_input") {
            app.canvas_dialogs.hook_picker.search_query = txt;
        }
        if idx < app.canvas_dialogs.hook_picker.options.len() {
            app.canvas_dialogs.hook_picker.selected_index = Some(idx);
        }
        return Some(handled_outcome(true));
    }
    if let Some(fmt) = payload.strip_prefix("export:fmt:") {
        app.canvas_dialogs.export_figure.format = match fmt {
            "svg" => ExportFigureFormat::Svg,
            "pdf" => ExportFigureFormat::Pdf,
            "png" => ExportFigureFormat::Png,
            "tiff_lzw" => ExportFigureFormat::Tiff(TiffCompression::Lzw),
            "tiff_none" => ExportFigureFormat::Tiff(TiffCompression::None),
            _ => ExportFigureFormat::Tiff(TiffCompression::Deflate),
        };
        if matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf) {
            state.export_transparent_bg = false;
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_dpi) = payload.strip_prefix("export:dpi:")
        && let Ok(dpi) = raw_dpi.parse::<u16>()
    {
        app.canvas_dialogs.export_figure.dpi = dpi.clamp(72, 2400);
        return Some(handled_outcome(true));
    }
    if payload == "export:dpi_dec" || payload == "export:dpi_inc" {
        let cur = i32::from(app.canvas_dialogs.export_figure.dpi);
        let next = if payload == "export:dpi_inc" { cur + 50 } else { cur - 50 };
        app.canvas_dialogs.export_figure.dpi = u16::try_from(next.clamp(72, 1200)).unwrap_or(300);
        return Some(handled_outcome(true));
    }
    if let Some(unit) = payload.strip_prefix("export:unit:") {
        state.export_unit = match unit {
            "mm" => String::from("mm"),
            "in" => String::from("in"),
            _ => String::from("px"),
        };
        return Some(handled_outcome(false));
    }
    if let Some(dim) = payload.strip_prefix("export:size:") {
        let step: i32 = match state.export_unit.as_str() {
            "mm" => ((10.0 / 25.4) * f64::from(app.canvas_dialogs.export_figure.dpi)).round() as i32,
            "in" => (0.5 * f64::from(app.canvas_dialogs.export_figure.dpi)).round() as i32,
            _ => 120,
        }
        .max(16);
        match dim {
            "w_dec" => {
                app.canvas_dialogs.export_figure.width_px = app.canvas_dialogs.export_figure.width_px.saturating_sub(step as u32).max(160);
            }
            "w_inc" => {
                app.canvas_dialogs.export_figure.width_px = app.canvas_dialogs.export_figure.width_px.saturating_add(step as u32).min(9600);
            }
            "h_dec" => {
                app.canvas_dialogs.export_figure.height_px = app.canvas_dialogs.export_figure.height_px.saturating_sub(step as u32).max(120);
            }
            "h_inc" => {
                app.canvas_dialogs.export_figure.height_px = app.canvas_dialogs.export_figure.height_px.saturating_add(step as u32).min(9600);
            }
            _ => {}
        }
        return Some(handled_outcome(true));
    }
    if payload == "export:transparent_bg" {
        if !matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf) {
            state.export_transparent_bg = !state.export_transparent_bg;
        }
        return Some(handled_outcome(true));
    }
    if let Some(cs) = payload.strip_prefix("export:cs:") {
        state.export_color_space = match cs {
            "p3" => String::from("Display-P3"),
            "gray" => String::from("Grayscale"),
            "cmyk" => String::from("CMYK-Proof"),
            _ => String::from("sRGB"),
        };
        return Some(handled_outcome(true));
    }
    None
}

fn handle_remaining_dialog_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(res) = handle_dup_change_fmt_exclude_action(app, state, payload) {
        return Some(res);
    }
    if let Some(res) = handle_layout_eq_rand_sim_print_action(app, state, payload) {
        return Some(res);
    }
    if let Some(sec) = payload.strip_prefix("about:tab:") {
        state.active_section = sec.to_owned();
        return Some(handled_outcome(false));
    }
    None
}

fn handle_dup_change_fmt_exclude_action(
    app: &mut UiApp,
    _state: &mut DialogsGraphMiscViewState,
    payload: &str,
) -> Option<Result<UiEventOutcome, String>> {
    if let Some(raw_idx) = payload.strip_prefix("change_data:cand:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(cand) = app.canvas_dialogs.change_data.candidates.get(idx)
            && cand.compatible
        {
            app.canvas_dialogs.change_data.selected_target_id = Some(cand.sheet_id.clone());
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_ds) = payload.strip_prefix("change_data:ds:")
        && let Ok(ds_idx) = raw_ds.parse::<usize>()
    {
        let sets = &mut app.canvas_dialogs.change_data.selected_data_sets;
        if let Some(pos) = sets.iter().position(|&x| x == ds_idx) {
            sets.remove(pos);
        } else {
            sets.push(ds_idx);
            sets.sort_unstable();
        }
        return Some(handled_outcome(true));
    }
    if let Some(color) = payload.strip_prefix("fmt_pts:color:") {
        let hex = match color {
            "black" => Some("#111827"),
            "red" => Some("#DC2626"),
            "blue" => Some("#2563EB"),
            "green" => Some("#16A34A"),
            "orange" => Some("#EA580C"),
            "purple" => Some("#7C3AED"),
            _ => None,
        };
        app.canvas_dialogs.format_points.color = hex.map(str::to_owned);
        app.canvas_dialogs.format_points.fill_color = hex.map(str::to_owned);
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if let Some(sym) = payload.strip_prefix("fmt_pts:sym:") {
        app.canvas_dialogs.format_points.symbol = match sym {
            "none" => None,
            other => Some(other.to_owned()),
        };
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:size_dec" || payload == "fmt_pts:size_inc" {
        let cur = app.canvas_dialogs.format_points.size_pt.unwrap_or(5.0);
        let next = if payload == "fmt_pts:size_inc" { cur + 1.5 } else { cur - 1.5 };
        app.canvas_dialogs.format_points.size_pt = Some(next.clamp(2.0, 24.0));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:bw_dec" || payload == "fmt_pts:bw_inc" {
        let cur = app.canvas_dialogs.format_points.border_width_pt.unwrap_or(1.0);
        let next = if payload == "fmt_pts:bw_inc" { cur + 0.5 } else { cur - 0.5 };
        app.canvas_dialogs.format_points.border_width_pt = Some(next.clamp(0.5, 6.0));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:fill_mode" {
        let next = if app.canvas_dialogs.format_points.fill_mode.as_deref() == Some("open") { "filled" } else { "open" };
        app.canvas_dialogs.format_points.fill_mode = Some(String::from(next));
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:hidden" {
        let next = !app.canvas_dialogs.format_points.hidden.unwrap_or(false);
        app.canvas_dialogs.format_points.hidden = Some(next);
        app.canvas_dialogs.format_points.clear_overrides = false;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:show_row_labels" {
        app.canvas_dialogs.format_points.show_row_labels = !app.canvas_dialogs.format_points.show_row_labels;
        return Some(handled_outcome(true));
    }
    if payload == "fmt_pts:clear_overrides" {
        app.canvas_dialogs.format_points.clear_overrides = !app.canvas_dialogs.format_points.clear_overrides;
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("exclude:tag:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(txt) = buffer_text(app, "exclude.note_input") {
            app.canvas_dialogs.exclude_with_reason.note = txt;
        }
        if let Some(tag) = crate::dialogs_canvas::ExcludeWithReasonDialogState::preset_reason_tags().get(idx) {
            app.canvas_dialogs.exclude_with_reason.reason_tag = (*tag).to_owned();
        }
        return Some(handled_outcome(true));
    }
    if payload == "exclude:apply_note" {
        if let Some(txt) = buffer_text(app, "exclude.note_input") {
            app.canvas_dialogs.exclude_with_reason.note = txt;
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("apply_style:donor:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some((sid, _)) = app.canvas_dialogs.apply_style.donor_candidates.get(idx) {
            app.canvas_dialogs.apply_style.donor_graph_id = Some(sid.clone());
        }
        return Some(handled_outcome(true));
    }
    if let Some(field) = payload.strip_prefix("apply_style:toggle:") {
        match field {
            "palette" => app.canvas_dialogs.apply_style.clone_palette = !app.canvas_dialogs.apply_style.clone_palette,
            "frame" => app.canvas_dialogs.apply_style.clone_frame_and_axes = !app.canvas_dialogs.apply_style.clone_frame_and_axes,
            "legend" => {
                app.canvas_dialogs.apply_style.clone_error_bars_and_legend = !app.canvas_dialogs.apply_style.clone_error_bars_and_legend;
            }
            _ => {}
        }
        return Some(handled_outcome(true));
    }
    None
}

fn handle_layout_eq_rand_sim_print_action(
    app: &mut UiApp,
    state: &mut DialogsGraphMiscViewState,
    payload: &str,
) -> Option<Result<UiEventOutcome, String>> {
    if let Some(res) = handle_page_layout_eq_action(app, payload) {
        return Some(res);
    }
    if let Some(res) = handle_rand_sim_action(app, state, payload) {
        return Some(res);
    }
    handle_print_action(app, payload)
}

fn handle_page_layout_eq_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(raw_idx) = payload.strip_prefix("page_layout:preset:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        let presets = [
            JournalWidthPreset::SingleColumn85Mm,
            JournalWidthPreset::OneAndHalfColumn120Mm,
            JournalWidthPreset::DoubleColumn174Mm,
            JournalWidthPreset::A4Portrait,
            JournalWidthPreset::A4Landscape,
        ];
        if let Some(&preset) = presets.get(idx) {
            app.canvas_dialogs.page_layout.apply_preset(preset);
        }
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:orientation" {
        app.canvas_dialogs.page_layout.orientation = match app.canvas_dialogs.page_layout.orientation {
            PageOrientation::Portrait => PageOrientation::Landscape,
            PageOrientation::Landscape => PageOrientation::Portrait,
        };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:rows_dec" || payload == "page_layout:rows_inc" {
        let cur = app.canvas_dialogs.page_layout.rows;
        app.canvas_dialogs.page_layout.rows = if payload == "page_layout:rows_inc" { (cur + 1).min(8) } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:cols_dec" || payload == "page_layout:cols_inc" {
        let cur = app.canvas_dialogs.page_layout.cols;
        app.canvas_dialogs.page_layout.cols = if payload == "page_layout:cols_inc" { (cur + 1).min(8) } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:arrange" {
        let graph_ids: Vec<SheetId> = app.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
        app.canvas_dialogs.page_layout.arrange_grid(&graph_ids, 24.0, 12.0);
        return Some(handled_outcome(true));
    }
    if let Some(badge) = payload.strip_prefix("page_layout:badge:") {
        app.canvas_dialogs.page_layout.panel_badges = match badge {
            "lower" => PanelBadgeStyle::LowerCaseLetters,
            "none" => PanelBadgeStyle::None,
            _ => PanelBadgeStyle::UpperCaseLetters,
        };
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:add_callout" {
        if let Some(txt) = buffer_text(app, "page_layout.callout_input") {
            app.canvas_dialogs.page_layout.new_callout_markdown = txt;
        }
        let md = app.canvas_dialogs.page_layout.new_callout_markdown.trim().to_owned();
        if !md.is_empty() {
            let next_id = format!("callout-{}", app.canvas_dialogs.page_layout.items.len() + 1);
            app.canvas_dialogs.page_layout.items.push(LayoutItem::TextCallout {
                id: next_id,
                markdown: md,
                bounds_pt: RectPt::new(24.0, 24.0, 180.0, 28.0),
                font_size_pt: 11.0,
            });
        }
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("eq:result:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(rid) = app.canvas_dialogs.insert_model_equation.available_result_ids.get(idx).cloned() {
            app.canvas_dialogs.insert_model_equation.result_sheet_id = Some(rid);
        }
        return Some(handled_outcome(true));
    }
    if payload == "eq:ds_dec" || payload == "eq:ds_inc" {
        let cur = app.canvas_dialogs.insert_model_equation.data_set_index;
        app.canvas_dialogs.insert_model_equation.data_set_index = if payload == "eq:ds_inc" { cur + 1 } else { cur.saturating_sub(1) };
        return Some(handled_outcome(true));
    }
    if payload == "eq:font_dec" || payload == "eq:font_inc" {
        let cur = app.canvas_dialogs.insert_model_equation.font_size_pt;
        let next = if payload == "eq:font_inc" { cur + 1.0 } else { cur - 1.0 };
        app.canvas_dialogs.insert_model_equation.font_size_pt = next.clamp(6.0, 36.0);
        return Some(handled_outcome(true));
    }
    if payload == "eq:bordered" {
        app.canvas_dialogs.insert_model_equation.bordered = !app.canvas_dialogs.insert_model_equation.bordered;
        return Some(handled_outcome(true));
    }
    if payload == "eq:toggle_mode" {
        let _ = app.canvas_dialogs.insert_model_equation.toggle_mode();
        return Some(handled_outcome(true));
    }
    if payload == "eq:update_existing" {
        app.canvas_dialogs.insert_model_equation.update_existing = !app.canvas_dialogs.insert_model_equation.update_existing;
        return Some(handled_outcome(true));
    }
    if payload == "page_layout:align_origins" {
        app.canvas_dialogs.page_layout.align_plot_origins = !app.canvas_dialogs.page_layout.align_plot_origins;
        return Some(handled_outcome(true));
    }
    None
}

fn handle_rand_sim_action(app: &mut UiApp, state: &mut DialogsGraphMiscViewState, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if let Some(preset) = payload.strip_prefix("rand_groups:arms:") {
        let raw = match preset {
            "three" => "Control, Low Dose, High Dose",
            "weighted" => "Control:1, Treatment:2",
            _ => "Control, Treatment",
        };
        let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(raw);
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:apply_text" {
        if let Some(txt) = buffer_text(app, "rand_groups.arms_input") {
            let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(&txt);
        }
        if let Some(txt) = buffer_text(app, "rand_groups.title_input") {
            app.canvas_dialogs.randomize_groups.title = txt;
        }
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:n_dec" || payload == "rand_groups:n_inc" {
        let cur = app.canvas_dialogs.randomize_groups.subject_count;
        app.canvas_dialogs.randomize_groups.subject_count =
            if payload == "rand_groups:n_inc" { (cur + 4).min(500) } else { cur.saturating_sub(4).max(2) };
        return Some(handled_outcome(true));
    }
    if let Some(scheme) = payload.strip_prefix("rand_groups:scheme:") {
        app.canvas_dialogs.randomize_groups.scheme =
            if scheme == "block" { RandomizationSchemeKind::PermutedBlock } else { RandomizationSchemeKind::CompleteBalanced };
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:block_dec" || payload == "rand_groups:block_inc" {
        let cur = app.canvas_dialogs.randomize_groups.block_sizes.first().copied().unwrap_or(4);
        let next = if payload == "rand_groups:block_inc" { (cur + 2).min(64) } else { cur.saturating_sub(2).max(2) };
        app.canvas_dialogs.randomize_groups.block_sizes = vec![next];
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:seed_dec" || payload == "rand_groups:seed_inc" {
        let cur = app.canvas_dialogs.randomize_groups.seed;
        app.canvas_dialogs.randomize_groups.seed = if payload == "rand_groups:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "rand_groups:toggle_info" {
        let next = if app.canvas_dialogs.randomize_groups.target_info_sheet_id.is_some() {
            None
        } else {
            app.document().info_sheets.first().map(|s| s.id.clone())
        };
        app.canvas_dialogs.randomize_groups.target_info_sheet_id = next;
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("rand_sample:src:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(ds) = app.document().data_sheets.get(idx) {
            app.canvas_dialogs.random_sample_rows.source_sheet_id = Some(ds.id.clone());
        }
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:n_dec" || payload == "rand_sample:n_inc" {
        let cur = app.canvas_dialogs.random_sample_rows.sample_size;
        app.canvas_dialogs.random_sample_rows.sample_size =
            if payload == "rand_sample:n_inc" { (cur + 5).min(5000) } else { cur.saturating_sub(5).max(1) };
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:seed_dec" || payload == "rand_sample:seed_inc" {
        let cur = app.canvas_dialogs.random_sample_rows.seed;
        app.canvas_dialogs.random_sample_rows.seed = if payload == "rand_sample:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if let Some(raw) = payload.strip_prefix("rand_sample:stratify:") {
        app.canvas_dialogs.random_sample_rows.stratify_column = if raw == "none" { None } else { raw.parse::<usize>().ok() };
        return Some(handled_outcome(true));
    }
    if payload == "rand_sample:proportional" {
        app.canvas_dialogs.random_sample_rows.proportional_allocation = !app.canvas_dialogs.random_sample_rows.proportional_allocation;
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:multi_var" {
        app.canvas_dialogs.simulate_distributions.use_multiple_variables = !app.canvas_dialogs.simulate_distributions.use_multiple_variables;
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:rows_dec" || payload == "sim_dist:rows_inc" {
        let cur = app.canvas_dialogs.simulate_distributions.n_rows;
        app.canvas_dialogs.simulate_distributions.n_rows =
            if payload == "sim_dist:rows_inc" { (cur + 25).min(10_000) } else { cur.saturating_sub(25).max(5) };
        return Some(handled_outcome(true));
    }
    if payload == "sim_dist:seed_dec" || payload == "sim_dist:seed_inc" {
        let cur = app.canvas_dialogs.simulate_distributions.seed;
        app.canvas_dialogs.simulate_distributions.seed = if payload == "sim_dist:seed_inc" { cur + 1 } else { cur.saturating_sub(1).max(1) };
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("sim_dist:col:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        state.sim_selected_col_idx = idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
        return Some(handled_outcome(false));
    }
    if payload == "sim_dist:add_col" {
        let idx = app.canvas_dialogs.simulate_distributions.columns.len();
        if idx < 12 {
            let draft = crate::dialogs_r9::SimulatedDistColumnDraft { name: format!("Var {}", idx + 1), ..Default::default() };
            app.canvas_dialogs.simulate_distributions.columns.push(draft);
            state.sim_selected_col_idx = idx;
        }
        return Some(handled_outcome(true));
    }
    if let Some(dist_s) = payload.strip_prefix("sim_dist:dist:") {
        let idx = state.sim_selected_col_idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
        if let Some(col) = app.canvas_dialogs.simulate_distributions.columns.get_mut(idx) {
            col.distribution = match dist_s {
                "uniform" => crate::dialogs_r9::SimDistKind::Uniform,
                "lognormal" => crate::dialogs_r9::SimDistKind::LogNormal,
                "binomial" => crate::dialogs_r9::SimDistKind::Binomial,
                "poisson" => crate::dialogs_r9::SimDistKind::Poisson,
                "exponential" => crate::dialogs_r9::SimDistKind::Exponential,
                _ => crate::dialogs_r9::SimDistKind::Gaussian,
            };
        }
        return Some(handled_outcome(true));
    }
    if let Some(p_op) = payload.strip_prefix("sim_dist:param:") {
        return Some(apply_sim_dist_param_step(app, state, p_op));
    }
    None
}

fn apply_sim_dist_param_step(app: &mut UiApp, state: &DialogsGraphMiscViewState, p_op: &str) -> Result<UiEventOutcome, String> {
    let idx = state.sim_selected_col_idx.min(app.canvas_dialogs.simulate_distributions.columns.len().saturating_sub(1));
    if let Some(col) = app.canvas_dialogs.simulate_distributions.columns.get_mut(idx) {
        let s = if p_op.ends_with("inc") { 1.0 } else { -1.0 };
        if p_op.starts_with("p1_") {
            match col.distribution {
                crate::dialogs_r9::SimDistKind::Uniform => col.min += s,
                crate::dialogs_r9::SimDistKind::Gaussian => col.mean += s,
                crate::dialogs_r9::SimDistKind::LogNormal => col.mean_log += s * 0.25,
                crate::dialogs_r9::SimDistKind::Binomial => {
                    let next = i64::try_from(col.trials).unwrap_or(10) + if s > 0.0 { 1 } else { -1 };
                    col.trials = u64::try_from(next.clamp(1, 500)).unwrap_or(10);
                }
                crate::dialogs_r9::SimDistKind::Poisson => col.lambda = (col.lambda + s).clamp(0.5, 500.0),
                crate::dialogs_r9::SimDistKind::Exponential => col.rate = (col.rate + s * 0.25).clamp(0.05, 100.0),
            }
        } else if p_op.starts_with("p2_") {
            match col.distribution {
                crate::dialogs_r9::SimDistKind::Uniform => col.max = (col.max + s).max(col.min + 0.5),
                crate::dialogs_r9::SimDistKind::Gaussian => col.sd = (col.sd + s * 0.5).clamp(0.1, 1000.0),
                crate::dialogs_r9::SimDistKind::LogNormal => col.sd_log = (col.sd_log + s * 0.1).clamp(0.05, 10.0),
                crate::dialogs_r9::SimDistKind::Binomial => col.p = (col.p + s * 0.05).clamp(0.05, 0.95),
                crate::dialogs_r9::SimDistKind::Poisson | crate::dialogs_r9::SimDistKind::Exponential => {}
            }
        }
    }
    handled_outcome(true)
}

fn handle_print_action(app: &mut UiApp, payload: &str) -> Option<Result<UiEventOutcome, String>> {
    if payload == "print:printer:default" {
        app.canvas_dialogs.print.select_printer(None);
        return Some(handled_outcome(true));
    }
    if let Some(raw_idx) = payload.strip_prefix("print:printer:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        let p = app.canvas_dialogs.print.discovered_printers.get(idx).cloned();
        app.canvas_dialogs.print.select_printer(p);
        return Some(handled_outcome(true));
    }
    if payload == "print:copies_dec" || payload == "print:copies_inc" {
        let cur = app.canvas_dialogs.print.copies;
        let next = if payload == "print:copies_inc" { cur.saturating_add(1) } else { cur.saturating_sub(1) };
        app.canvas_dialogs.print.update_copies(next);
        return Some(handled_outcome(true));
    }
    if payload == "print:scale_dec" || payload == "print:scale_inc" {
        let cur = app.canvas_dialogs.print.job.scale_percent;
        let next = if payload == "print:scale_inc" { cur + 10.0 } else { cur - 10.0 };
        app.canvas_dialogs.print.set_scale_percent(next);
        return Some(handled_outcome(true));
    }
    if payload == "print:bleed_dec" || payload == "print:bleed_inc" {
        let cur = app.canvas_dialogs.print.job.bleed_pt;
        let next = if payload == "print:bleed_inc" { cur + 2.0 } else { cur - 2.0 };
        app.canvas_dialogs.print.job.bleed_pt = next.clamp(0.0, 36.0);
        return Some(handled_outcome(true));
    }
    if payload == "print:crop_marks" {
        app.canvas_dialogs.print.job.crop_marks = !app.canvas_dialogs.print.job.crop_marks;
        return Some(handled_outcome(true));
    }
    if payload == "print:submit_to_os" {
        if !app.is_gui_only() {
            app.canvas_dialogs.print.submit_to_os = !app.canvas_dialogs.print.submit_to_os;
        }
        return Some(handled_outcome(true));
    }
    None
}
