//! Unified linked-region provenance strip and read-only guard popover for all 8 data grid archetypes.

use curvebook_doc::LinkedDataRegion;
use egui::{Align, CornerRadius, FontId, Id, Key, Modifiers, Order, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::grid_view::grid_tr;
use crate::inspect::ZLayer;

use super::super::FrameInspectEntryInput;
use super::super::ui_kit::{LinkButtonSpec, TooltipMode, TooltipSpec, link_button};
use super::shared::{
    CellTextStyle, MultiGridCtx, MultiGridFrameState, is_linked_region_broken, link_border_color32, paint_broken_link_glyph,
    paint_dashed_rect_stroke, record_meta_if_enabled, render_ellipsized_text,
};
use super::{ColumnGridEguiState, LinkedRegionPopover};

pub(super) fn open_link_popover(state: &mut ColumnGridEguiState, reg: &LinkedDataRegion, locale: &str) {
    let prompt = grid_tr(locale, "w5.grid_core.linked_read_only_prompt", "Linked region is read-only. Go to source or unlink to edit.");
    let msg = if reg.provenance_summary.is_empty() { prompt.to_owned() } else { format!("{}\n{}", reg.provenance_summary, prompt) };
    state.active_link_popover =
        Some(LinkedRegionPopover { region_id: reg.id.clone(), upstream_sheet_id: reg.spec.source.upstream_sheet_id().cloned(), message: msg });
}

pub(super) fn render_link_provenance_strip(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, strip_rect: Rect) {
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    ui.scope_builder(UiBuilder::new().max_rect(strip_rect), |ui| {
        ui.set_clip_rect(strip_rect);
        ui.painter().rect_filled(strip_rect, CornerRadius::ZERO, tokens.panel_bg);
        ui.painter().rect_stroke(strip_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let badge_lbl = grid_tr(mctx.locale, "w5.grid_core.linked_badge", "Linked");
        let mut cursor_x = strip_rect.left() + 6.0;
        for (idx, reg) in mctx.ds.linked_regions.iter().enumerate() {
            let rem = strip_rect.right() - cursor_x;
            if rem < 72.0 {
                break;
            }
            let chip_w = 240.0_f32.min((rem - 6.0).max(0.0));
            let chip_rect = Rect::from_min_size(pos2(cursor_x, strip_rect.top() + 3.0), vec2(chip_w, (strip_rect.height() - 6.0).max(16.0)));
            let broken = is_linked_region_broken(mctx.app.document(), reg);
            let color = if broken { tokens.link_red } else { link_border_color32(reg.border_color(), tokens) };
            let raw_label = if reg.provenance_summary.is_empty() {
                format!("{badge_lbl} #{}", idx + 1)
            } else {
                format!("{badge_lbl}: {}", reg.provenance_summary)
            };
            let label = if broken { format!("Broken {raw_label}") } else { raw_label };
            let wid = format!("grid.link_badge.{idx}");
            let sense = if blocked { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(chip_rect, ui.make_persistent_id((&wid, mctx.ds.id.as_str())), sense);
            let fill = if resp.hovered() && !blocked { color.gamma_multiply(0.24) } else { color.gamma_multiply(0.12) };
            ui.painter().rect_filled(chip_rect, CornerRadius::same(tokens.metrics.radius_sm), fill);
            if broken {
                paint_dashed_rect_stroke(ui.painter(), chip_rect, Stroke::new(2.0, color));
                let glyph_r = Rect::from_min_size(pos2(chip_rect.left() + 4.0, chip_rect.center().y - 6.0), vec2(12.0, 12.0));
                paint_broken_link_glyph(ui.painter(), glyph_r, color);
            } else {
                ui.painter().rect_stroke(chip_rect, CornerRadius::same(tokens.metrics.radius_sm), Stroke::new(2.0, color), StrokeKind::Inside);
            }
            let text_rect = if broken { Rect::from_min_max(pos2(chip_rect.left() + 18.0, chip_rect.top()), chip_rect.max) } else { chip_rect };
            let (elided, _) = render_ellipsized_text(
                ui,
                text_rect,
                &label,
                CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_primary, Align::LEFT),
            );
            let acc_label = label.clone();
            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_label.clone()));
            if elided {
                let _ = resp.clone().on_hover_text(&label);
            }
            if resp.clicked() && !blocked {
                open_link_popover(fstate.state, reg, mctx.locale);
                ui.ctx().request_repaint();
            }
            record_meta_if_enabled(
                ui,
                fstate.meta,
                fstate.state.record_inspect_meta,
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
}

pub(super) fn render_linked_region_guard_popover(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) -> bool {
    let Some(pop) = fstate.state.active_link_popover.clone() else {
        return false;
    };
    let tokens = mctx.tokens;
    let viewport = ui.ctx().content_rect();
    let backdrop_scrim = tokens.scrim.gamma_multiply(0.35);

    let mut outside_click_blocked = false;
    let backdrop_resp = egui::Area::new(Id::unique(("grid_link_guard_backdrop", mctx.ds.id.as_str())))
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
        fstate.state.active_link_popover = None;
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        ui.ctx().request_repaint();
        return outside_click_blocked;
    }

    let card_w = 400.0_f32.min((mctx.geom.available_rect.width() - 24.0).max(240.0));
    let card_pos = pos2(
        (mctx.geom.available_rect.center().x - card_w * 0.5).max(viewport.left() + 8.0),
        (mctx.geom.available_rect.top() + 48.0).min(viewport.bottom() - 150.0),
    );
    let title = grid_tr(mctx.locale, "w5.grid_core.linked_region_title", "Linked Data Region");
    let mut should_close = false;

    let card_area = egui::Area::new(Id::unique(("grid_link_guard_card", mctx.ds.id.as_str()))).order(Order::Foreground).fixed_pos(card_pos).show(
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
                        let goto_lbl = grid_tr(mctx.locale, "w5.grid_core.go_to_source", "Go To Source");
                        let has_upstream = pop.upstream_sheet_id.is_some();
                        let goto_resp = link_button(
                            ui,
                            tokens,
                            fstate.meta,
                            LinkButtonSpec {
                                font_size: tokens.metrics.font_ui_body_pt,
                                command_id: Some("sheet.select"),
                                enabled: has_upstream,
                                disabled_reason: (!has_upstream).then_some("External source"),
                                tooltip: has_upstream.then(|| TooltipSpec::new("Go To Source").with_summary("Jump to the upstream source sheet")),
                                record_in_meta: fstate.state.record_inspect_meta,
                                tooltip_mode: TooltipMode::ModalAlwaysOpen,
                                pad_x: 4.0,
                                ..LinkButtonSpec::new("grid.link_popover.goto", goto_lbl, ZLayer::ModalDialog)
                            },
                        );
                        if goto_resp.clicked()
                            && let Some(target_sid) = &pop.upstream_sheet_id
                        {
                            fstate.intents.push_engine("sheet.select", json!({ "sheet_id": target_sid.as_str(), "mode": "navigator" }));
                            should_close = true;
                        }

                        let unlink_lbl = grid_tr(mctx.locale, "w5.grid_core.unlink", "Unlink");
                        let unlink_resp = ui.add_enabled(!mctx.ds.frozen, egui::Button::new(unlink_lbl));
                        record_meta_if_enabled(
                            ui,
                            fstate.meta,
                            fstate.state.record_inspect_meta,
                            &unlink_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.unlink",
                                role: "button",
                                label: unlink_lbl,
                                command_id: Some("table.clearCells"),
                                value: None,
                                enabled: !mctx.ds.frozen,
                                disabled_reason: mctx.ds.frozen.then_some("Frozen"),
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if unlink_resp.clicked() && !mctx.ds.frozen {
                            fstate.intents.push_engine(
                                "table.clearCells",
                                json!({ "sheet_id": mctx.ds.id.as_str(), "cells": [], "unlink": true, "region_id": pop.region_id }),
                            );
                            should_close = true;
                        }

                        let close_lbl = grid_tr(mctx.locale, "w5.grid_core.close", "Close");
                        let close_resp = ui.button(close_lbl);
                        record_meta_if_enabled(
                            ui,
                            fstate.meta,
                            fstate.state.record_inspect_meta,
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

    if fstate.state.record_inspect_meta && !ui.ctx().will_discard() {
        record_meta_if_enabled(
            ui,
            fstate.meta,
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
        fstate.state.active_link_popover = None;
        ui.ctx().request_repaint();
    }

    outside_click_blocked
}
