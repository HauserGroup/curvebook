//! Linked-region provenance strips and read-only guard popovers for `Column` and multi-archetype
//! data grids.

use curvebook_doc::{DataSheet, LinkedDataRegion};
use egui::{Align, CornerRadius, FontId, Id, Key, Modifiers, Order, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::inspect::ZLayer;
use crate::views::grid_core::grid_tr;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use super::shared::{CellTextStyle, GridRenderCtx, link_border_color32, record_meta_if_enabled, render_ellipsized_text};
use super::{ColumnGridEguiState, LinkedRegionPopover};

pub(super) fn open_link_popover(state: &mut ColumnGridEguiState, reg: &LinkedDataRegion, locale: &str) {
    let prompt = grid_tr(locale, "w5.grid_core.linked_read_only_prompt", "Linked region is read-only. Go to source or unlink to edit.");
    let msg = if reg.provenance_summary.is_empty() { prompt.to_owned() } else { format!("{}\n{}", reg.provenance_summary, prompt) };
    state.active_link_popover =
        Some(LinkedRegionPopover { region_id: reg.id.clone(), upstream_sheet_id: reg.spec.source.upstream_sheet_id().cloned(), message: msg });
}

pub(super) fn render_link_provenance_strip(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    strip_rect: Rect,
) {
    let tokens = rctx.tokens;
    ui.scope_builder(UiBuilder::new().max_rect(strip_rect), |ui| {
        ui.set_clip_rect(strip_rect);
        ui.painter().rect_filled(strip_rect, CornerRadius::ZERO, tokens.panel_bg);
        ui.painter().rect_stroke(strip_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let badge_lbl = grid_tr(rctx.locale, "w5.grid_core.linked_badge", "Linked");
        let mut cursor_x = strip_rect.left() + 6.0;
        let blocked = rctx.input_blocked(state);
        for (idx, reg) in rctx.ds.linked_regions.iter().enumerate() {
            let rem = strip_rect.right() - cursor_x;
            if rem < 72.0 {
                break;
            }
            let chip_w = 240.0_f32.min((rem - 6.0).max(0.0));
            let chip_rect = Rect::from_min_size(pos2(cursor_x, strip_rect.top() + 3.0), vec2(chip_w, (strip_rect.height() - 6.0).max(16.0)));
            let color = link_border_color32(reg.border_color(), tokens);
            let label = if reg.provenance_summary.is_empty() {
                format!("{badge_lbl} #{}", idx + 1)
            } else {
                format!("{badge_lbl}: {}", reg.provenance_summary)
            };
            let sense = if blocked { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(chip_rect, rctx.sheet_id_base.with(("link_badge", idx)), sense);
            let fill = if resp.hovered() && !blocked { color.gamma_multiply(0.24) } else { color.gamma_multiply(0.12) };
            ui.painter().rect_filled(chip_rect, CornerRadius::same(3), fill);
            ui.painter().rect_stroke(chip_rect, CornerRadius::same(3), Stroke::new(2.0, color), StrokeKind::Inside);
            let (elided, _) =
                render_ellipsized_text(ui, chip_rect, &label, CellTextStyle::plain(FontId::proportional(11.5), tokens.text_primary, Align::LEFT));
            let acc_label = label.clone();
            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_label.clone()));
            if elided {
                let _ = resp.clone().on_hover_text(&label);
            }
            if resp.clicked() && !blocked {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
            }
            let wid = format!("grid.link_badge.{idx}");
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "button",
                    label: &label,
                    command_id: None,
                    value: Some(json!({ "region_id": reg.id })),
                    enabled: !blocked,
                    disabled_reason: blocked.then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: elided,
                    full_label: elided.then_some(label.as_str()),
                },
            );
            cursor_x += chip_w + 6.0;
        }
    });
    let _ = intents;
}

