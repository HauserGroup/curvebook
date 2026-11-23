//! Frozen 3-tier column headers, corner archetype triggers, and row-title header bands for
//! `Column` and multi-archetype data grids.

use std::collections::HashSet;
use std::ops::Range;

use curvebook_doc::{ColumnDesignHint, DataSheet, SheetId, SubcolumnMode, TableArchetype, XColumnMode, default_column_label};
use curvebook_tools::GridCellCoord;
use egui::{Align, CornerRadius, FontId, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::i18n::tr;
use crate::inspect::ZLayer;
use crate::views::grid_core::grid_tr;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents};
use super::inline_edit::{InlineTextEditSpec, begin_inline_edit, render_inline_text_edit};
use super::mv_roles::render_mv_tier3_role_chip;
use super::shared::{
    AuxInlineTarget, CORNER_TRIGGER_HEIGHT_PT, CellTextStyle, GridRenderCtx, MultiGridCtx, MultiGridFrameState, archetype_badge,
    compute_visible_range, is_multi_click_edit, pressed_key_event_modifiers, record_meta_if_enabled, record_meta_with_unclipped_if_enabled,
    render_ellipsized_text, set_grid_focus_active,
};
use super::{ColumnGridEguiState, GridInlineEditTarget};

#[derive(Clone, Debug, PartialEq, Eq)]
enum FactorTitleTarget {
    RowFactor,
    ColumnFactor { col: usize },
}

#[derive(Clone, Debug)]
struct FactorTitleEditBuffer {
    target: FactorTitleTarget,
    original: String,
    draft: String,
    just_focused: bool,
    suppress_loss_once: bool,
}

fn factor_title_edit_key(sheet_id: &SheetId) -> egui::Id {
    egui::Id::unique(("r10_27_factor_title_edit", sheet_id.as_str()))
}

fn factor_te_id(ui: &Ui, sheet_id: &SheetId, widget_id: &str) -> egui::Id {
    ui.make_persistent_id(("r10_27_factor_te", sheet_id.as_str(), widget_id))
}

fn current_row_factor_title(arch: &TableArchetype) -> Option<&str> {
    match arch {
        TableArchetype::Grouped(g) => Some(g.row_factor_title.as_str()),
        TableArchetype::Contingency(c) => Some(c.row_factor_title.as_str()),
        _ => None,
    }
}

fn current_col_factor_title(arch: &TableArchetype) -> Option<&str> {
    match arch {
        TableArchetype::Grouped(g) => Some(g.column_factor_title.as_str()),
        TableArchetype::Contingency(c) => Some(c.column_factor_title.as_str()),
        _ => None,
    }
}

fn begin_factor_title_edit(
    ui: &Ui,
    fstate: &mut MultiGridFrameState<'_>,
    sheet_id: &SheetId,
    widget_id: &str,
    target: FactorTitleTarget,
    current: &str,
) {
    fstate.state.inline_edit = None;
    fstate.aux.aux_edit = None;
    let buf = FactorTitleEditBuffer { target, original: current.to_owned(), draft: current.to_owned(), just_focused: true, suppress_loss_once: true };
    let key = factor_title_edit_key(sheet_id);
    ui.ctx().data_mut(|d| d.insert_temp(key, Some(buf)));
    let te_id = factor_te_id(ui, sheet_id, widget_id);
    ui.memory_mut(|m| m.request_focus(te_id));
    ui.ctx().request_repaint();
}

fn render_factor_title_editor(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, rect: Rect, widget_id: &str, hint: &str) {
    let ds = mctx.ds;
    let key = factor_title_edit_key(&ds.id);
    let Some(mut edit) = ui.ctx().data_mut(|d| d.get_temp::<Option<FactorTitleEditBuffer>>(key).flatten()) else {
        return;
    };
    let te_id = factor_te_id(ui, &ds.id, widget_id);
    let mut te = TextEdit::singleline(&mut edit.draft).id(te_id).desired_width(rect.width()).margin(vec2(4.0, 2.0));
    if !hint.is_empty() {
        te = te.hint_text(hint);
    }
    let resp = ui.put(rect, te);
    if edit.just_focused {
        resp.request_focus();
        edit.just_focused = false;
        edit.suppress_loss_once = true;
    }
    record_meta_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "textbox",
            label: &edit.draft,
            command_id: Some("table.setTitles"),
            value: Some(serde_json::Value::String(edit.draft.clone())),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    let esc = ui.input(|i| i.key_pressed(Key::Escape));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let tab = pressed_key_event_modifiers(ui, Key::Tab).is_some();
    let lost = resp.lost_focus() && !edit.suppress_loss_once;
    edit.suppress_loss_once = false;

    if esc {
        ui.ctx().data_mut(|d| d.remove_temp::<Option<FactorTitleEditBuffer>>(key));
        ui.memory_mut(|m| m.surrender_focus(te_id));
        return;
    }
    if enter || tab || lost {
        if enter {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        }
        if tab {
            ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Tab);
                i.consume_key(Modifiers::SHIFT, Key::Tab);
            });
        }
        let trimmed = edit.draft.trim();
        let field = match edit.target {
            FactorTitleTarget::RowFactor => "row_factor_title",
            FactorTitleTarget::ColumnFactor { .. } => "column_factor_title",
        };
        if trimmed != edit.original.trim() {
            fstate.intents.push_engine("table.setTitles", json!({ "sheet_id": ds.id.as_str(), field: trimmed }));
        }
        ui.ctx().data_mut(|d| d.remove_temp::<Option<FactorTitleEditBuffer>>(key));
        ui.memory_mut(|m| m.surrender_focus(te_id));
        return;
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, Some(edit)));
}

