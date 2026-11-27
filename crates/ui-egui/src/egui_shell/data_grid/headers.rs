//! Unified frozen 3-tier column headers, corner archetype trigger, and row-title header band
//! across all 8 table archetypes.

use std::collections::HashSet;
use std::ops::Range;

use curvebook_doc::{ColumnDesignHint, DataSheet, SheetId, SubcolumnMode, TableArchetype, XColumnMode, default_column_label};
use egui::{Align, CornerRadius, FontId, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::grid_view::grid_tr;
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::super::{FrameInspectEntryInput, UiIntent};
use super::GridInlineEditTarget;
use super::inline_edit::begin_inline_edit;
use super::mv_roles::render_mv_tier3_role_chip;
use super::shared::{
    AuxInlineTarget, CellTextStyle, GridDragOutcome, MultiGridCtx, MultiGridFrameState, SelectionFastLookup, archetype_badge, compute_visible_range,
    is_multi_click_edit, paint_find_highlight, pressed_key_event_modifiers, record_meta_if_enabled, record_meta_with_unclipped_if_enabled,
    render_ellipsized_text, set_grid_focus_active,
};

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

pub(super) fn is_factor_title_editing(ctx: &egui::Context, sheet_id: &SheetId) -> bool {
    ctx.data(|d| d.get_temp::<Option<FactorTitleEditBuffer>>(factor_title_edit_key(sheet_id)).flatten().is_some())
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
    let will_discard = ui.ctx().will_discard();
    if edit.just_focused {
        resp.request_focus();
        if !will_discard {
            edit.just_focused = false;
            edit.suppress_loss_once = true;
        }
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

    if will_discard {
        ui.ctx().data_mut(|d| d.insert_temp(key, Some(edit)));
        return;
    }

    let esc = ui.input(|i| i.key_pressed(Key::Escape));
    let enter = ui.input(|i| i.key_pressed(Key::Enter));
    let tab = pressed_key_event_modifiers(ui, Key::Tab).is_some();
    let lost = resp.lost_focus() && !edit.suppress_loss_once;
    edit.suppress_loss_once = false;

    if esc {
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
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

/// Tooltip design-hint tag for a [`curvebook_doc::ColumnTable`] (`AUD-13-24`).
///
/// Pre-summarized subcolumn modes (`MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`,
/// `UpperLowerLimits`, `MedianIqr`) always display `"Summary"` alongside explicit
/// [`ColumnDesignHint::PrecomputedError`].
#[must_use]
pub fn column_design_hint_tag(col_t: &curvebook_doc::ColumnTable) -> &'static str {
    if col_t.subcolumn_mode.is_pre_summarized() {
        return "Summary";
    }
    match col_t.design_hint {
        ColumnDesignHint::UnpairedReplicates => "Unpaired",
        ColumnDesignHint::PairedRepeatedMeasures => "Paired",
        ColumnDesignHint::PrecomputedError => "Summary",
    }
}

pub(super) fn column_subcolumn_role_label(mode: SubcolumnMode, subcol: usize, locale: &str) -> String {
    match mode {
        SubcolumnMode::Single => String::from("Y"),
        SubcolumnMode::MeanSd => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sd_col", "SD").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanSem => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sem_col", "SEM").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
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
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_strong, Align::Center),
    );
    let acc_badge = badge.to_owned();
    corner_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_badge.clone()));
    if corner_resp.hovered() {
        let _ = corner_resp.clone().on_hover_text(tr(mctx.locale, "cmd.ui.table.convertArchetype.open"));
    }
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
    let editing_sheet_title = fstate.aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::SheetTitle);
    if sheet_btn_r.width() > 8.0 && !editing_sheet_title {
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
            if st_resp.hovered() {
                let _ = st_resp.clone().on_hover_text(&ds.title);
            }
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
        render_factor_title_editor(ui, mctx, fstate, rt_hdr_r, "grid.row_titles.band", "Row factor");
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
    if toggle_resp.hovered() {
        let _ = toggle_resp.clone().on_hover_text(acc_tgl);
    }
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
            value: Some(json!({ "collapsed": geom.row_titles_collapsed })),
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
        fstate.intents.push(UiIntent::SetRowTitlesCollapsed { collapsed: next });
        ui.ctx().request_repaint();
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

pub(super) struct MultiColHeaderView<'a> {
    pub(super) scroll_x: f32,
    pub(super) vis_flat_hint: &'a Range<usize>,
    pub(super) active_mv_operands: &'a HashSet<usize>,
    pub(super) sel: &'a SelectionFastLookup<'a>,
    pub(super) drag_outcome: &'a GridDragOutcome,
}

