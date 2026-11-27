//! Shared widget primitives, localization titles, and collapsed-rail renderer for the Trailing Studio Dock.

use curvebook_doc::SheetId;
use egui::{Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, widgets};
use crate::i18n::{is_rtl_locale, tr};
use crate::inspect::ZLayer;

use super::{StudioContextSurface, TrailingStudioContext};

pub(super) const STUDIO_INNER_PAD: f32 = 8.0;
pub(super) const SCROLL_BAR_RESERVE_PT: f32 = 16.0;
pub(super) const SECTION_BTN_H: f32 = 24.0;

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
    studio_chip_with_shortcut(ui, tokens, meta, spec, None)
}

pub(super) fn studio_chip_with_shortcut(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: &StudioChipSpec<'_>,
    shortcut: Option<&str>,
) -> bool {
    let ak_role = match spec.role {
        "tab" => egui::Role::Tab,
        "text" => egui::Role::Label,
        _ => egui::Role::Button,
    };
    let tip_override = shortcut.map(|sc| {
        let mut tip = crate::egui_shell::ui_kit::TooltipSpec::new(spec.label).with_shortcut(sc);
        if !spec.tooltip.trim().is_empty() {
            if spec.label.chars().count() <= 2 {
                tip.title = spec.tooltip;
            } else {
                tip = tip.with_summary(spec.tooltip);
            }
        }
        if !spec.enabled
            && let Some(reason) = spec.disabled_reason
        {
            tip = tip.with_disabled_reason(reason);
        }
        tip
    });
    let (_, clicked) = crate::egui_shell::ui_kit::styled_button_with_tooltip(
        ui,
        tokens,
        meta,
        crate::egui_shell::ui_kit::StyledButtonSpec {
            widget_id: spec.widget_id,
            role: spec.role,
            a11y_role: ak_role,
            a11y_selectable: false,
            label: spec.label,
            tooltip: (!spec.tooltip.is_empty()).then_some(spec.tooltip),
            command_id: spec.command_id,
            width: crate::egui_shell::ui_kit::ButtonWidth::FitText { min_w: 24.0_f32.min(spec.max_w), max_w: spec.max_w.max(24.0), pad_x: 14.0 },
            height: SECTION_BTN_H,
            font_size: 11.5,
            radius: tokens.metrics.radius_md,
            text_align: crate::egui_shell::ui_kit::ButtonTextAlign::Left(7.0),
            break_anywhere: true,
            primary: false,
            selected: spec.selected,
            enabled: spec.enabled,
            accent_text_when_selected: false,
            border_override: spec.border_override,
            value: None,
            disabled_reason: spec.disabled_reason,
            z_layer: ZLayer::Workspace,
            skip_offscreen: false,
            clip_interact_to_viewport: true,
            tooltip_mode: crate::egui_shell::ui_kit::TooltipMode::Standard,
        },
        tip_override,
    );
    clicked
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
    studio_sized_button(ui, tokens, meta, ui.available_width().max(48.0), spec)
}

pub(super) fn studio_full_button_with_shortcut(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: &StudioButtonSpec<'_>,
    shortcut: Option<&str>,
) -> bool {
    studio_sized_button_with_shortcut(ui, tokens, meta, ui.available_width().max(48.0), spec, shortcut)
}

pub(super) fn studio_sized_button(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, width: f32, spec: &StudioButtonSpec<'_>) -> bool {
    studio_sized_button_with_shortcut(ui, tokens, meta, width, spec, None)
}

pub(super) fn studio_sized_button_with_shortcut(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    width: f32,
    spec: &StudioButtonSpec<'_>,
    shortcut: Option<&str>,
) -> bool {
    let tip_override = shortcut.map(|sc| {
        let mut tip = crate::egui_shell::ui_kit::TooltipSpec::new(spec.label).with_shortcut(sc);
        if !spec.tooltip.trim().is_empty() && !spec.tooltip.trim().eq_ignore_ascii_case(spec.label.trim()) {
            tip = tip.with_summary(spec.tooltip);
        }
        if !spec.enabled
            && let Some(reason) = spec.disabled_reason
        {
            tip = tip.with_disabled_reason(reason);
        }
        tip
    });
    let (_, clicked) = crate::egui_shell::ui_kit::styled_button_with_tooltip(
        ui,
        tokens,
        meta,
        crate::egui_shell::ui_kit::StyledButtonSpec {
            widget_id: spec.widget_id,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: false,
            label: spec.label,
            tooltip: (!spec.tooltip.is_empty()).then_some(spec.tooltip),
            command_id: spec.command_id,
            width: crate::egui_shell::ui_kit::ButtonWidth::Fixed(width.max(48.0)),
            height: SECTION_BTN_H,
            font_size: 11.5,
            radius: tokens.metrics.radius_md,
            text_align: crate::egui_shell::ui_kit::ButtonTextAlign::Center,
            break_anywhere: true,
            primary: spec.primary,
            selected: spec.selected,
            enabled: spec.enabled,
            accent_text_when_selected: false,
            border_override: None,
            value: None,
            disabled_reason: spec.disabled_reason,
            z_layer: ZLayer::Workspace,
            skip_offscreen: false,
            clip_interact_to_viewport: true,
            tooltip_mode: crate::egui_shell::ui_kit::TooltipMode::Standard,
        },
        tip_override,
    );
    clicked
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
    app: &UiApp,
    tokens: &Tokens,
    surface: StudioContextSurface,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = &tokens.locale;
    let expand_sc = super::studio_toggle_shortcut(app);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        let expand_resp = studio_chip_with_shortcut(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: if is_rtl_locale(locale) { "»" } else { "«" },
                tooltip: "Expand Studio",
                max_w: 26.0,
                selected: false,
                enabled,
                border_override: None,
                widget_id: "inspector.expand_rail",
                role: "button",
                command_id: Some("ui.inspector.toggle"),
                disabled_reason: (!enabled).then_some("modal dialog is active"),
            },
            expand_sc.as_deref(),
        );
        if expand_resp && enabled {
            intents.push(UiIntent::ToggleTrailingCollapsedRail);
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
                intents.push(UiIntent::ToggleTrailingCollapsedRail);
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
            intents.push_engine(spec.insert_cmd, json!({ "sheet_id": spec.sheet_id.as_str() }));
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
            intents.push(UiIntent::PasteImageFromClipboard { target_sheet_id: spec.sheet_id.clone(), anchor_pt: None });
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