pub(super) fn render_corner_and_row_titles_header(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let blocked = rctx.input_blocked(state);
    ui.scope_builder(UiBuilder::new().max_rect(geom.corner_rect), |ui| {
        ui.set_clip_rect(geom.corner_rect);

        // Top-left Corner Archetype Trigger (`Column`)
        let trigger_h = CORNER_TRIGGER_HEIGHT_PT.min(geom.header_h);
        let trigger_rect = Rect::from_min_size(geom.corner_rect.min, vec2(geom.row_num_w, trigger_h));
        let trigger_sense = if blocked { Sense::hover() } else { Sense::click() };
        let trigger_resp = ui.interact(trigger_rect, rctx.sheet_id_base.with("corner_archetype"), trigger_sense);
        let trigger_bg = if trigger_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
        ui.painter().rect_filled(trigger_rect, CornerRadius::ZERO, trigger_bg);
        ui.painter().rect_stroke(trigger_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let archetype_label = format!("{} ▾", tr(rctx.locale, "table.column"));
        let (trigger_elided, _) = render_ellipsized_text(
            ui,
            trigger_rect,
            &archetype_label,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_strong, Align::Center),
        );
        if trigger_elided {
            state.ellipsized_headers += 1;
        }
        let acc_archetype = archetype_label.clone();
        trigger_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_archetype.clone()));
        if trigger_resp.hovered() {
            let tip = if trigger_elided {
                format!("{archetype_label} — {}", tr(rctx.locale, "cmd.ui.table.convertArchetype.open"))
            } else {
                tr(rctx.locale, "cmd.ui.table.convertArchetype.open").to_owned()
            };
            let _ = trigger_resp.clone().on_hover_text(tip);
        }
        if trigger_resp.clicked() && !blocked {
            intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": rctx.ds.id.as_str() }));
            ui.ctx().request_repaint();
        }
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &trigger_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.archetype",
                role: "button",
                label: &archetype_label,
                command_id: Some("ui.table.convertArchetype.open"),
                value: Some(json!({ "archetype": "column" })),
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: trigger_elided,
                full_label: trigger_elided.then_some(archetype_label.as_str()),
            },
        );

        // `#` header below Corner Trigger
        let hash_rect = Rect::from_min_max(
            pos2(geom.corner_rect.left(), geom.corner_rect.top() + trigger_h),
            pos2(geom.corner_rect.left() + geom.row_num_w, geom.corner_rect.bottom()),
        );
        let hash_resp = ui.interact(hash_rect, rctx.sheet_id_base.with("corner_row_num"), Sense::hover());
        ui.painter().rect_filled(hash_rect, CornerRadius::ZERO, tokens.header_bg);
        ui.painter().rect_stroke(hash_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        let hash_label = grid_tr(rctx.locale, "w5.grid_core.row_number_header", "#");
        let (hash_elided, _) = render_ellipsized_text(
            ui,
            hash_rect,
            hash_label,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
        );
        let acc_hash = hash_label.to_owned();
        hash_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_hash.clone()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &hash_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.row_num",
                role: "label",
                label: hash_label,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: hash_elided,
                full_label: hash_elided.then_some(hash_label),
            },
        );

        // Row Titles header + toggle
        let rt_x = geom.corner_rect.left() + geom.row_num_w;
        let rt_header_rect = Rect::from_min_size(pos2(rt_x, geom.corner_rect.top()), vec2(geom.rt_w, geom.header_h));
        render_row_titles_header_block(ui, rctx, state, meta, intents, rt_header_rect, blocked);
    });
}

fn render_row_titles_header_block(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    rt_header_rect: Rect,
    blocked: bool,
) {
    let tokens = rctx.tokens;
    let geom = rctx.geom;
    let toggle_sense = if blocked { Sense::hover() } else { Sense::click() };

    if geom.effective_collapsed {
        let expand_lbl = grid_tr(rctx.locale, "w5.grid_core.expand_row_titles", "Expand Row Titles");
        let short_lbl = grid_tr(rctx.locale, "w5.grid_core.row_titles_short", "RT");
        let resp = ui.interact(rt_header_rect, rctx.sheet_id_base.with("row_titles_toggle"), toggle_sense);
        let bg = if resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
        ui.painter().rect_filled(rt_header_rect, CornerRadius::ZERO, bg);
        ui.painter().rect_stroke(rt_header_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        let (elided, _) = render_ellipsized_text(
            ui,
            rt_header_rect,
            short_lbl,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_primary, Align::Center),
        );
        if elided {
            state.ellipsized_headers += 1;
        }
        let acc = expand_lbl.to_owned();
        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
        if resp.hovered() {
            let _ = resp.clone().on_hover_text(expand_lbl);
        }
        if resp.clicked() && !blocked {
            state.row_titles_collapsed = false;
            intents.push_set_row_titles_collapsed(false);
            ui.ctx().request_repaint();
        }
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: "grid.row_titles.toggle",
                role: "button",
                label: expand_lbl,
                command_id: None,
                value: Some(json!({ "collapsed": true })),
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        return;
    }

    let top_rect = Rect::from_min_size(rt_header_rect.min, vec2(geom.rt_w, geom.tier1_h));
    let rt_title = grid_tr(rctx.locale, "w5.grid_core.row_titles", "Row Titles");
    let band_resp = ui.interact(top_rect, rctx.sheet_id_base.with("row_titles_band"), Sense::hover());
    ui.painter().rect_filled(top_rect, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().rect_stroke(top_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let (band_elided, _) = render_ellipsized_text(
        ui,
        top_rect,
        rt_title,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_strong, Align::Center),
    );
    if band_elided {
        state.ellipsized_headers += 1;
    }
    let acc_band = rt_title.to_owned();
    band_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_band.clone()));
    if band_elided && band_resp.hovered() {
        let _ = band_resp.clone().on_hover_text(rt_title);
    }
    record_meta_if_enabled(
        ui,
        meta,
        state.record_inspect_meta,
        &band_resp,
        FrameInspectEntryInput {
            widget_id: "grid.row_titles.band",
            role: "label",
            label: rt_title,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: band_elided,
            full_label: band_elided.then_some(rt_title),
        },
    );

    let toggle_rect = Rect::from_min_max(pos2(rt_header_rect.left(), top_rect.bottom()), rt_header_rect.max);
    let collapse_lbl = grid_tr(rctx.locale, "w5.grid_core.collapse_row_titles", "Collapse Row Titles");
    let resp = ui.interact(toggle_rect, rctx.sheet_id_base.with("row_titles_toggle"), toggle_sense);
    let bg = if resp.hovered() && !blocked { tokens.hover_bg } else { tokens.panel_bg };
    ui.painter().rect_filled(toggle_rect, CornerRadius::ZERO, bg);
    ui.painter().rect_stroke(toggle_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let toggle_visual = format!("◀ {}", grid_tr(rctx.locale, "w5.grid_core.row_titles_short", "RT"));
    let (elided, _) = render_ellipsized_text(
        ui,
        toggle_rect,
        &toggle_visual,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
    );
    let acc_toggle = collapse_lbl.to_owned();
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_toggle.clone()));
    if resp.hovered() {
        let _ = resp.clone().on_hover_text(collapse_lbl);
    }
    if resp.clicked() && !blocked {
        state.row_titles_collapsed = true;
        intents.push_set_row_titles_collapsed(true);
        ui.ctx().request_repaint();
    }
    record_meta_if_enabled(
        ui,
        meta,
        state.record_inspect_meta,
        &resp,
        FrameInspectEntryInput {
            widget_id: "grid.row_titles.toggle",
            role: "button",
            label: collapse_lbl,
            command_id: None,
            value: Some(json!({ "collapsed": false })),
            enabled: !blocked,
            disabled_reason: blocked.then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: elided,
            full_label: elided.then_some(collapse_lbl),
        },
    );
}

