//! Page Layout Composer studio surface.

use curvebook_doc::{LayoutItem, LayoutSheet, LayoutSpec, PageOrientation, PanelBadgeStyle, RectPt};
use egui::Vec2;
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, widgets};
use crate::i18n::tr;

use super::helpers::{MediaControlsSpec, StudioButtonSpec, StudioChipSpec, render_canvas_media_buttons, studio_chip, studio_full_button};
use super::{TrailingStudioContext, TrailingStudioEguiState};

fn arrange_layout_grid_items(spec: &LayoutSpec, rows_u8: u8, cols_u8: u8, margin_mm: u16) -> Vec<LayoutItem> {
    let page = spec.page_size_pt();
    let margin = f64::from(margin_mm) * (72.0 / 25.4);
    let gutter = 12.0_f64;
    let rows = usize::from(rows_u8.max(1));
    let cols = usize::from(cols_u8.max(1));
    let usable_w = (page.width - 2.0 * margin - gutter * ((cols - 1) as f64)).max(40.0);
    let usable_h = (page.height - 2.0 * margin - gutter * ((rows - 1) as f64)).max(40.0);
    let cell_w = usable_w / (cols as f64);
    let cell_h = usable_h / (rows as f64);
    let mut updated = spec.items.clone();
    let mut grid_slot = 0usize;
    for item in &mut updated {
        if let LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } = item {
            let col = grid_slot % cols;
            let row = grid_slot / cols;
            *bounds_pt = RectPt::new(margin + (col as f64) * (cell_w + gutter), margin + (row as f64) * (cell_h + gutter), cell_w, cell_h);
            grid_slot += 1;
        }
    }
    updated
}

fn push_margin_rearrange_intent(ls: &LayoutSheet, margin_mm: u16, intents: &mut PendingUiIntents) {
    if ls.spec.items.is_empty() {
        return;
    }
    let (rows_u8, cols_u8) = ls.spec.grid_template.unwrap_or((2, 2));
    let next_items = arrange_layout_grid_items(&ls.spec, rows_u8, cols_u8, margin_mm);
    if next_items != ls.spec.items {
        intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": { "items": next_items } }));
    }
}

