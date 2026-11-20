//! Shared widget primitives, localization titles, and collapsed-rail renderer for the Trailing Studio Dock.

use curvebook_doc::SheetId;
use egui::{Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, widgets};
use crate::i18n::{is_rtl_locale, tr};
use crate::inspect::ZLayer;

use super::{StudioContextSurface, TrailingStudioContext};

pub(super) const STUDIO_INNER_PAD: f32 = 8.0;
pub(super) const SCROLL_BAR_RESERVE_PT: f32 = 16.0;
pub(super) const SECTION_BTN_H: f32 = 22.0;

pub(super) struct StudioChipSpec<'a> {
    pub(super) label: &'a str,
    pub(super) tooltip: &'a str,
    pub(super) max_w: f32,
    pub(super) selected: bool,
    pub(super) enabled: bool,
    pub(super) border_override: Option<Color32>,
    pub(super) widget_id: &'a str,
    pub(super) role: &'a str,
    pub(super) command_id: Option<&'a str>,
    pub(super) disabled_reason: Option<&'a str>,
}

pub(super) fn studio_chip(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: &StudioChipSpec<'_>) -> bool {
    let StudioChipSpec { label, tooltip, max_w, selected, enabled, border_override, widget_id, role, command_id, disabled_reason } = *spec;
    let font_id = egui::FontId::proportional(11.5);
    let tc = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let max_text_w = (max_w - 14.0).max(12.0);
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id.clone(), tc);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = max_text_w;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let w = (galley.size().x + 14.0).clamp(24.0_f32.min(max_w), max_w.max(24.0));
    let rect = ui.allocate_space(Vec2::new(w, SECTION_BTN_H)).1;
    let visible = rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if !visible.is_positive() || visible.width() < 4.0 || visible.height() < 4.0 {
        return false;
    }

    let resp =
        ui.interact(visible, ui.make_persistent_id(("egui_shell.studio_chip", widget_id)), if enabled { Sense::click() } else { Sense::hover() });
    let (galley, ellipsized) = if visible.width() < rect.width() - 0.5 {
        let mut vis_job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, tc);
        vis_job.wrap = egui::text::TextWrapping {
            max_width: (visible.width() - 14.0).max(4.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let vg = ui.fonts_mut(|f| f.layout_job(vis_job));
        let ve = vg.elided;
        (vg, ve)
    } else {
        let e = galley.elided;
        (galley, e)
    };

    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let bc = if !enabled {
        tokens.divider
    } else if let Some(ov) = border_override {
        ov
    } else if selected || resp.hovered() {
        tokens.accent
    } else {
        tokens.button_border
    };
    let sw = if border_override.is_some() { 1.5 } else { 1.0 };

    let painter = ui.painter().with_clip_rect(visible);
    painter.rect(rect, 4.0, bg, Stroke::new(sw, bc), StrokeKind::Inside);
    if galley.size().x <= visible.width() - 8.0 || ellipsized {
        let tp = Pos2::new(visible.min.x + 7.0, rect.center().y - galley.size().y * 0.5);
        painter.galley(tp, galley, tc);
    }

    let acc_name = label.to_owned();
    let ak_role = match role {
        "tab" => egui::Role::Tab,
        "text" => egui::Role::Label,
        _ => egui::Role::Button,
    };
    resp.widget_info(move || egui::WidgetInfo::labeled(ak_role, enabled, acc_name.clone()));

    let resp = if !enabled {
        if let Some(r) = disabled_reason { resp.on_disabled_hover_text(r) } else { resp }
    } else if ellipsized {
        resp.on_hover_text(label)
    } else if !tooltip.is_empty() && tooltip != label {
        resp.on_hover_text(tooltip)
    } else {
        resp
    };

    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role,
            label,
            command_id,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some(disabled_reason).flatten(),
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    resp.clicked() && enabled
}

pub(super) struct StudioButtonSpec<'a> {
    pub(super) label: &'a str,
    pub(super) tooltip: &'a str,
    pub(super) selected: bool,
    pub(super) enabled: bool,
    pub(super) primary: bool,
    pub(super) widget_id: &'a str,
    pub(super) command_id: Option<&'a str>,
    pub(super) disabled_reason: Option<&'a str>,
}