pub(super) fn render_column_headers_3tier(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    scroll_x: f32,
    vis_subcols: &Range<usize>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let blocked = rctx.input_blocked(state);
    if geom.total_cols == 0 || geom.header_data_rect.width() <= 1.0 {
        return;
    }

    let flat_range = if vis_subcols.is_empty() {
        compute_visible_range(scroll_x, geom.header_data_rect.width(), geom.subcol_w, geom.total_flat_cols)
    } else {
        vis_subcols.clone()
    };
    if flat_range.is_empty() {
        return;
    }

    let start_col = flat_range.start / geom.subs_per_col;
    let end_col = flat_range.end.div_ceil(geom.subs_per_col).min(geom.total_cols);

    ui.scope_builder(UiBuilder::new().max_rect(geom.header_data_rect), |ui| {
        ui.set_clip_rect(geom.header_data_rect);
        let y1 = geom.header_data_rect.top();
        let y2 = y1 + geom.tier1_h;
        let y3 = y2 + geom.tier2_h;

        for col in start_col..end_col {
            #[allow(clippy::cast_precision_loss)]
            let col_x = geom.header_data_rect.left() - scroll_x + ((col * geom.subs_per_col) as f32) * geom.subcol_w;
            #[allow(clippy::cast_precision_loss)]
            let col_w = (geom.subs_per_col as f32) * geom.subcol_w;

            // Tier 1: Super-Column ID (`Col A`, `Col B`, ...)
            let r1_raw = Rect::from_min_size(pos2(col_x, y1), vec2(col_w, geom.tier1_h));
            let r1 = r1_raw.intersect(geom.header_data_rect);
            if r1.width() > 0.5 && r1.height() > 0.5 {
                let super_label = format!("Col {}", default_column_label(col));
                let r1_resp = ui.interact(r1, rctx.sheet_id_base.with(("col_super", col)), Sense::hover());
                ui.painter().rect_filled(r1, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(r1, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                let (elided, _) = render_ellipsized_text(
                    ui,
                    r1,
                    &super_label,
                    CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
                );
                if elided {
                    state.ellipsized_headers += 1;
                }
                let acc = super_label.clone();
                r1_resp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc.clone()));
                if elided && r1_resp.hovered() {
                    let _ = r1_resp.clone().on_hover_text(&super_label);
                }
                record_meta_if_enabled(
                    ui,
                    meta,
                    state.record_inspect_meta,
                    &r1_resp,
                    FrameInspectEntryInput {
                        widget_id: &format!("grid.col_super.{col}"),
                        role: "label",
                        label: &super_label,
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(super_label.as_str()),
                    },
                );
            }

            // Tier 2: Editable Column Title
            let r2_raw = Rect::from_min_size(pos2(col_x, y2), vec2(col_w, geom.tier2_h));
            let r2 = r2_raw.intersect(geom.header_data_rect);
            if r2.width() > 0.5 && r2.height() > 0.5 {
                let col_title_raw = rctx.col_table.data_sets.get(col).map_or("", |d| d.title.as_str());
                let col_title = if col_title_raw.is_empty() { default_column_label(col) } else { col_title_raw.to_owned() };
                if state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::ColumnTitle { col }) {
                    let widget_id = format!("grid.col_title.{col}");
                    let _ = render_inline_text_edit(
                        ui,
                        rctx,
                        state,
                        meta,
                        intents,
                        InlineTextEditSpec {
                            rect: r2,
                            editor_id: rctx.sheet_id_base.with(("col_title_edit", col)),
                            hint: &col_title,
                            widget_id: &widget_id,
                        },
                    );
                } else {
                    let sense = if blocked { Sense::hover() } else { Sense::click() };
                    let r2_resp = ui.interact(r2, rctx.sheet_id_base.with(("col_title", col)), sense);
                    let bg = if r2_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.panel_bg };
                    ui.painter().rect_filled(r2, CornerRadius::ZERO, bg);
                    ui.painter().rect_stroke(r2, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                    let (elided, _) = render_ellipsized_text(
                        ui,
                        r2,
                        &col_title,
                        CellTextStyle::plain(FontId::proportional(12.0), tokens.text_strong, Align::Center),
                    );
                    if elided {
                        state.ellipsized_headers += 1;
                    }
                    let acc = col_title.clone();
                    let editable = !rctx.ds.frozen && !blocked;
                    r2_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, editable, acc.clone()));
                    if r2_resp.hovered() {
                        let _ = r2_resp.clone().on_hover_text(&col_title);
                    }
                    if !blocked {
                        if is_multi_click_edit(&r2_resp) && !rctx.ds.frozen {
                            begin_inline_edit(ui, state, GridInlineEditTarget::ColumnTitle { col }, col_title.clone());
                        } else if r2_resp.clicked() && geom.total_rows > 0 {
                            intents.push_select_grid_cell(GridCellCoord::new(0, col, 0));
                            intents.push_extend_grid_selection(GridCellCoord::new(geom.total_rows - 1, col, geom.subs_per_col.saturating_sub(1)));
                            set_grid_focus_active(ui, true);
                            ui.ctx().request_repaint();
                        }
                    }
                    record_meta_with_unclipped_if_enabled(
                        ui,
                        meta,
                        state.record_inspect_meta,
                        &r2_resp,
                        r2_raw,
                        FrameInspectEntryInput {
                            widget_id: &format!("grid.col_title.{col}"),
                            role: "button",
                            label: &col_title,
                            command_id: None,
                            value: Some(json!({ "col": col, "title": col_title })),
                            enabled: editable,
                            disabled_reason: blocked.then_some("modal dialog is active").or_else(|| rctx.ds.frozen.then_some("Frozen")),
                            z_layer: ZLayer::Workspace,
                            ellipsized: elided,
                            full_label: elided.then_some(col_title.as_str()),
                        },
                    );
                }
            }

            // Tier 3: Subcolumn Roles
            for subcol in 0..geom.subs_per_col {
                #[allow(clippy::cast_precision_loss)]
                let sub_x = col_x + (subcol as f32) * geom.subcol_w;
                let r3_raw = Rect::from_min_size(pos2(sub_x, y3), vec2(geom.subcol_w, geom.tier3_h));
                let r3 = r3_raw.intersect(geom.header_data_rect);
                if r3.width() <= 0.5 || r3.height() <= 0.5 {
                    continue;
                }
                let sub_label = column_subcolumn_role_label(rctx.col_table.subcolumn_mode, subcol, rctx.locale);
                let r3_resp = ui.interact(r3, rctx.sheet_id_base.with(("subcol_hdr", col, subcol)), Sense::hover());
                ui.painter().rect_filled(r3, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(r3, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                let (elided, _) = render_ellipsized_text(
                    ui,
                    r3,
                    &sub_label,
                    CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
                );
                if elided {
                    state.ellipsized_headers += 1;
                }
                let acc = sub_label.clone();
                r3_resp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc.clone()));
                if r3_resp.hovered() {
                    let hint_tag = match rctx.col_table.design_hint {
                        ColumnDesignHint::UnpairedReplicates => "Unpaired",
                        ColumnDesignHint::PairedRepeatedMeasures => "Paired",
                        ColumnDesignHint::PrecomputedError => "Summary",
                    };
                    let _ = r3_resp.clone().on_hover_text(format!("{sub_label} · {hint_tag}"));
                }
                record_meta_if_enabled(
                    ui,
                    meta,
                    state.record_inspect_meta,
                    &r3_resp,
                    FrameInspectEntryInput {
                        widget_id: &format!("grid.subcol_title.{col}.{subcol}"),
                        role: "label",
                        label: &sub_label,
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(sub_label.as_str()),
                    },
                );
            }
        }
    });
}