pub(super) fn render_multi_column_headers(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, view: &MultiColHeaderView<'_>) {
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let hdr_rect = geom.col_header_rect;
    if !hdr_rect.is_positive() {
        return;
    }
    let vis_flat = if view.vis_flat_hint.is_empty() {
        compute_visible_range(view.scroll_x, hdr_rect.width(), geom.subcol_w, geom.total_flat_cols)
    } else {
        view.vis_flat_hint.clone()
    };

    ui.scope_builder(UiBuilder::new().max_rect(hdr_rect), |hdr_ui| {
        hdr_ui.set_clip_rect(hdr_rect);
        hdr_ui.painter().rect_filled(hdr_rect, 0.0, tokens.header_bg);

        let y1 = hdr_rect.min.y;
        let y2 = y1 + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let y4 = hdr_rect.max.y;

        if geom.x_subcols > 0 && vis_flat.start < geom.x_subcols {
            render_multi_x_headers(hdr_ui, mctx, fstate, &vis_flat, view.scroll_x, view.drag_outcome);
        }

        let vis_data_start = vis_flat.start.saturating_sub(geom.x_subcols);
        let vis_data_end = vis_flat.end.saturating_sub(geom.x_subcols);
        if vis_data_start < vis_data_end && geom.col_count > 0 {
            render_multi_data_col_headers(
                hdr_ui,
                mctx,
                fstate,
                vis_data_start..vis_data_end,
                view.scroll_x,
                view.active_mv_operands,
                view.sel,
                view.drag_outcome,
            );
        }

        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y2), pos2(hdr_rect.max.x, y2)], Stroke::new(1.0, tokens.divider));
        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y3), pos2(hdr_rect.max.x, y3)], Stroke::new(1.0, tokens.divider));
        hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y4), pos2(hdr_rect.max.x, y4)], Stroke::new(1.5, tokens.border));
    });
}