pub(super) fn studio_full_button(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: &StudioButtonSpec<'_>) -> bool {
    let StudioButtonSpec { label, tooltip, selected, enabled, primary, widget_id, command_id, disabled_reason } = *spec;
    let w = ui.available_width().max(48.0);
    let font_id = egui::FontId::proportional(11.5);
    let tc = if !enabled {
        tokens.text_disabled
    } else if primary {
        tokens.paper_white
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let rect = ui.allocate_space(Vec2::new(w, 24.0)).1;
    let visible = rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if !visible.is_positive() || visible.width() < 4.0 || visible.height() < 4.0 {
        return false;
    }

    let mut job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font_id, tc);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = (visible.width() - 16.0).max(8.0);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let ellipsized = galley.elided;
    let resp = ui.interact(
        visible,
        ui.make_persistent_id(("egui_shell.studio_full_button", widget_id)),
        if enabled { Sense::click() } else { Sense::hover() },
    );

    let bg = if !enabled {
        tokens.panel_bg
    } else if primary {
        if resp.hovered() { tokens.accent_strong } else { tokens.accent }
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let bc = if !enabled {
        tokens.divider
    } else if primary || selected || resp.hovered() {
        tokens.accent
    } else {
        tokens.button_border
    };

    let painter = ui.painter().with_clip_rect(visible);
    painter.rect(rect, 5.0, bg, Stroke::new(1.0, bc), StrokeKind::Inside);
    if galley.size().x <= visible.width() - 8.0 || ellipsized {
        let tp = Pos2::new(visible.min.x + ((visible.width() - galley.size().x) * 0.5).max(8.0), rect.center().y - galley.size().y * 0.5);
        painter.galley(tp, galley, tc);
    }

    let acc_name = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, acc_name.clone()));

    let resp = if !enabled {
        if let Some(r) = disabled_reason { resp.on_disabled_hover_text(r) } else { resp }
    } else if ellipsized {
        resp.on_hover_text(label)
    } else if !tooltip.is_empty() && tooltip != label {
        resp.on_hover_text(tooltip)
    } else {
        resp
    };

    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label,
            command_id,
            value: None,
            enabled,
            disabled_reason: (!enabled).then_some(disabled_reason).flatten(),
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    resp.clicked() && enabled
}

pub(super) fn surface_localized_title(locale: &str, surface: StudioContextSurface) -> &'static str {
    let key = match surface {
        StudioContextSurface::MultipleVariablesFormula => "mv.editor_btn",
        StudioContextSurface::MetadataHooks => "nav.info",
        StudioContextSurface::ResultsInterpret => "results.interpret",
        StudioContextSurface::DataPointDrillBack => "inspector.section.drill_back",
        _ => "",
    };
    if !key.is_empty() {
        let translated = tr(locale, key);
        if translated != key && !translated.is_empty() {
            return translated;
        }
    }
    match surface {
        StudioContextSurface::MultipleVariablesFormula => "MV Formula Studio",
        StudioContextSurface::TableSelection => "Table & Selection",
        StudioContextSurface::MetadataHooks => "Metadata & Hooks",
        StudioContextSurface::ResultsInterpret => "Interpret & Diagnostics",
        StudioContextSurface::GraphInspector => "Graph Inspector",
        StudioContextSurface::DataPointDrillBack => "Data Point Drill-Back",
        StudioContextSurface::PageLayoutComposer => "Page Layout Composer",
    }
}

fn render_passive_collapsed_rail_badge(ui: &mut egui::Ui, tokens: &Tokens, glyph: &str) {
    let font_id = egui::FontId::proportional(11.5);
    let mut job = egui::text::LayoutJob::simple_singleline(glyph.to_owned(), font_id, tokens.text_disabled);
    job.wrap = egui::text::TextWrapping { max_rows: 1, break_anywhere: true, overflow_character: Some('…'), ..Default::default() };
    job.wrap.max_width = 12.0;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let w = (galley.size().x + 14.0).clamp(24.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, SECTION_BTN_H), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(rect, 4.0, tokens.panel_bg, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        let tp = Pos2::new(rect.min.x + 7.0, rect.center().y - galley.size().y * 0.5);
        let painter = ui.painter().with_clip_rect(rect.shrink2(Vec2::new(4.0, 1.0)));
        painter.galley(tp, galley, tokens.text_disabled);
    }
}