pub(super) fn column_subcolumn_role_label(mode: SubcolumnMode, subcol: usize, locale: &str) -> String {
    match mode {
        SubcolumnMode::Single => String::from("Y"),
        SubcolumnMode::MeanSdN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sd_col", "SD").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanSemN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sem_col", "SEM").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanCvN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => String::from("%CV"),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanLowHigh => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => String::from("-Err"),
            2 => String::from("+Err"),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::UpperLowerLimits => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.min_col", "Min").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.max_col", "Max").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MedianIqr => match subcol {
            0 => grid_tr(locale, "w5.grid_core.median_col", "Median").to_owned(),
            1 => String::from("Q1"),
            2 => String::from("Q3"),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::RawReplicates(_) => {
            if subcol == 0 {
                String::from("Y")
            } else {
                format!("Y{}", subcol + 1)
            }
        }
    }
}

pub(super) fn render_multi_corner(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let cr = geom.corner_rect;
    ui.painter().rect_filled(cr, 0.0, tokens.header_bg);
    ui.painter().rect_stroke(cr, 0.0, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    let y1 = cr.min.y;
    let y2 = y1 + geom.h_tier1;
    let y3 = y2 + geom.h_tier2;
    let y4 = cr.max.y;
    let x_mid = cr.min.x + geom.row_num_w;
    let x_end = cr.max.x;

    let corner_btn_r = Rect::from_min_max(pos2(cr.min.x, y1), pos2(x_mid, y2));
    let badge = archetype_badge(&ds.archetype);
    let corner_resp =
        ui.interact(corner_btn_r, ui.make_persistent_id("grid.corner.archetype"), if blocked { Sense::hover() } else { Sense::click() });
    let fill = if corner_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
    ui.painter().rect_filled(corner_btn_r, 0.0, fill);
    render_ellipsized_text(
        ui,
        corner_btn_r.shrink(2.0),
        badge,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.accent, Align::Center),
    );
    let acc_badge = badge.to_owned();
    corner_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_badge.clone()));
    record_meta_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &corner_resp,
        FrameInspectEntryInput {
            widget_id: "grid.corner.archetype",
            role: "button",
            label: badge,
            command_id: Some("ui.table.convertArchetype.open"),
            value: None,
            enabled: !blocked,
            disabled_reason: blocked.then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    if corner_resp.clicked() && !blocked {
        fstate.intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": ds.id.as_str() }));
    }

    let sheet_btn_r = Rect::from_min_max(pos2(x_mid, y1), pos2(x_end, y2));
    if sheet_btn_r.width() > 8.0 {
        let sheet_editable = !blocked && !ds.frozen;
        let st_resp =
            ui.interact(sheet_btn_r, ui.make_persistent_id("grid.sheet_title"), if sheet_editable { Sense::click() } else { Sense::hover() });
        if st_resp.hovered() && sheet_editable {
            ui.painter().rect_filled(sheet_btn_r, 0.0, tokens.hover_bg);
        }
        let (ellip, _) = render_ellipsized_text(
            ui,
            sheet_btn_r.shrink2(vec2(4.0, 1.0)),
            &ds.title,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_strong, Align::LEFT),
        );
        if ellip {
            fstate.state.ellipsized_headers += 1;
        }
        let acc_title = ds.title.clone();
        st_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, sheet_editable, acc_title.clone()));
        record_meta_if_enabled(
            ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
            &st_resp,
            FrameInspectEntryInput {
                widget_id: "grid.sheet_title",
                role: "button",
                label: &ds.title,
                command_id: Some("sheet.rename"),
                value: Some(serde_json::Value::String(ds.title.clone())),
                enabled: sheet_editable,
                disabled_reason: if blocked {
                    Some("modal dialog is active")
                } else if ds.frozen {
                    Some("Data sheet is frozen (read-only).")
                } else {
                    None
                },
                z_layer: ZLayer::Workspace,
                ellipsized: ellip,
                full_label: ellip.then_some(ds.title.as_str()),
            },
        );
        if st_resp.clicked() && sheet_editable {
            fstate.state.inline_edit = None;
            fstate.aux.aux_edit = Some((AuxInlineTarget::SheetTitle, ds.title.clone(), true));
        }
    }

    let hash_r = Rect::from_min_max(pos2(cr.min.x, y2), pos2(x_mid, y3));
    let hash_resp = ui.interact(hash_r, ui.make_persistent_id("grid.corner.row_num"), Sense::hover());
    render_ellipsized_text(
        ui,
        hash_r,
        "#",
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
    );
    hash_resp.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, String::from("#")));
    record_meta_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &hash_resp,
        FrameInspectEntryInput {
            widget_id: "grid.corner.row_num",
            role: "label",
            label: "#",
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    let rt_hdr_r = Rect::from_min_max(pos2(x_mid, y2), pos2(x_end, y3));
    let rt_band_label = row_titles_band_label(ds, &app.locale);
    let row_factor = current_row_factor_title(&ds.archetype).map(str::to_owned);
    let row_factor_editable = !blocked && !ds.frozen && !geom.row_titles_collapsed && row_factor.is_some();
    let editing_row_factor = row_factor_editable
        && ui.ctx().data(|d| {
            d.get_temp::<Option<FactorTitleEditBuffer>>(factor_title_edit_key(&ds.id))
                .flatten()
                .is_some_and(|e| e.target == FactorTitleTarget::RowFactor)
        });
    if editing_row_factor {
        render_factor_title_editor(ui, mctx, fstate, rt_hdr_r, "grid.row_titles.band", "Row factor...");
    } else {
        let rt_sense = if row_factor_editable { Sense::click() } else { Sense::hover() };
        let rt_resp = ui.interact(rt_hdr_r, ui.make_persistent_id("grid.row_titles.band"), rt_sense);
        if row_factor_editable && rt_resp.hovered() {
            ui.painter().rect_filled(rt_hdr_r, CornerRadius::ZERO, tokens.hover_bg);
            let _ = rt_resp.clone().on_hover_text("Double-click to edit row factor title");
        }
        if !geom.row_titles_collapsed {
            render_ellipsized_text(
                ui,
                rt_hdr_r.shrink2(vec2(5.0, 1.0)),
                &rt_band_label,
                CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_strong, Align::LEFT),
            );
        }
        let acc_rt = rt_band_label.clone();
        let has_row_factor = row_factor.is_some();
        let role = if has_row_factor { egui::Role::Button } else { egui::Role::Label };
        rt_resp.widget_info(move || WidgetInfo::labeled(role, !has_row_factor || row_factor_editable, acc_rt.clone()));
        record_meta_if_enabled(
            ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
            &rt_resp,
            FrameInspectEntryInput {
                widget_id: "grid.row_titles.band",
                role: if has_row_factor { "button" } else { "label" },
                label: &rt_band_label,
                command_id: row_factor.as_ref().map(|_| "table.setTitles"),
                value: row_factor.as_deref().map(|rf| serde_json::Value::String(rf.to_owned())),
                enabled: !has_row_factor || row_factor_editable,
                disabled_reason: (has_row_factor && !row_factor_editable).then_some("Frozen or blocked"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if row_factor_editable
            && is_multi_click_edit(&rt_resp)
            && let Some(rf) = row_factor.as_deref()
        {
            begin_factor_title_edit(ui, fstate, &ds.id, "grid.row_titles.band", FactorTitleTarget::RowFactor, rf);
        }
    }

    let toggle_r = Rect::from_min_max(pos2(x_mid, y3), pos2(x_end, y4));
    let toggle_lbl = if geom.row_titles_collapsed { ">" } else { "<|>" };
    let toggle_resp = ui.interact(toggle_r, ui.make_persistent_id("grid.row_titles.toggle"), if blocked { Sense::hover() } else { Sense::click() });
    if toggle_resp.hovered() && !blocked {
        ui.painter().rect_filled(toggle_r, 0.0, tokens.hover_bg);
    }
    render_ellipsized_text(
        ui,
        toggle_r.shrink(2.0),
        toggle_lbl,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
    );
    let acc_tgl = if geom.row_titles_collapsed { "Expand Row Titles" } else { "Collapse Row Titles" };
    toggle_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_tgl.to_owned()));
    record_meta_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &toggle_resp,
        FrameInspectEntryInput {
            widget_id: "grid.row_titles.toggle",
            role: "button",
            label: toggle_lbl,
            command_id: None,
            value: None,
            enabled: !blocked,
            disabled_reason: blocked.then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    if toggle_resp.clicked() && !blocked {
        let next = !geom.row_titles_collapsed;
        fstate.state.row_titles_collapsed = next;
        fstate.intents.push_set_row_titles_collapsed(next);
    }

    let p = ui.painter();
    p.line_segment([pos2(cr.min.x, y2), pos2(cr.max.x, y2)], Stroke::new(1.0, tokens.divider));
    p.line_segment([pos2(cr.min.x, y3), pos2(cr.max.x, y3)], Stroke::new(1.0, tokens.divider));
    p.line_segment([pos2(x_mid, y1), pos2(x_mid, y4)], Stroke::new(1.0, tokens.divider));
}

fn row_titles_band_label(ds: &DataSheet, locale: &str) -> String {
    match &ds.archetype {
        TableArchetype::Grouped(g) if !g.row_factor_title.trim().is_empty() => g.row_factor_title.clone(),
        TableArchetype::Contingency(c) if !c.row_factor_title.trim().is_empty() => c.row_factor_title.clone(),
        TableArchetype::PartsOfWhole(_) => String::from("Slice Labels"),
        _ => grid_tr(locale, "w6.grid.row_titles", "Row Titles").to_owned(),
    }
}

pub(super) fn render_multi_column_headers(
    ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    active_mv_operands: &HashSet<usize>,
) {
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let hdr_rect = geom.col_header_rect;
    if !hdr_rect.is_positive() {
        return;
    }
    let scroll_x = fstate.state.scroll_offset_pt.x;
    let vis_flat = compute_visible_range(scroll_x, hdr_rect.width(), geom.subcol_w, geom.total_flat_cols);

    ui.scope_builder(UiBuilder::new().max_rect(hdr_rect), |hdr_ui| {
        hdr_ui.set_clip_rect(hdr_rect);
        hdr_ui.painter().rect_filled(hdr_rect, 0.0, tokens.header_bg);

        let y1 = hdr_rect.min.y;
        let y2 = y1 + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let y4 = hdr_rect.max.y;

        if geom.x_subcols > 0 && vis_flat.start < geom.x_subcols {
            render_multi_x_headers(hdr_ui, mctx, fstate, &vis_flat, scroll_x);
        }

        let vis_data_start = vis_flat.start.saturating_sub(geom.x_subcols);
        let vis_data_end = vis_flat.end.saturating_sub(geom.x_subcols);
        if vis_data_start < vis_data_end && geom.col_count > 0 {
            render_multi_data_col_headers(hdr_ui, mctx, fstate, vis_data_start..vis_data_end, scroll_x, active_mv_operands);
        }

        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y2), pos2(hdr_rect.max.x, y2)], Stroke::new(1.0, tokens.divider));
        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y3), pos2(hdr_rect.max.x, y3)], Stroke::new(1.0, tokens.divider));
        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y4), pos2(hdr_rect.max.x, y4)], Stroke::new(1.5, tokens.border));
    });
}