fn render_multi_x_headers(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    vis_flat: &Range<usize>,
    scroll_x: f32,
    drag_outcome: &GridDragOutcome,
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
    let x_x0 = hdr_rect.min.x - scroll_x;
    #[allow(clippy::cast_precision_loss)]
    let x_w = (geom.x_subcols as f32) * geom.subcol_w;
    let x_tier1_r = Rect::from_min_max(pos2(x_x0, y1), pos2(x_x0 + x_w, y2));
    let x_tier1_clip = x_tier1_r.intersect(hdr_rect);
    let super_x = if matches!(ds.archetype, TableArchetype::Survival(_)) { "Time" } else { "X" };
    if x_tier1_clip.width() > 2.0 {
        let all_x_sel = geom.x_subcols > 0 && (0..geom.x_subcols).all(|sx| fstate.aux.x_full_subcol_selected(geom, sx));
        let any_x_sel = geom.x_subcols > 0 && (0..geom.x_subcols).any(|sx| fstate.aux.x_contains_subcol(sx));
        if all_x_sel || any_x_sel {
            let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
            hdr_ui.painter().rect_filled(x_tier1_clip, 0.0, fill);
        }
        let t1_resp = hdr_ui.interact(
            x_tier1_clip,
            hdr_ui.make_persistent_id("grid.col_super.x"),
            if blocked { Sense::hover() } else { Sense::click_and_drag() },
        );
        t1_resp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, !blocked, super_x.to_owned()));
        if !drag_outcome.suppress_click_this_frame && !drag_outcome.press_selected_x && t1_resp.clicked() && !blocked && geom.row_count > 0 {
            let last_x_sub = geom.x_subcols.saturating_sub(1);
            fstate.aux.selected_x_anchor = Some((0, 0));
            fstate.aux.selected_x_cell = Some((geom.row_count - 1, last_x_sub));
            super::shared::push_select_x_if_needed(mctx.app, geom.row_count - 1, last_x_sub, fstate.intents);
            set_grid_focus_active(hdr_ui, true);
            hdr_ui.ctx().request_repaint();
        }
        let super_x_col = if (all_x_sel || any_x_sel) && mctx.pane_focused { tokens.text_strong } else { tokens.text_muted };
        render_ellipsized_text(
            hdr_ui,
            x_tier1_clip.shrink2(vec2(4.0, 1.0)),
            super_x,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), super_x_col, Align::Center),
        );
    }

    for sx in 0..geom.x_subcols {
        if !vis_flat.contains(&sx) {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let sx_x0 = x_x0 + (sx as f32) * geom.subcol_w;
        let sx_x1 = sx_x0 + geom.subcol_w;
        let x_col_sel = fstate.aux.x_full_subcol_selected(geom, sx) || fstate.aux.x_contains_subcol(sx);

        let t2_raw = Rect::from_min_max(pos2(sx_x0, y2), pos2(sx_x1, y3));
        let t2_r = t2_raw.intersect(hdr_rect);
        let x_title = x_column_title(&ds.archetype, sx);
        let editing_x_title = fstate.aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::XTitle { subcol: sx });
        if t2_r.width() > 2.0 && !editing_x_title {
            let wid = format!("grid.x_title.{sx}");
            let x_title_editable = !blocked && !ds.frozen && sx == 0;
            let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click_and_drag() });
            let x_sentinel = crate::egui_shell::sheet_find_bar::X_OR_TIME_COL_SENTINEL;
            let x_find_active = sx == 0 && fstate.state.active_find_matches.as_ref().is_some_and(|h| h.active_col_title == Some(x_sentinel));
            let x_find_match =
                x_find_active || (sx == 0 && fstate.state.active_find_matches.as_ref().is_some_and(|h| h.matching_col_titles.contains(&x_sentinel)));
            if x_find_match {
                super::shared::paint_find_highlight(hdr_ui, t2_r, tokens, x_find_active);
            } else if x_col_sel {
                let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
                hdr_ui.painter().rect_filled(t2_r, 0.0, fill);
            } else if resp.hovered() && x_title_editable {
                hdr_ui.painter().rect_filled(t2_r, 0.0, tokens.hover_bg);
            }
            render_ellipsized_text(
                hdr_ui,
                t2_r.shrink2(vec2(4.0, 1.0)),
                &x_title,
                CellTextStyle::plain(FontId::proportional(12.0), tokens.text_strong, Align::Center),
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
            if !drag_outcome.suppress_click_this_frame && resp.clicked() {
                if x_title_editable {
                    fstate.state.inline_edit = None;
                    fstate.aux.aux_edit = Some((AuxInlineTarget::XTitle { subcol: sx }, x_title, true));
                } else if !drag_outcome.press_selected_x && !blocked && geom.row_count > 0 {
                    fstate.aux.selected_x_anchor = Some((0, sx));
                    fstate.aux.selected_x_cell = Some((geom.row_count - 1, sx));
                    super::shared::push_select_x_if_needed(mctx.app, geom.row_count - 1, sx, fstate.intents);
                    set_grid_focus_active(hdr_ui, true);
                    hdr_ui.ctx().request_repaint();
                }
            }
        }

        let t3_r = Rect::from_min_max(pos2(sx_x0, y3), pos2(sx_x1, y4)).intersect(hdr_rect);
        let x_sub_lbl = x_subcolumn_label(&ds.archetype, sx, &mctx.app.locale);
        if t3_r.width() > 2.0 {
            let swid = format!("grid.subcol_hdr.x.{sx}");
            let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), if blocked { Sense::hover() } else { Sense::click_and_drag() });
            if x_col_sel {
                let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
                hdr_ui.painter().rect_filled(t3_r, 0.0, fill);
            }
            let x_sub_col = if x_col_sel && mctx.pane_focused { tokens.text_strong } else { tokens.text_muted };
            render_ellipsized_text(
                hdr_ui,
                t3_r.shrink2(vec2(3.0, 1.0)),
                &x_sub_lbl,
                CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), x_sub_col, Align::Center),
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
            if !drag_outcome.suppress_click_this_frame && !drag_outcome.press_selected_x && sresp.clicked() && !blocked && geom.row_count > 0 {
                fstate.aux.selected_x_anchor = Some((0, sx));
                fstate.aux.selected_x_cell = Some((geom.row_count - 1, sx));
                super::shared::push_select_x_if_needed(mctx.app, geom.row_count - 1, sx, fstate.intents);
                set_grid_focus_active(hdr_ui, true);
                hdr_ui.ctx().request_repaint();
            }
        }
        hdr_ui.painter().line_segment([pos2(sx_x1, y2), pos2(sx_x1, y4)], Stroke::new(1.0, tokens.divider));
    }
    let x_sep_x = x_x0 + x_w;
    hdr_ui.painter().line_segment([pos2(x_sep_x, y1), pos2(x_sep_x, y4)], Stroke::new(1.5, tokens.border));
}