pub(super) fn render_linked_region_guard_popover(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let Some(pop) = state.active_link_popover.clone() else {
        return false;
    };
    let tokens = rctx.tokens;
    let viewport = ui.ctx().content_rect();
    let backdrop_scrim = tokens.scrim.gamma_multiply(0.35);

    let mut outside_click_blocked = false;
    let backdrop_resp = egui::Area::new(Id::unique(("grid_link_guard_backdrop", rctx.ds.id.as_str())))
        .order(Order::Middle)
        .fixed_pos(viewport.min)
        .show(ui.ctx(), |ui| {
            let (rect, resp) = ui.allocate_exact_size(viewport.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, backdrop_scrim);
            resp
        })
        .inner;

    if backdrop_resp.clicked() {
        outside_click_blocked = true;
    }

    if ui.input(|i| i.key_pressed(Key::Escape)) {
        state.active_link_popover = None;
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        ui.ctx().request_repaint();
        return outside_click_blocked;
    }

    let card_w = 400.0_f32.min((rctx.geom.available_rect.width() - 24.0).max(240.0));
    let card_pos = pos2(
        (rctx.geom.available_rect.center().x - card_w * 0.5).max(viewport.left() + 8.0),
        (rctx.geom.available_rect.top() + 48.0).min(viewport.bottom() - 150.0),
    );
    let title = grid_tr(rctx.locale, "w5.grid_core.linked_region_title", "Linked Data Region");
    let mut should_close = false;

    let card_area = egui::Area::new(Id::unique(("grid_link_guard_card", rctx.ds.id.as_str()))).order(Order::Foreground).fixed_pos(card_pos).show(
        ui.ctx(),
        |ui| {
            egui::Frame::popup(ui.style())
                .fill(tokens.surface_bg)
                .stroke(Stroke::new(2.0, tokens.link_blue))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(card_w);
                    ui.strong(title);
                    ui.add_space(4.0);
                    ui.colored_label(tokens.link_red, &pop.message);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let goto_lbl = grid_tr(rctx.locale, "w5.grid_core.go_to_source", "Go To Source");
                        let has_upstream = pop.upstream_sheet_id.is_some();
                        let goto_resp = ui.add_enabled(has_upstream, egui::Button::new(goto_lbl));
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &goto_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.goto",
                                role: "button",
                                label: goto_lbl,
                                command_id: Some("sheet.select"),
                                value: None,
                                enabled: has_upstream,
                                disabled_reason: (!has_upstream).then_some("External source"),
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if goto_resp.clicked()
                            && let Some(target_sid) = &pop.upstream_sheet_id
                        {
                            intents.push_engine("sheet.select", json!({ "sheet_id": target_sid.as_str(), "mode": "navigator" }));
                            should_close = true;
                        }

                        let unlink_lbl = grid_tr(rctx.locale, "w5.grid_core.unlink", "Unlink");
                        let unlink_resp = ui.add_enabled(!rctx.ds.frozen, egui::Button::new(unlink_lbl));
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &unlink_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.unlink",
                                role: "button",
                                label: unlink_lbl,
                                command_id: Some("table.clearCells"),
                                value: None,
                                enabled: !rctx.ds.frozen,
                                disabled_reason: rctx.ds.frozen.then_some("Frozen"),
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if unlink_resp.clicked() && !rctx.ds.frozen {
                            intents.push_engine("table.clearCells", json!({ "sheet_id": rctx.ds.id.as_str(), "cells": [], "unlink": true }));
                            should_close = true;
                        }

                        let close_lbl = grid_tr(rctx.locale, "w5.grid_core.close", "Close");
                        let close_resp = ui.button(close_lbl);
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &close_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.close",
                                role: "button",
                                label: close_lbl,
                                command_id: None,
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if close_resp.clicked() {
                            should_close = true;
                        }
                    });
                });
        },
    );

    if state.record_inspect_meta && !ui.ctx().will_discard() {
        record_meta_if_enabled(
            ui,
            meta,
            true,
            &card_area.response,
            FrameInspectEntryInput {
                widget_id: "grid.link_popover.panel",
                role: "dialog",
                label: title,
                command_id: None,
                value: Some(json!({ "region_id": pop.region_id })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::ModalDialog,
                ellipsized: false,
                full_label: None,
            },
        );
    }

    if should_close {
        state.active_link_popover = None;
        ui.ctx().request_repaint();
    }

    outside_click_blocked
}