fn render_multi_x_headers(hdr_ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, vis_flat: &Range<usize>, scroll_x: f32) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let hdr_rect = geom.col_header_rect;
    let y1 = hdr_rect.min.y;
    let y2 = y1 + geom.h_tier1;
    let y3 = y2 + geom.h_tier2;
    let y4 = hdr_rect.max.y;
    let x_x0 = hdr_rect.min.x - scroll_x;
    #[allow(clippy::cast_precision_loss)]
    let x_w = (geom.x_subcols as f32) * geom.subcol_w;
    let x_tier1_r = Rect::from_min_max(pos2(x_x0, y1), pos2(x_x0 + x_w, y2));
    let super_x = if matches!(ds.archetype, TableArchetype::Survival(_)) { "Time" } else { "X" };
    render_ellipsized_text(
        hdr_ui,
        x_tier1_r.intersect(hdr_rect).shrink2(vec2(4.0, 1.0)),
        super_x,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
    );

    for sx in 0..geom.x_subcols {
        if !vis_flat.contains(&sx) {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let sx_x0 = x_x0 + (sx as f32) * geom.subcol_w;
        let sx_x1 = sx_x0 + geom.subcol_w;

        let t2_raw = Rect::from_min_max(pos2(sx_x0, y2), pos2(sx_x1, y3));
        let t2_r = t2_raw.intersect(hdr_rect);
        let x_title = x_column_title(&ds.archetype, sx);
        if t2_r.width() > 2.0 {
            let wid = format!("grid.x_title.{sx}");
            let x_title_editable = !blocked && !ds.frozen && sx == 0;
            let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if x_title_editable { Sense::click() } else { Sense::hover() });
            if resp.hovered() && x_title_editable {
                hdr_ui.painter().rect_filled(t2_r, 0.0, tokens.hover_bg);
            }
            render_ellipsized_text(
                hdr_ui,
                t2_r.shrink2(vec2(4.0, 1.0)),
                &x_title,
                CellTextStyle::plain(FontId::proportional(12.0), tokens.accent, Align::Center),
            );
            let acc = format!("X Title: {x_title}");
            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, x_title_editable, acc.clone()));
            record_meta_with_unclipped_if_enabled(
                hdr_ui,
                fstate.meta,
                fstate.state.record_inspect_meta,
                &resp,
                t2_raw,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "button",
                    label: &x_title,
                    command_id: Some("table.setTitles"),
                    value: Some(serde_json::Value::String(x_title.clone())),
                    enabled: x_title_editable,
                    disabled_reason: if blocked {
                        Some("modal dialog is active")
                    } else if ds.frozen {
                        Some("Data sheet is frozen (read-only).")
                    } else if sx != 0 {
                        Some("X error subcolumn title is derived.")
                    } else {
                        None
                    },
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() && x_title_editable {
                fstate.state.inline_edit = None;
                fstate.aux.aux_edit = Some((AuxInlineTarget::XTitle { subcol: sx }, x_title, true));
            }
        }

        let t3_r = Rect::from_min_max(pos2(sx_x0, y3), pos2(sx_x1, y4)).intersect(hdr_rect);
        let x_sub_lbl = x_subcolumn_label(&ds.archetype, sx, &mctx.app.locale);
        if t3_r.width() > 2.0 {
            let swid = format!("grid.subcol_hdr.x.{sx}");
            let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), Sense::hover());
            render_ellipsized_text(
                hdr_ui,
                t3_r.shrink2(vec2(3.0, 1.0)),
                &x_sub_lbl,
                CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
            );
            let acc_s = x_sub_lbl.clone();
            sresp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc_s.clone()));
            record_meta_if_enabled(
                hdr_ui,
                fstate.meta,
                fstate.state.record_inspect_meta,
                &sresp,
                FrameInspectEntryInput {
                    widget_id: &swid,
                    role: "columnheader",
                    label: &x_sub_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }
        hdr_ui.painter().line_segment([pos2(sx_x1, y2), pos2(sx_x1, y4)], Stroke::new(1.0, tokens.divider));
    }
    let x_sep_x = x_x0 + x_w;
    hdr_ui.painter().line_segment([pos2(x_sep_x, y1), pos2(x_sep_x, y4)], Stroke::new(1.5, tokens.border));
}