#[allow(clippy::too_many_arguments)]
fn render_multi_data_col_headers(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    vis_data: Range<usize>,
    scroll_x: f32,
    active_mv_operands: &HashSet<usize>,
    sel: &SelectionFastLookup<'_>,
    drag_outcome: &GridDragOutcome,
) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
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
        let full_col_selected = fstate.aux.selected_x_cell.is_none() && sel.contains_full_col(geom, col);
        let any_col_selected = fstate.aux.selected_x_cell.is_none() && sel.contains_col(geom, col);
        let mv_operand = active_mv_operands.contains(&col);
        let col_tinted = mv_operand || full_col_selected;

        if mv_operand {
            let highlight_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y4)).intersect(hdr_rect);
            hdr_ui.painter().rect_filled(highlight_r, 0.0, tokens.accent.linear_multiply(0.14));
        } else if full_col_selected {
            let highlight_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y4)).intersect(hdr_rect);
            let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
            hdr_ui.painter().rect_filled(highlight_r, 0.0, fill);
        } else if any_col_selected {
            let highlight_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y3)).intersect(hdr_rect);
            let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
            hdr_ui.painter().rect_filled(highlight_r, 0.0, fill);
        }

        let t1_raw = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y2));
        let t1_r = t1_raw.intersect(hdr_rect);
        if t1_r.width() > 4.0 {
            render_data_col_tier1_super_header(
                hdr_ui,
                mctx,
                fstate,
                col,
                span_subs,
                full_col_selected || any_col_selected,
                t1_r,
                t1_raw,
                drag_outcome,
            );
        }

        let t2_raw = Rect::from_min_max(pos2(cx0, y2), pos2(cx1, y3));
        let t2_r = t2_raw.intersect(hdr_rect);
        if t2_r.width() > 4.0 && fstate.state.inline_edit.as_ref().is_none_or(|e| e.target != GridInlineEditTarget::ColumnTitle { col }) {
            render_data_col_tier2_title(hdr_ui, mctx, fstate, col, span_subs, t2_r, t2_raw, drag_outcome);
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
            render_subcolumn_tier3_cells(hdr_ui, mctx, fstate, col, span_subs, cx0, sel, col_tinted, drag_outcome);
        }
        hdr_ui.painter().line_segment([pos2(cx1, y1), pos2(cx1, y4)], Stroke::new(1.25, tokens.border));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_data_col_tier1_super_header(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    col: usize,
    span_subs: usize,
    col_selected: bool,
    t1_r: Rect,
    t1_raw: Rect,
    drag_outcome: &GridDragOutcome,
) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let last_row = geom.row_count.saturating_sub(1);
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
    if editing_col_factor {
        render_factor_title_editor(hdr_ui, mctx, fstate, t1_r, &super_wid, "Column factor");
        return;
    }
    let s_sense = if blocked { Sense::hover() } else { Sense::click_and_drag() };
    let s_resp = hdr_ui.interact(t1_r, hdr_ui.make_persistent_id(&super_wid), s_sense);
    if col_factor_editable && s_resp.hovered() {
        hdr_ui.painter().rect_filled(t1_r, CornerRadius::ZERO, tokens.hover_bg);
        let _ = s_resp.clone().on_hover_text("Double-click to edit column factor title");
    }
    let super_fg = if col_selected && mctx.pane_focused { tokens.text_strong } else { tokens.text_muted };
    let (ellip_s, _) = render_ellipsized_text(
        hdr_ui,
        t1_r.shrink2(vec2(4.0, 1.0)),
        &super_lbl,
        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), super_fg, Align::Center),
    );
    if ellip_s {
        fstate.state.ellipsized_headers += 1;
        if !col_factor_editable && s_resp.hovered() {
            let _ = s_resp.clone().on_hover_text(&super_lbl);
        }
    }
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
    if !drag_outcome.suppress_click_this_frame {
        if col_factor_editable
            && is_multi_click_edit(&s_resp)
            && let Some(cf) = col_factor.as_deref()
        {
            begin_factor_title_edit(hdr_ui, fstate, &ds.id, &super_wid, FactorTitleTarget::ColumnFactor { col }, cf);
        } else if s_resp.clicked() && !drag_outcome.press_selected_data && !blocked && geom.row_count > 0 {
            fstate.aux.clear_x_selection();
            fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord: curvebook_tools::GridCellCoord::new(0, col, 0) });
            fstate.intents.push(UiIntent::ExtendGridSelection {
                sheet_id: Some(ds.id.clone()),
                coord: curvebook_tools::GridCellCoord::new(last_row, col, span_subs.saturating_sub(1)),
            });
            set_grid_focus_active(hdr_ui, true);
            hdr_ui.ctx().request_repaint();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_data_col_tier2_title(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    col: usize,
    span_subs: usize,
    t2_r: Rect,
    t2_raw: Rect,
    drag_outcome: &GridDragOutcome,
) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let last_row = geom.row_count.saturating_sub(1);
    let col_title = data_column_title(&ds.archetype, col);
    let wid = format!("grid.col_title.{col}");
    let col_editable = !blocked && !ds.frozen;
    let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click_and_drag() });
    let col_find_active = fstate.state.active_find_matches.as_ref().is_some_and(|h| h.active_col_title == Some(col));
    let col_find_match = col_find_active || fstate.state.active_find_matches.as_ref().is_some_and(|h| h.matching_col_titles.contains(&col));
    if col_find_match {
        paint_find_highlight(hdr_ui, t2_r, tokens, col_find_active);
    } else if resp.hovered() && col_editable {
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
    if resp.hovered() && ellip {
        let _ = resp.clone().on_hover_text(&col_title);
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
    if !drag_outcome.suppress_click_this_frame && resp.clicked() {
        if col_editable {
            fstate.aux.aux_edit = None;
            begin_inline_edit(hdr_ui, fstate.state, GridInlineEditTarget::ColumnTitle { col }, col_title);
        } else if !drag_outcome.press_selected_data && !blocked && geom.row_count > 0 {
            fstate.aux.clear_x_selection();
            fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord: curvebook_tools::GridCellCoord::new(0, col, 0) });
            fstate.intents.push(UiIntent::ExtendGridSelection {
                sheet_id: Some(ds.id.clone()),
                coord: curvebook_tools::GridCellCoord::new(last_row, col, span_subs.saturating_sub(1)),
            });
            set_grid_focus_active(hdr_ui, true);
            hdr_ui.ctx().request_repaint();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_subcolumn_tier3_cells(
    hdr_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    col: usize,
    span_subs: usize,
    cx0: f32,
    sel: &SelectionFastLookup<'_>,
    col_already_tinted: bool,
    drag_outcome: &GridDragOutcome,
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
    let is_column = matches!(ds.archetype, TableArchetype::Column(_));
    let last_row = geom.row_count.saturating_sub(1);
    for subcol in 0..span_subs {
        #[allow(clippy::cast_precision_loss)]
        let sx0 = cx0 + (subcol as f32) * geom.subcol_w;
        let sx1 = sx0 + geom.subcol_w;
        let t3_raw = Rect::from_min_max(pos2(sx0, y3), pos2(sx1, y4));
        let t3_r = t3_raw.intersect(hdr_rect);
        if t3_r.width() <= 2.0 {
            continue;
        }
        let editing_sub = fstate.aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::SubcolumnTitle { col, subcol });
        if editing_sub {
            continue;
        }
        let sub_lbl = data_subcolumn_header_label(ds, col, subcol, &app.locale);
        let swid = if is_nested || is_column { format!("grid.subcol_title.{col}.{subcol}") } else { format!("grid.subcol_hdr.{col}.{subcol}") };
        let sub_editable = is_nested && !blocked && !ds.frozen;
        let sense = if blocked { Sense::hover() } else { Sense::click_and_drag() };
        let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), sense);
        let flat_data_col = geom.coord_to_flat_all(false, col, subcol).saturating_sub(geom.x_subcols);
        let subcol_selected =
            fstate.aux.selected_x_cell.is_none() && (sel.contains_full_flat_col(geom, flat_data_col) || sel.contains_flat_col(geom, flat_data_col));
        if !col_already_tinted && subcol_selected {
            let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
            hdr_ui.painter().rect_filled(t3_r, 0.0, fill);
        } else if is_nested && sresp.hovered() && sub_editable {
            hdr_ui.painter().rect_filled(t3_r, 0.0, tokens.hover_bg);
        }
        let fg = if (col_already_tinted || subcol_selected) && mctx.pane_focused {
            tokens.text_strong
        } else if is_nested {
            tokens.text_primary
        } else {
            tokens.text_muted
        };
        let (sub_elided, _) = render_ellipsized_text(
            hdr_ui,
            t3_r.shrink2(vec2(3.0, 1.0)),
            &sub_lbl,
            CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), fg, Align::Center),
        );
        if sub_elided {
            fstate.state.ellipsized_headers += 1;
        }
        if sresp.hovered() {
            if let TableArchetype::Column(col_t) = &ds.archetype {
                let hint_tag = column_design_hint_tag(col_t);
                let _ = sresp.clone().on_hover_text(format!("{sub_lbl} · {hint_tag}"));
            } else if sub_elided {
                let _ = sresp.clone().on_hover_text(&sub_lbl);
            }
        }
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
                ellipsized: sub_elided,
                full_label: sub_elided.then_some(sub_lbl.as_str()),
            },
        );
        if !drag_outcome.suppress_click_this_frame && sresp.clicked() {
            if sub_editable {
                fstate.state.inline_edit = None;
                fstate.aux.aux_edit = Some((AuxInlineTarget::SubcolumnTitle { col, subcol }, sub_lbl, true));
            } else if !drag_outcome.press_selected_data && !blocked && geom.row_count > 0 {
                fstate.aux.clear_x_selection();
                fstate
                    .intents
                    .push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord: curvebook_tools::GridCellCoord::new(0, col, subcol) });
                fstate.intents.push(UiIntent::ExtendGridSelection {
                    sheet_id: Some(ds.id.clone()),
                    coord: curvebook_tools::GridCellCoord::new(last_row, col, subcol),
                });
                set_grid_focus_active(hdr_ui, true);
                hdr_ui.ctx().request_repaint();
            }
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
        TableArchetype::Column(_) => format!("Col {letter}"),
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
        TableArchetype::Column(t) => {
            let raw = t.data_sets.get(col).map_or("", |d| d.title.as_str());
            if raw.is_empty() { default_column_label(col) } else { raw.to_owned() }
        }
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