pub(super) fn render_collapsed_rail(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    surface: StudioContextSurface,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = &tokens.locale;
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let expand_resp = studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: if is_rtl_locale(locale) { "»" } else { "«" },
                tooltip: "Expand Studio (Cmd/Ctrl+Shift+I)",
                max_w: 26.0,
                selected: false,
                enabled,
                border_override: None,
                widget_id: "inspector.expand_rail",
                role: "button",
                command_id: Some("ui.inspector.toggle"),
                disabled_reason: (!enabled).then_some("modal dialog is active"),
            },
        );
        if expand_resp && enabled {
            intents.push_toggle_trailing_collapsed_rail();
        }

        widgets::divider(ui);

        let all_surfaces = [
            StudioContextSurface::MultipleVariablesFormula,
            StudioContextSurface::TableSelection,
            StudioContextSurface::MetadataHooks,
            StudioContextSurface::ResultsInterpret,
            StudioContextSurface::GraphInspector,
            StudioContextSurface::DataPointDrillBack,
            StudioContextSurface::PageLayoutComposer,
        ];
        for ctx_surface in all_surfaces {
            if !enabled {
                // While a modal dialog scrim covers the workspace, omit behind-backdrop AccessKit `Tab`
                // nodes so foreground modal scrollbars cannot collide with occluded rail items.
                render_passive_collapsed_rail_badge(ui, tokens, ctx_surface.rail_glyph());
                continue;
            }
            let active = ctx_surface == surface;
            let title = surface_localized_title(locale, ctx_surface);
            let wid = format!("inspector.rail.{}", ctx_surface.slug());
            let r = studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: ctx_surface.rail_glyph(),
                    tooltip: title,
                    max_w: 26.0,
                    selected: active,
                    enabled,
                    border_override: active.then_some(tokens.accent),
                    widget_id: &wid,
                    role: "tab",
                    command_id: None,
                    disabled_reason: None,
                },
            );
            if r {
                intents.push_toggle_trailing_collapsed_rail();
            }
        }
    });
}

pub(super) struct MediaControlsSpec<'a> {
    pub(super) prefix: &'a str,
    pub(super) sheet_id: &'a SheetId,
    pub(super) insert_cmd: &'a str,
    pub(super) idx_key: &'a str,
    pub(super) img_idx: Option<usize>,
    pub(super) table_idx: Option<usize>,
}

pub(super) fn render_canvas_media_buttons(
    ui: &mut egui::Ui,
    app: &UiApp,
    spec: MediaControlsSpec<'_>,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = tokens.locale;
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: tr(locale, "r13.canvas.grid_overlay"),
            tooltip: "Toggle non-exported visual alignment grid",
            selected: app.canvas_dialogs.grid_overlay_visible,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: &format!("inspector.{}.grid_overlay", spec.prefix),
            command_id: Some("ui.gridOverlay.toggle"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.gridOverlay.toggle", json!({}));
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: tr(locale, "r13.canvas.insert_image"),
                tooltip: "Insert PNG, JPEG, or TIFF image from file",
                max_w: 116.0,
                selected: false,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: &format!("inspector.{}.insert_image", spec.prefix),
                role: "button",
                command_id: Some(spec.insert_cmd),
                disabled_reason: None,
            },
        ) {
            intents.push_engine(spec.insert_cmd, json!({}));
        }
        if studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: tr(locale, "r13.canvas.paste_image"),
                tooltip: "Paste image bytes from clipboard",
                max_w: 110.0,
                selected: false,
                enabled: ctx_in.enabled,
                border_override: None,
                widget_id: &format!("inspector.{}.paste_image", spec.prefix),
                role: "button",
                command_id: Some("edit.pasteImage"),
                disabled_reason: None,
            },
        ) {
            intents.push_paste_image_from_clipboard(spec.sheet_id.clone(), None);
        }
    });

    if let Some(idx) = spec.img_idx {
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: tr(locale, "r13.canvas.format_image"),
                tooltip: "Edit image placement, rotation, 4-edge crop, and border",
                selected: false,
                enabled: ctx_in.enabled,
                primary: false,
                widget_id: &format!("inspector.{}.format_image", spec.prefix),
                command_id: Some("ui.image.format.open"),
                disabled_reason: None,
            },
        ) {
            intents.push_ui("ui.image.format.open", json!({ "sheet_id": spec.sheet_id.as_str(), spec.idx_key: idx }));
        }
    }

    if let Some(idx) = spec.table_idx {
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: tr(locale, "r13.canvas.format_embedded_table"),
                tooltip: "Format embedded result table borders, gridlines, font, and fill",
                selected: false,
                enabled: ctx_in.enabled,
                primary: false,
                widget_id: &format!("inspector.{}.format_embedded_table", spec.prefix),
                command_id: Some("ui.embeddedTable.format.open"),
                disabled_reason: None,
            },
        ) {
            intents.push_ui("ui.embeddedTable.format.open", json!({ "sheet_id": spec.sheet_id.as_str(), spec.idx_key: idx }));
        }
    }
}