pub(super) fn render_multi_link_strip(
    ui: &mut Ui,
    app: &UiApp,
    ds: &DataSheet,
    strip_rect: Rect,
    state: &mut ColumnGridEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) {
    ui.painter().rect_filled(strip_rect, 0.0, tokens.panel_bg);
    ui.painter().line_segment([pos2(strip_rect.min.x, strip_rect.max.y), pos2(strip_rect.max.x, strip_rect.max.y)], Stroke::new(1.0, tokens.divider));
    let mut cursor_x = strip_rect.min.x + 6.0;
    for (idx, reg) in ds.linked_regions.iter().enumerate() {
        let color = link_border_color32(reg.spec.source.border_color(), tokens);
        let badge_txt = format!("Link {}: {}", idx + 1, reg.provenance_summary);
        #[allow(clippy::cast_precision_loss)]
        let pill_w = (badge_txt.len() as f32 * 6.5 + 18.0).min(240.0);
        if cursor_x + pill_w > strip_rect.max.x - 4.0 {
            break;
        }
        let r = Rect::from_min_size(pos2(cursor_x, strip_rect.min.y + 2.0), vec2(pill_w, strip_rect.height() - 4.0));
        let wid = format!("grid.link_badge.{idx}");
        let resp = ui.interact(r, ui.make_persistent_id(&wid), Sense::click());
        ui.painter().rect_filled(r, 3.0, color.linear_multiply(0.16));
        ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, color), StrokeKind::Inside);
        render_ellipsized_text(
            ui,
            r.shrink2(vec2(5.0, 1.0)),
            &badge_txt,
            CellTextStyle::plain(FontId::proportional(11.0), tokens.text_primary, Align::LEFT),
        );
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &badge_txt,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if resp.clicked() {
            open_link_popover(state, reg, &app.locale);
        }
        cursor_x += pill_w + 6.0;
    }
}

pub(super) fn render_multi_link_guard_popover(
    ui: &mut Ui,
    ds: &DataSheet,
    state: &mut ColumnGridEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    bounds: Rect,
) {
    let Some(pop) = state.active_link_popover.clone() else { return };
    let pop_w = (bounds.width() - 24.0).clamp(240.0, 360.0);
    let pos = pos2(bounds.center().x - pop_w * 0.5, bounds.min.y + 32.0);
    egui::Area::new(Id::unique(("multi_link_guard", ds.id.as_str()))).fixed_pos(pos).order(Order::Foreground).show(ui.ctx(), |pui| {
        egui::Frame::popup(pui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.5, tokens.link_blue)).show(pui, |box_ui| {
            box_ui.set_min_width(pop_w);
            box_ui.label(egui::RichText::new(&pop.message).color(tokens.text_primary));
            box_ui.add_space(6.0);
            box_ui.horizontal(|row| {
                if let Some(up) = &pop.upstream_sheet_id {
                    let go = row.button("Go To Source");
                    record_meta_if_enabled(
                        row,
                        meta,
                        true,
                        &go,
                        FrameInspectEntryInput {
                            widget_id: "grid.link_guard.go_to_source",
                            role: "button",
                            label: "Go To Source",
                            command_id: Some("sheet.select"),
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::ModalDialog,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if go.clicked() {
                        intents.push_engine("sheet.select", json!({ "sheet_id": up.as_str() }));
                        state.active_link_popover = None;
                    }
                }
                let ul = row.button("Unlink Region");
                record_meta_if_enabled(
                    row,
                    meta,
                    true,
                    &ul,
                    FrameInspectEntryInput {
                        widget_id: "grid.link_guard.unlink",
                        role: "button",
                        label: "Unlink Region",
                        command_id: Some("table.unlinkRegion"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if ul.clicked() {
                    intents.push_engine("table.unlinkRegion", json!({ "sheet_id": ds.id.as_str(), "link_id": pop.region_id }));
                    state.active_link_popover = None;
                }
                let dis = row.button("Dismiss");
                record_meta_if_enabled(
                    row,
                    meta,
                    true,
                    &dis,
                    FrameInspectEntryInput {
                        widget_id: "grid.link_guard.dismiss",
                        role: "button",
                        label: "Dismiss",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if dis.clicked() {
                    state.active_link_popover = None;
                }
            });
        });
    });
}