fn render_multi_data_col_headers(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    vis_data: Range<usize>,
    scroll_x: f32,
    active_mv_operands: &HashSet<usize>,
) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let hdr_rect = geom.col_header_rect;
    let y1 = hdr_rect.min.y;
    let y2 = y1 + geom.h_tier1;
    let y3 = y2 + geom.h_tier2;
    let y4 = hdr_rect.max.y;
    let first_col = geom.flat_data_to_coord(vis_data.start).map_or(0, |(c, _)| c);
    let last_col = geom.flat_data_to_coord(vis_data.end.saturating_sub(1)).map_or(geom.col_count - 1, |(c, _)| c);

    for col in first_col..=last_col.min(geom.col_count.saturating_sub(1)) {
        let (span_flat_start, span_subs) = geom.col_span_flat(col);
        #[allow(clippy::cast_precision_loss)]
        let cx0 = hdr_rect.min.x + (span_flat_start as f32) * geom.subcol_w - scroll_x;
        #[allow(clippy::cast_precision_loss)]
        let cx1 = cx0 + (span_subs as f32) * geom.subcol_w;

        if active_mv_operands.contains(&col) {
            let highlight_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y4)).intersect(hdr_rect);
            hdr_ui.painter().rect_filled(highlight_r, 0.0, tokens.accent.linear_multiply(0.14));
        }

        let t1_raw = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y2));
        let t1_r = t1_raw.intersect(hdr_rect);
        let super_lbl = data_super_column_label(&ds.archetype, col);
        let col_factor = current_col_factor_title(&ds.archetype).map(str::to_owned);
        let col_factor_editable = !blocked && !ds.frozen && col_factor.is_some();
        let super_wid = format!("grid.col_super.{col}");
        let editing_col_factor = col_factor_editable
            && hdr_ui.ctx().data(|d| {
                d.get_temp::<Option<FactorTitleEditBuffer>>(factor_title_edit_key(&ds.id))
                    .flatten()
                    .is_some_and(|e| e.target == FactorTitleTarget::ColumnFactor { col })
            });
        if t1_r.width() > 4.0 {
            if editing_col_factor {
                render_factor_title_editor(hdr_ui, mctx, fstate, t1_r, &super_wid, "Column factor...");
            } else {
                let s_sense = if col_factor_editable { Sense::click() } else { Sense::hover() };
                let s_resp = hdr_ui.interact(t1_r, hdr_ui.make_persistent_id(&super_wid), s_sense);
                if col_factor_editable && s_resp.hovered() {
                    hdr_ui.painter().rect_filled(t1_r, CornerRadius::ZERO, tokens.hover_bg);
                    let _ = s_resp.clone().on_hover_text("Double-click to edit column factor title");
                }
                let (ellip_s, _) = render_ellipsized_text(
                    hdr_ui,
                    t1_r.shrink2(vec2(4.0, 1.0)),
                    &super_lbl,
                    CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
                );
                let acc_s = super_lbl.clone();
                let has_col_factor = col_factor.is_some();
                let s_role = if has_col_factor { egui::Role::Button } else { egui::Role::Label };
                s_resp.widget_info(move || WidgetInfo::labeled(s_role, !has_col_factor || col_factor_editable, acc_s.clone()));
                record_meta_with_unclipped_if_enabled(
                    hdr_ui,
                    fstate.meta,
                    fstate.state.record_inspect_meta,
                    &s_resp,
                    t1_raw,
                    FrameInspectEntryInput {
                        widget_id: &super_wid,
                        role: if has_col_factor { "button" } else { "label" },
                        label: &super_lbl,
                        command_id: col_factor.as_ref().map(|_| "table.setTitles"),
                        value: col_factor.as_deref().map(|cf| json!({ "col": col, "column_factor_title": cf })),
                        enabled: !has_col_factor || col_factor_editable,
                        disabled_reason: (has_col_factor && !col_factor_editable).then_some("Frozen or blocked"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: ellip_s,
                        full_label: ellip_s.then_some(super_lbl.as_str()),
                    },
                );
                if col_factor_editable
                    && is_multi_click_edit(&s_resp)
                    && let Some(cf) = col_factor.as_deref()
                {
                    begin_factor_title_edit(hdr_ui, fstate, &ds.id, &super_wid, FactorTitleTarget::ColumnFactor { col }, cf);
                }
            }
        }

        let t2_raw = Rect::from_min_max(pos2(cx0, y2), pos2(cx1, y3));
        let t2_r = t2_raw.intersect(hdr_rect);
        let col_title = data_column_title(&ds.archetype, col);
        if t2_r.width() > 4.0 && fstate.state.inline_edit.as_ref().is_none_or(|e| e.target != GridInlineEditTarget::ColumnTitle { col }) {
            let wid = format!("grid.col_title.{col}");
            let col_editable = !blocked && !ds.frozen;
            let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if col_editable { Sense::click() } else { Sense::hover() });
            if resp.hovered() && col_editable {
                hdr_ui.painter().rect_filled(t2_r, 0.0, tokens.hover_bg);
            }
            let (ellip, _) = render_ellipsized_text(
                hdr_ui,
                t2_r.shrink2(vec2(4.0, 1.0)),
                &col_title,
                CellTextStyle::plain(FontId::proportional(12.0), tokens.text_strong, Align::Center),
            );
            if ellip {
                fstate.state.ellipsized_headers += 1;
            }
            let acc = col_title.clone();
            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, col_editable, acc.clone()));
            record_meta_with_unclipped_if_enabled(
                hdr_ui,
                fstate.meta,
                fstate.state.record_inspect_meta,
                &resp,
                t2_raw,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "button",
                    label: &col_title,
                    command_id: Some("table.setTitles"),
                    value: Some(serde_json::Value::String(col_title.clone())),
                    enabled: col_editable,
                    disabled_reason: if blocked {
                        Some("modal dialog is active")
                    } else if ds.frozen {
                        Some("Data sheet is frozen (read-only).")
                    } else {
                        None
                    },
                    z_layer: ZLayer::Workspace,
                    ellipsized: ellip,
                    full_label: ellip.then_some(col_title.as_str()),
                },
            );
            if resp.clicked() && col_editable {
                fstate.aux.aux_edit = None;
                begin_inline_edit(hdr_ui, fstate.state, GridInlineEditTarget::ColumnTitle { col }, col_title);
            }
        }

        if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
            if let Some(var) = mv.variables.get(col) {
                let chip_raw = Rect::from_min_max(pos2(cx0, y3), pos2(cx1, y4));
                let chip_r = chip_raw.intersect(hdr_rect);
                if chip_r.width() > 6.0 {
                    render_mv_tier3_role_chip(hdr_ui, mctx, fstate, col, var, chip_r, chip_raw);
                }
            }
        } else {
            render_subcolumn_tier3_cells(hdr_ui, mctx, fstate, col, span_subs, cx0);
        }
        hdr_ui.painter().line_segment([pos2(cx1, y1), pos2(cx1, y4)], Stroke::new(1.25, tokens.border));
    }
}

