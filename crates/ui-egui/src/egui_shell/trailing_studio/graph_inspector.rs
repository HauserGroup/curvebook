//! 5-Tab Graph Inspector studio surface (`Data`, `Axes`, `Legends`, `Annotations`, `Canvas`).

use curvebook_doc::{
    AxisSpec, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphAnnotation, GraphSheet, GraphicalDataObject, GroupedArrangement,
    PairwiseDisplayMode, SeriesSpec,
};
use curvebook_math::AxisScale;
use curvebook_palette::{CanvasPreviewFilter, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use egui::{Color32, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, UiIntent, widgets};

use super::helpers::{MediaControlsSpec, StudioButtonSpec, StudioChipSpec, render_canvas_media_buttons, studio_chip, studio_full_button};
use super::{GraphStudioTab, TrailingStudioContext, TrailingStudioEguiState};

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

fn current_series_marker_shape(series: &SeriesSpec) -> Option<MarkerShape> {
    match &series.gdo {
        GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } | GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => Some(m.shape),
        GraphicalDataObject::AlignedDotPlot { marker, .. } => Some(marker.shape),
        _ => None,
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

const fn extract_series_alpha_byte(gdo: &GraphicalDataObject) -> u8 {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
            if let Some(m) = marker {
                m.fill_color.a
            } else if let Some(l) = line {
                l.color.a
            } else if let Some(af) = area_fill {
                af.fill_color.a
            } else {
                255
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => fill_color.a,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color.a,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => line_color.a,
        GraphicalDataObject::PartsOfWhole { .. } => 255,
    }
}

fn render_graph_orientation_chips(
    ui: &mut egui::Ui,
    gs: &GraphSheet,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !matches!(gs.spec.kind, curvebook_doc::GraphKind::Column | curvebook_doc::GraphKind::Grouped) {
        return;
    }
    ui.add_space(4.0);
    widgets::subheader(ui, "Bar / Plot Orientation");
    ui.horizontal_wrapped(|row| {
        row.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (slug, lbl, target) in [
            ("vertical", "Vertical", curvebook_doc::CartesianOrientation::Vertical),
            ("horizontal", "Horizontal", curvebook_doc::CartesianOrientation::Horizontal),
        ] {
            if studio_chip(
                row,
                ctx_in.tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Orient column/grouped plot vertically or horizontally (graph.setSpec)",
                    max_w: 96.0,
                    selected: gs.spec.orientation == target,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &format!("inspector.graph.orientation.{slug}"),
                    role: "button",
                    command_id: Some("graph.setSpec"),
                    disabled_reason: None,
                },
            ) {
                intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "orientation": slug }));
            }
        }
    });
}

fn render_graph_studio_tabs(
    ui: &mut egui::Ui,
    state: &mut TrailingStudioEguiState,
    tokens: &crate::egui_shell::Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) {
    let tab_kinds = [GraphStudioTab::Data, GraphStudioTab::Axes, GraphStudioTab::Legends, GraphStudioTab::Annotations, GraphStudioTab::Canvas];
    let tab_specs = [
        crate::egui_shell::ui_kit::UnderlineTabSpec { widget_id: "inspector.graph.tab.data", label: "Data", tooltip: Some("Data & Series") },
        crate::egui_shell::ui_kit::UnderlineTabSpec { widget_id: "inspector.graph.tab.axes", label: "Axes", tooltip: Some("Axes & Frame") },
        crate::egui_shell::ui_kit::UnderlineTabSpec { widget_id: "inspector.graph.tab.legends", label: "Legend", tooltip: Some("Legend") },
        crate::egui_shell::ui_kit::UnderlineTabSpec {
            widget_id: "inspector.graph.tab.annotations",
            label: "Annotations",
            tooltip: Some("Annotations & Pairwise"),
        },
        crate::egui_shell::ui_kit::UnderlineTabSpec { widget_id: "inspector.graph.tab.canvas", label: "Canvas", tooltip: Some("Canvas & Export") },
    ];
    let cur_tab_idx = tab_kinds.iter().position(|&k| k == state.graph_tab).unwrap_or(0);
    if let Some(clicked_idx) =
        crate::egui_shell::ui_kit::underline_tab_bar(ui, tokens, meta, &tab_specs, cur_tab_idx, crate::inspect::ZLayer::Workspace)
        && enabled
    {
        state.graph_tab = tab_kinds[clicked_idx];
    }
    ui.add_space(tokens.metrics.space_xs);
}

