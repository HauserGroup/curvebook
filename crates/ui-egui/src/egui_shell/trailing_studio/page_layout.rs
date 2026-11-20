//! Page Layout Composer studio surface.

use curvebook_doc::{LayoutSheet, PageOrientation, PanelBadgeStyle};
use egui::Vec2;
use serde_json::json;

use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, widgets};
use crate::i18n::tr;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
use super::{TrailingStudioContext, TrailingStudioEguiState};

pub(super) fn render_page_layout_composer_studio(
    ui: &mut egui::Ui,
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
        ui.label(egui::RichText::new(tr(locale, "inspector.layout.margins")).size(11.0).color(tokens.text_muted));
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
                command_id: None,
                disabled_reason: None,
            },
        ) {
            state.layout_margin_mm = state.layout_margin_mm.saturating_sub(2).max(2);
        }
        ui.label(egui::RichText::new(format!("{} mm", state.layout_margin_mm)).size(11.0).strong().color(tokens.text_primary));
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
                command_id: None,
                disabled_reason: None,
            },
        ) {
            state.layout_margin_mm = (state.layout_margin_mm + 2).min(40);
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
                intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "grid_template": [rows, cols] }));
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