fn render_subcolumn_tier3_cells(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    col: usize,
    span_subs: usize,
    cx0: f32,
) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let hdr_rect = geom.col_header_rect;
    let y3 = hdr_rect.min.y + geom.h_tier1 + geom.h_tier2;
    let y4 = hdr_rect.max.y;
    let is_nested = matches!(ds.archetype, TableArchetype::Nested(_));
    for subcol in 0..span_subs {
        #[allow(clippy::cast_precision_loss)]
        let sx0 = cx0 + (subcol as f32) * geom.subcol_w;
        let sx1 = sx0 + geom.subcol_w;
        let t3_raw = Rect::from_min_max(pos2(sx0, y3), pos2(sx1, y4));
        let t3_r = t3_raw.intersect(hdr_rect);
        if t3_r.width() <= 2.0 {
            continue;
        }
        let sub_lbl = data_subcolumn_header_label(ds, col, subcol, &app.locale);
        let swid = if is_nested { format!("grid.subcol_title.{col}.{subcol}") } else { format!("grid.subcol_hdr.{col}.{subcol}") };
        let sub_editable = is_nested && !blocked && !ds.frozen;
        let sense = if sub_editable { Sense::click() } else { Sense::hover() };
        let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), sense);
        if is_nested && sresp.hovered() && sub_editable {
            hdr_ui.painter().rect_filled(t3_r, 0.0, tokens.hover_bg);
        }
        let fg = if is_nested { tokens.text_primary } else { tokens.text_muted };
        render_ellipsized_text(
            hdr_ui,
            t3_r.shrink2(vec2(3.0, 1.0)),
            &sub_lbl,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), fg, Align::Center),
        );
        let acc_s = sub_lbl.clone();
        let role = if is_nested { egui::Role::Button } else { egui::Role::ColumnHeader };
        let role_str = if is_nested { "button" } else { "columnheader" };
        sresp.widget_info(move || WidgetInfo::labeled(role, !is_nested || sub_editable, acc_s.clone()));
        record_meta_with_unclipped_if_enabled(
            hdr_ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
            &sresp,
            t3_raw,
            FrameInspectEntryInput {
                widget_id: &swid,
                role: role_str,
                label: &sub_lbl,
                command_id: is_nested.then_some("table.setSubcolumnTitles"),
                value: is_nested.then(|| serde_json::Value::String(sub_lbl.clone())),
                enabled: !is_nested || sub_editable,
                disabled_reason: if is_nested && blocked {
                    Some("modal dialog is active")
                } else if is_nested && ds.frozen {
                    Some("Data sheet is frozen (read-only).")
                } else {
                    None
                },
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if sub_editable && sresp.clicked() {
            fstate.state.inline_edit = None;
            fstate.aux.aux_edit = Some((AuxInlineTarget::SubcolumnTitle { col, subcol }, sub_lbl, true));
        }
        if subcol + 1 < span_subs {
            hdr_ui.painter().line_segment([pos2(sx1, y3), pos2(sx1, y4)], Stroke::new(1.0, tokens.divider));
        }
    }
}