pub(super) fn render_page_layout_composer_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
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
            meta,
            &StudioChipSpec {
                label: "85 mm",
                tooltip: "Single-column journal width (85 mm)",
                max_w: 68.0,
                selected: is_85,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.preset.85mm",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "85mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "114 mm",
                tooltip: "1.5-column journal width (114 mm)",
                max_w: 68.0,
                selected: is_114,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.preset.114mm",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "width": 114.0, "height": 90.0, "unit": "mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "174 mm",
                tooltip: "Double-column journal width (174 mm)",
                max_w: 68.0,
                selected: is_174,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.preset.174mm",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "174mm" }));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "A4",
                tooltip: "ISO A4 portrait (210 × 297 mm)",
                max_w: 48.0,
                selected: false,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.preset.a4",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "preset": "a4_portrait" }));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "Letter",
                tooltip: "US Letter portrait (8.5 × 11 in)",
                max_w: 58.0,
                selected: false,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.preset.letter",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
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
            meta,
            &StudioChipSpec {
                label: "Portrait",
                tooltip: "Portrait page orientation",
                max_w: 74.0,
                selected: is_port,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.orient.portrait",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "orientation": "portrait" }));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "Landscape",
                tooltip: "Landscape page orientation",
                max_w: 84.0,
                selected: !is_port,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.orient.landscape",
                role: "button",
                command_id: Some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "orientation": "landscape" }));
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new(tr(locale, "inspector.layout.margins")).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "-",
                tooltip: "Decrease page margin (mm)",
                max_w: 28.0,
                selected: false,
                enabled: ctx_in.enabled && state.layout_margin_mm > 2,
                border_override: None,
                widget_id: "inspector.layout.margin.dec",
                role: "button",
                command_id: (!spec.items.is_empty()).then_some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            state.layout_margin_mm = state.layout_margin_mm.saturating_sub(2).max(2);
            push_margin_rearrange_intent(ls, state.layout_margin_mm, intents);
        }
        ui.label(
            egui::RichText::new(format!("{} mm", state.layout_margin_mm)).size(tokens.metrics.font_ui_small_pt).strong().color(tokens.text_primary),
        );
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "+",
                tooltip: "Increase page margin (mm)",
                max_w: 28.0,
                selected: false,
                enabled: ctx_in.enabled && state.layout_margin_mm < 40,
                border_override: None,
                widget_id: "inspector.layout.margin.inc",
                role: "button",
                command_id: (!spec.items.is_empty()).then_some("layout.setSpec"),
                disabled_reason: None,
            },
        ) {
            state.layout_margin_mm = (state.layout_margin_mm + 2).min(40);
            push_margin_rearrange_intent(ls, state.layout_margin_mm, intents);
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
                meta,
                &StudioChipSpec {
                    label: &format!("{dpi} DPI"),
                    tooltip: "Target rasterization / print DPI",
                    max_w: 64.0,
                    selected: state.layout_dpi == dpi,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.layout_dpi = dpi;
            }
        }
        let snap_lbl = if state.snap_enabled { "Snap: On" } else { "Snap: Off" };
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: snap_lbl,
                tooltip: "Toggle panel snap-to-grid guides",
                max_w: 68.0,
                selected: state.snap_enabled,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: "inspector.layout.snap",
                role: "button",
                command_id: None,
                disabled_reason: None,
            },
        ) {
            state.snap_enabled = !state.snap_enabled;
            super::set_studio_snap_enabled(ui.ctx(), state.snap_enabled);
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
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Panel arrangement grid template",
                    max_w: 46.0,
                    selected: sel,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: Some("layout.setSpec"),
                    disabled_reason: None,
                },
            ) {
                if spec.items.is_empty() {
                    intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "grid_template": [rows, cols] }));
                } else {
                    let next_items = arrange_layout_grid_items(spec, rows, cols, state.layout_margin_mm);
                    intents.push_engine(
                        "layout.setSpec",
                        json!({
                            "sheet_id": ls.id.as_str(),
                            "spec": { "grid_template": [rows, cols], "items": next_items },
                        }),
                    );
                }
            }
        }
    });

    ui.add_space(4.0);
    widgets::subheader(ui, "Panel Alignment & Badges");
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: tr(locale, "inspector.layout.align_origins"),
            tooltip: "Align adjacent plot origins across panels",
            selected: spec.align_plot_origins,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.layout.align_origins",
            command_id: Some("layout.alignOrigins"),
            disabled_reason: None,
        },
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
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Panel badge lettering",
                    max_w: 54.0,
                    selected: spec.panel_badges == style,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: Some("layout.setSpec"),
                    disabled_reason: None,
                },
            ) {
                intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "panel_badges": slug }));
            }
        }
    });

    render_layout_add_panel_controls(ui, app, ls, ctx_in, meta, intents);

    let has_live_graphs = spec.items.iter().any(|it| matches!(it, LayoutItem::LiveGraph { .. }));
    let can_reorder = ctx_in.enabled && spec.items.len() >= 2;
    let target_panel_idx = ctx_in.selected_panel_index.unwrap_or(0).min(spec.items.len().saturating_sub(1));

    ui.add_space(6.0);
    widgets::subheader(ui, "Panel Z-Order & Scaling");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for (slug, lbl, action_str) in [
            ("front", "Front", "bring_to_front"),
            ("forward", "Forward", "bring_forward"),
            ("backward", "Backward", "send_backward"),
            ("back", "Back", "send_to_back"),
        ] {
            let wid = format!("inspector.layout.zorder.{slug}");
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Reorder panel paint z-order depth",
                    max_w: 56.0,
                    selected: false,
                    enabled: can_reorder,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: Some("layout.reorderPanel"),
                    disabled_reason: (!can_reorder).then_some("Place at least 2 panels to reorder z-order"),
                },
            ) {
                intents
                    .push_engine("layout.reorderPanel", json!({ "sheet_id": ls.id.as_str(), "panel_index": target_panel_idx, "action": action_str }));
            }
        }
    });

    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Equalize Scaling…",
            tooltip: "Match plot scaling across live graph panels",
            selected: false,
            enabled: ctx_in.enabled && has_live_graphs,
            primary: false,
            widget_id: "inspector.layout.equalize_scaling",
            command_id: Some("ui.layout.equalizeScaling.open"),
            disabled_reason: (!has_live_graphs).then_some("Place at least 1 LiveGraph panel first"),
        },
    ) {
        intents.push_ui("ui.layout.equalizeScaling.open", json!({ "sheet_id": ls.id.as_str() }));
    }

    render_layout_media_controls(ui, app, ls, ctx_in, meta, intents);

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "More… Layout Composer",
            tooltip: "Open full Page Layout Composer dialog",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.layout.more_composer",
            command_id: Some("ui.layout.composer.open"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.layout.composer.open", json!({ "sheet_id": ls.id.as_str() }));
    }
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Export Figure…",
            tooltip: "Export layout page to PDF, SVG, or PNG",
            selected: false,
            enabled: ctx_in.enabled,
            primary: true,
            widget_id: "inspector.layout.export_fig",
            command_id: Some("ui.export.figure.open"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.export.figure.open", json!({ "sheet_id": ls.id.as_str(), "dpi": state.layout_dpi }));
    }
}

fn render_layout_add_panel_controls(
    ui: &mut egui::Ui,
    app: &UiApp,
    ls: &LayoutSheet,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    ui.add_space(6.0);
    widgets::subheader(ui, "Add Graph Panel");
    let graphs = &app.document().graph_sheets;
    if graphs.is_empty() {
        ui.label(egui::RichText::new("No graph sheets in document.").small().color(tokens.text_muted));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (idx, gs) in graphs.iter().enumerate() {
            let label = format!("+ {}", gs.title);
            let widget_id = format!("studio.layout.add_panel.{idx}");
            let tip = format!("Add a panel for {} to this layout", gs.title);
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: &label,
                    tooltip: &tip,
                    max_w: 156.0,
                    selected: false,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &widget_id,
                    role: "button",
                    command_id: Some("layout.addPanel"),
                    disabled_reason: None,
                },
            ) {
                intents.push_engine(
                    "layout.addPanel",
                    json!({
                        "sheet_id": ls.id.as_str(),
                        "kind": "graph",
                        "graph_sheet_id": gs.id.as_str(),
                    }),
                );
            }
        }
    });
}

fn render_layout_media_controls(
    ui: &mut egui::Ui,
    app: &UiApp,
    ls: &LayoutSheet,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let sel = ctx_in.selected_panel_index;
    let img_idx = sel
        .filter(|&i| matches!(ls.spec.items.get(i), Some(LayoutItem::EmbeddedImage { .. })))
        .or_else(|| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::EmbeddedImage { .. })));
    let table_idx = sel
        .filter(|&i| matches!(ls.spec.items.get(i), Some(LayoutItem::LiveTable { .. })))
        .or_else(|| ls.spec.items.iter().position(|it| matches!(it, LayoutItem::LiveTable { .. })));
    render_canvas_media_buttons(
        ui,
        app,
        MediaControlsSpec { prefix: "layout", sheet_id: &ls.id, insert_cmd: "layout.insertImage", idx_key: "panel_index", img_idx, table_idx },
        ctx_in,
        meta,
        intents,
    );
}