fn render_graph_series_selector(
    ui: &mut egui::Ui,
    gs: &GraphSheet,
    state: &mut TrailingStudioEguiState,
    tokens: &crate::egui_shell::Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) {
    if gs.spec.series.is_empty() {
        return;
    }
    let sel_idx = state.graph_series_index;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (idx, s) in gs.spec.series.iter().enumerate() {
            let base_lbl = s.legend_label.clone().unwrap_or_else(|| format!("Series {}", idx + 1));
            let lbl = format!("● {base_lbl}");
            let rgba = series_primary_color(s);
            let swatch = Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, 255);
            let wid = format!("inspector.graph.series.{idx}");
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: &lbl,
                    tooltip: &base_lbl,
                    max_w: 120.0,
                    selected: idx == sel_idx,
                    enabled,
                    border_override: Some(swatch),
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.graph_series_index = idx;
            }
        }
    });
}

pub(super) fn render_graph_inspector_studio(
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

    render_graph_studio_tabs(ui, state, tokens, ctx_in.enabled, meta);

    let series_count = gs.spec.series.len();
    if series_count > 0 && state.graph_series_index >= series_count {
        state.graph_series_index = 0;
    }
    let sel_idx = state.graph_series_index;

    match state.graph_tab {
        GraphStudioTab::Data => {
            widgets::subheader(ui, "Series & Error Bars");
            render_graph_series_selector(ui, gs, state, tokens, ctx_in.enabled, meta);

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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: pal.label(),
                            max_w: 86.0,
                            selected: gs.spec.palette == pal,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.setSpec"),
                            disabled_reason: None,
                        },
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
                ui.add_space(tokens.metrics.space_xs);
                widgets::subheader(ui, "Symbol Shape");
                let shapes = [
                    (MarkerShape::Circle, "●", "inspector.graph.shape.Circle"),
                    (MarkerShape::Square, "■", "inspector.graph.shape.Square"),
                    (MarkerShape::TriangleUp, "▲", "inspector.graph.shape.TriangleUp"),
                    (MarkerShape::Diamond, "◆", "inspector.graph.shape.Diamond"),
                ];
                let active_shape = current_series_marker_shape(series);
                let active_shape_idx = active_shape.and_then(|cur| shapes.iter().position(|(s, _, _)| *s == cur)).unwrap_or(usize::MAX);
                let seg_opts: Vec<crate::egui_shell::ui_kit::SegmentedOption<'_>> = shapes
                    .iter()
                    .map(|(_, lbl, wid)| crate::egui_shell::ui_kit::SegmentedOption {
                        widget_id: wid,
                        label: lbl,
                        tooltip: Some("Set series marker shape"),
                        value: None,
                    })
                    .collect();
                if let Some(chosen_idx) = crate::egui_shell::ui_kit::segmented_control(
                    ui,
                    tokens,
                    meta,
                    &seg_opts,
                    active_shape_idx,
                    ctx_in.enabled,
                    crate::inspect::ZLayer::Workspace,
                ) {
                    let mut cloned = series.clone();
                    if set_series_marker_shape(&mut cloned, shapes[chosen_idx].0) {
                        intents.push_engine("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": cloned.gdo }));
                    }
                }

                ui.add_space(tokens.metrics.space_xs);
                widgets::subheader(ui, "Opacity & Overlay Visibility");
                let cur_alpha = extract_series_alpha_byte(&series.gdo);
                let cur_pct = ((f64::from(cur_alpha) / 255.0) * 100.0).round() as u8;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for pct in [25u8, 50, 75, 100] {
                        let lbl = format!("{pct}%");
                        let wid = format!("inspector.graph.alpha.{pct}");
                        let alpha_u8 = ((f64::from(pct) / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
                        if studio_chip(
                            ui,
                            tokens,
                            meta,
                            &StudioChipSpec {
                                label: &lbl,
                                tooltip: "Set series opacity percentage",
                                max_w: 46.0,
                                selected: cur_pct.abs_diff(pct) <= 4,
                                enabled: ctx_in.enabled,
                                border_override: None,
                                widget_id: &wid,
                                role: "button",
                                command_id: Some("graph.setSeries"),
                                disabled_reason: None,
                            },
                        ) {
                            intents.push_engine("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "alpha": alpha_u8 }));
                        }
                    }
                });

                ui.add_space(tokens.metrics.space_xs);
                if crate::egui_shell::ui_kit::choice_row(
                    ui,
                    tokens,
                    meta,
                    crate::egui_shell::ui_kit::ChoiceRowSpec {
                        widget_id: "inspector.graph.show_raw_data",
                        label: "Data points",
                        checked: series.show_raw_data,
                        is_radio: false,
                        enabled: ctx_in.enabled,
                        disabled_reason: None,
                        font_size: tokens.metrics.font_ui_body_pt,
                        row_height: tokens.metrics.min_hit_target_pt,
                        z_layer: crate::inspect::ZLayer::Workspace,
                        value: Value::Bool(series.show_raw_data),
                        tooltip_mode: crate::egui_shell::ui_kit::TooltipMode::Standard,
                    },
                ) {
                    intents.push_engine(
                        "graph.setSeries",
                        json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "show_raw_data": !series.show_raw_data }),
                    );
                }

                if series.regression_overlay.is_some()
                    && crate::egui_shell::ui_kit::choice_row(
                        ui,
                        tokens,
                        meta,
                        crate::egui_shell::ui_kit::ChoiceRowSpec {
                            widget_id: "inspector.graph.hide_fitted_curve",
                            label: "Fitted curve overlay",
                            checked: true,
                            is_radio: false,
                            enabled: ctx_in.enabled,
                            disabled_reason: None,
                            font_size: tokens.metrics.font_ui_body_pt,
                            row_height: tokens.metrics.min_hit_target_pt,
                            z_layer: crate::inspect::ZLayer::Workspace,
                            value: Value::Bool(true),
                            tooltip_mode: crate::egui_shell::ui_kit::TooltipMode::Standard,
                        },
                    )
                {
                    intents.push_engine(
                        "graph.setSeries",
                        json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "regression_overlay": Value::Null }),
                    );
                }

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
                            meta,
                            &StudioChipSpec {
                                label: lbl,
                                tooltip: "Error bar dispersion metric",
                                max_w: 62.0,
                                selected: cur_metric == metric,
                                enabled: ctx_in.enabled,
                                border_override: None,
                                widget_id: &wid,
                                role: "button",
                                command_id: Some("graph.setSeries"),
                                disabled_reason: None,
                            },
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
                            meta,
                            &StudioChipSpec {
                                label: lbl,
                                tooltip: "Grouped bar/point cluster arrangement",
                                max_w: 92.0,
                                selected: gs.spec.grouped_arrangement == arr,
                                enabled: ctx_in.enabled,
                                border_override: None,
                                widget_id: &wid,
                                role: "button",
                                command_id: Some("graph.setSpec"),
                                disabled_reason: None,
                            },
                        ) {
                            intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "grouped_arrangement": arr } }));
                        }
                    }
                });
            }

            render_graph_orientation_chips(ui, gs, ctx_in, meta, intents);

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "Format Graph…",
                    tooltip: "Open Format Graph dialog",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.more_format",
                    command_id: Some("ui.graph.format.open"),
                    disabled_reason: None,
                },
            ) {
                intents.push_ui("ui.graph.format.open", json!({ "sheet_id": gs.id.as_str() }));
            }
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "Bubble & MV Encoding…",
                    tooltip: "Open Multivariate Bubble Plot settings",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.more_bubble",
                    command_id: Some("ui.graph.bubblePlot.open"),
                    disabled_reason: None,
                },
            ) {
                intents.push_ui("ui.graph.bubblePlot.open", json!({ "sheet_id": gs.id.as_str() }));
            }
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "Apply Style From…",
                    tooltip: "Clone graph styling from another sheet",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.apply_style",
                    command_id: Some("ui.applyStyleFrom.open"),
                    disabled_reason: None,
                },
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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Select target axis",
                            max_w: 54.0,
                            selected: state.graph_axis_target == idx,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: None,
                            disabled_reason: None,
                        },
                    ) {
                        state.graph_axis_target = idx;
                    }
                }
            });

            let default_y2 = AxisSpec::linear("Right Y");
            let target_axis: &AxisSpec = match state.graph_axis_target {
                0 => &gs.spec.x_axis,
                2 => gs.spec.y2_axis.as_ref().unwrap_or(&default_y2),
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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Set axis scale transform",
                            max_w: 56.0,
                            selected: cur_scale == sc,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.setAxes"),
                            disabled_reason: None,
                        },
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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Set frame spine arrangement",
                            max_w: 68.0,
                            selected: gs.spec.frame_preset == fp,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.setAxes"),
                            disabled_reason: None,
                        },
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
                meta,
                &StudioButtonSpec {
                    label: "More… Format Axes",
                    tooltip: "Open Format Axes dialog",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.more_axes",
                    command_id: Some("ui.graph.axes.open"),
                    disabled_reason: None,
                },
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
                meta,
                &StudioButtonSpec {
                    label: if legend.visible { "Legend: Visible" } else { "Legend: Hidden" },
                    tooltip: "Toggle legend visibility on graph",
                    selected: legend.visible,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.legend_visible",
                    command_id: Some("graph.setSpec"),
                    disabled_reason: None,
                },
            ) {
                legend.visible = !legend.visible;
                intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "legend": legend } }));
            }
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: if legend.bordered { "Border Box: On" } else { "Border Box: Off" },
                    tooltip: "Toggle legend frame border",
                    selected: legend.bordered,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.legend_border",
                    command_id: Some("graph.setSpec"),
                    disabled_reason: None,
                },
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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Set pairwise significance display mode",
                            max_w: 68.0,
                            selected: gs.spec.pairwise_display_mode == mode,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.setSpec"),
                            disabled_reason: None,
                        },
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
                meta,
                &StudioButtonSpec {
                    label: "More… Pairwise & CLD",
                    tooltip: "Open Pairwise Significance & CLD dialog",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.more_pairwise",
                    command_id: Some("ui.graph.pairwise.open"),
                    disabled_reason: None,
                },
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
                        GraphAnnotation::EmbeddedImage { asset_id, .. } => format!("Image: {asset_id}"),
                    };
                    let wid = format!("inspector.graph.annot.remove.{idx}");
                    if studio_full_button(
                        ui,
                        tokens,
                        meta,
                        &StudioButtonSpec {
                            label: &format!("✕ {summary}"),
                            tooltip: "Remove annotation",
                            selected: false,
                            enabled: ctx_in.enabled,
                            primary: false,
                            widget_id: &wid,
                            command_id: Some("graph.setSpec"),
                            disabled_reason: None,
                        },
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
                meta,
                &StudioButtonSpec {
                    label: "+ Add Model Equation…",
                    tooltip: "Insert live regression equation callout",
                    selected: false,
                    enabled: ctx_in.enabled && !doc.result_sheets.is_empty(),
                    primary: false,
                    widget_id: "inspector.graph.add_equation",
                    command_id: Some("ui.graph.insertModelEquation"),
                    disabled_reason: doc.result_sheets.is_empty().then_some("no result sheets available"),
                },
            ) {
                intents.push_ui("ui.graph.insertModelEquation", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
        GraphStudioTab::Canvas => {
            widgets::subheader(ui, "Canvas & Publication Presets");
            widgets::dim_label(ui, &format!("{:.1} × {:.1} pt", gs.spec.canvas_size_pt.width, gs.spec.canvas_size_pt.height));

            ui.add_space(4.0);
            widgets::dim_label(ui, "Paper Background");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (slug, lbl, fill_val) in [
                    ("white", "White", Some(Rgba8::WHITE)),
                    ("cream", "Cream", Some(Rgba8::rgb(255, 251, 235))),
                    ("slate", "Slate", Some(Rgba8::rgb(241, 245, 249))),
                    ("transparent", "None", None),
                ] {
                    let wid = format!("inspector.graph.paper_fill.{slug}");
                    let selected = gs.spec.paper_fill == fill_val;
                    if studio_chip(
                        ui,
                        tokens,
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Set graph outer paper background fill",
                            max_w: 54.0,
                            selected,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.setSpec"),
                            disabled_reason: None,
                        },
                    ) {
                        let next_fill = fill_val.map_or(Value::Null, |c| json!(c));
                        intents.push_engine("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "paper_fill": next_fill }));
                    }
                }
            });

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
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Apply publication canvas preset",
                            max_w: 72.0,
                            selected: false,
                            enabled: ctx_in.enabled,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("graph.applyCanvasPreset"),
                            disabled_reason: None,
                        },
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
                meta,
                &StudioButtonSpec {
                    label: if is_gray { "Grayscale Print Preview: On" } else { "Grayscale Print Preview: Off" },
                    tooltip: "Toggle grayscale publication readability preview",
                    selected: is_gray,
                    enabled: ctx_in.enabled,
                    primary: false,
                    widget_id: "inspector.graph.grayscale_toggle",
                    command_id: Some("ui.previewFilter.set"),
                    disabled_reason: None,
                },
            ) {
                intents.push(UiIntent::SetPreviewFilter {
                    filter: if is_gray { CanvasPreviewFilter::Normal } else { CanvasPreviewFilter::GrayscalePrint },
                });
            }

            render_graph_canvas_media_controls(ui, app, gs, ctx_in, meta, intents);

            ui.add_space(6.0);
            widgets::divider(ui);
            ui.add_space(4.0);
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "More… Export Figure",
                    tooltip: "Open Figure Export dialog (PDF / SVG / PNG)",
                    selected: false,
                    enabled: ctx_in.enabled,
                    primary: true,
                    widget_id: "inspector.graph.export_fig",
                    command_id: Some("ui.export.figure.open"),
                    disabled_reason: None,
                },
            ) {
                intents.push_ui("ui.export.figure.open", json!({ "sheet_id": gs.id.as_str() }));
            }
        }
    }
}

fn render_graph_canvas_media_controls(
    ui: &mut egui::Ui,
    app: &UiApp,
    gs: &GraphSheet,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let sel = ctx_in.selected_graph_annotation;
    let img_idx = sel
        .filter(|&i| matches!(gs.spec.annotations.get(i), Some(GraphAnnotation::EmbeddedImage { .. })))
        .or_else(|| gs.spec.annotations.iter().position(|a| matches!(a, GraphAnnotation::EmbeddedImage { .. })));
    let table_idx = sel
        .filter(|&i| matches!(gs.spec.annotations.get(i), Some(GraphAnnotation::EmbeddedResultTable { .. })))
        .or_else(|| gs.spec.annotations.iter().position(|a| matches!(a, GraphAnnotation::EmbeddedResultTable { .. })));
    render_canvas_media_buttons(
        ui,
        app,
        MediaControlsSpec { prefix: "canvas", sheet_id: &gs.id, insert_cmd: "graph.insertImage", idx_key: "annotation_index", img_idx, table_idx },
        ctx_in,
        meta,
        intents,
    );
}