fn x_column_title(arch: &TableArchetype, subcol: usize) -> String {
    match arch {
        TableArchetype::Xy(xy) => {
            if subcol >= 1 {
                String::from("±X Error")
            } else {
                xy.x_title.clone()
            }
        }
        TableArchetype::Survival(s) => s.time_title.clone(),
        _ => String::from("X"),
    }
}

fn x_subcolumn_label(arch: &TableArchetype, subcol: usize, locale: &str) -> String {
    match arch {
        TableArchetype::Xy(xy) => match &xy.x_mode {
            XColumnMode::Numbers { subcolumn_mode } => {
                if subcolumn_mode.subcolumn_count() > 1 && subcol > 0 {
                    column_subcolumn_role_label(*subcolumn_mode, subcol, locale)
                } else {
                    String::from("X")
                }
            }
            XColumnMode::DateOrElapsedTime { unit_label } => {
                if unit_label.trim().is_empty() {
                    String::from("Time")
                } else {
                    format!("Time ({unit_label})")
                }
            }
            XColumnMode::GeneratedSequence { .. } => String::from("Seq"),
        },
        TableArchetype::Survival(_) => String::from("Time"),
        _ => String::from("X"),
    }
}

fn data_super_column_label(arch: &TableArchetype, col: usize) -> String {
    let letter = default_column_label(col);
    match arch {
        TableArchetype::Grouped(g) if !g.column_factor_title.trim().is_empty() => format!("{} {letter}", g.column_factor_title),
        TableArchetype::Contingency(c) if !c.column_factor_title.trim().is_empty() => format!("{} {letter}", c.column_factor_title),
        TableArchetype::Survival(_) => format!("Cohort {letter}"),
        TableArchetype::MultipleVariables(_) => format!("Var {}", col + 1),
        TableArchetype::PartsOfWhole(_) => String::from("Slice Share"),
        _ => format!("Group {letter}"),
    }
}

fn data_column_title(arch: &TableArchetype, col: usize) -> String {
    match arch {
        TableArchetype::Column(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Xy(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map_or_else(String::new, |v| v.title.clone()),
        TableArchetype::Nested(t) => t.groups.get(col).map_or_else(String::new, |g| g.title.clone()),
    }
}

fn data_subcolumn_header_label(ds: &DataSheet, col: usize, subcol: usize, locale: &str) -> String {
    match &ds.archetype {
        TableArchetype::Xy(t) => column_subcolumn_role_label(t.y_subcolumn_mode, subcol, locale),
        TableArchetype::Grouped(t) => column_subcolumn_role_label(t.subcolumn_mode, subcol, locale),
        TableArchetype::Column(t) => column_subcolumn_role_label(t.subcolumn_mode, subcol, locale),
        TableArchetype::Contingency(_) => String::from("Count"),
        TableArchetype::Survival(s) => format!("Event({}) / Cens({})", s.event_code, s.censored_code),
        TableArchetype::PartsOfWhole(_) => String::from("Value"),
        TableArchetype::MultipleVariables(_) => String::from("Var"),
        TableArchetype::Nested(t) => t
            .groups
            .get(col)
            .and_then(|g| g.subcolumn_titles.get(subcol))
            .cloned()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("Unit {}", subcol + 1)),
    }
}

pub(super) fn row_title_at(arch: &TableArchetype, row: usize) -> String {
    let slice = match arch {
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    slice.get(row).cloned().unwrap_or_default()
}
