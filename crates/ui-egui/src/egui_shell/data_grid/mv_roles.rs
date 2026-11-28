//! Multiple-Variables role chips, role-override popup menu, and calculated-column operand
//! highlighting (`R15.4` / `R16.7`).

use std::collections::HashSet;

use curvebook_doc::{DataSheet, TableArchetype, VariableColumn, VariableRole, VariableSource};
use egui::{Align, Id, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::super::ui_kit::{ActionMenuAnchor, ActionMenuEntry, ActionMenuItem, ActionMenuSpec, action_menu_popup};
use super::super::{FrameInspectEntryInput, UiIntent};
use super::shared::{MultiGridCtx, MultiGridFrameState, multi_text_style, record_meta_with_unclipped_if_enabled, render_ellipsized_text};

pub(super) fn active_mv_formula_operands(app: &UiApp, ds: &DataSheet) -> HashSet<usize> {
    let mut out = HashSet::new();
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else { return out };
    let formula_opt = if app.grid.formula_bar.active && !app.grid.formula_bar.input.trim().is_empty() {
        Some(app.grid.formula_bar.input.as_str())
    } else if let Some(active) = app.grid.active_cell_for_sheet(&ds.id) {
        mv.variables.get(active.col).and_then(|v| match &v.source {
            VariableSource::Calculated { formula, .. } => Some(formula.as_str()),
            VariableSource::Entered => None,
        })
    } else {
        None
    };
    let Some(formula) = formula_opt else { return out };
    for (idx, var) in mv.variables.iter().enumerate() {
        let bracketed = format!("[{}]", var.title);
        if formula.contains(&bracketed) || formula.contains(&var.title) {
            out.insert(idx);
        }
    }
    out
}

pub(super) fn render_mv_tier3_role_chip(
    ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    col: usize,
    var: &VariableColumn,
    rect: Rect,
    unclipped_rect: Rect,
) {
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let is_calc = matches!(var.source, VariableSource::Calculated { .. });
    let (chip_txt, chip_col) = if is_calc {
        ("FX=", tokens.role_fx)
    } else {
        let short = match var.role {
            VariableRole::Continuous => "NUM",
            VariableRole::Categorical => "CAT",
            VariableRole::Label => "LBL",
            VariableRole::Calculated => "FX=",
        };
        let c = match var.role {
            VariableRole::Continuous => tokens.role_num,
            VariableRole::Categorical => tokens.role_cat,
            VariableRole::Label => tokens.role_lbl,
            VariableRole::Calculated => tokens.role_fx,
        };
        (short, c)
    };
    let label = if !is_calc && var.role_inferred { format!("Auto·{chip_txt}") } else { chip_txt.to_owned() };
    let wid = format!("mv_formula.role_badge.{col}");
    let resp = ui.interact(rect, ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
    let pill_rect = rect.shrink(tokens.metrics.space_xxs);
    let alpha = if is_calc || !var.role_inferred { 0.22 } else { 0.10 };
    if pill_rect.is_positive() {
        let is_num = !is_calc && var.role == VariableRole::Continuous;
        let fill_col = if is_num { tokens.surface_recessed } else { chip_col.linear_multiply(alpha) };
        let stroke_col = if is_num { tokens.border } else { chip_col };
        ui.painter().rect_filled(pill_rect, tokens.metrics.radius_sm, fill_col);
        let stroke_w = if !is_calc && var.role_inferred { tokens.metrics.stroke_thin_pt } else { 1.5 };
        ui.painter().rect_stroke(pill_rect, tokens.metrics.radius_sm, Stroke::new(stroke_w, stroke_col), StrokeKind::Inside);
    }
    render_ellipsized_text(ui, rect.shrink2(vec2(4.0, 0.0)), &label, multi_text_style(11.5, false, chip_col, Align::Center, is_calc, false));
    let acc = format!("Variable {} role: {label}", var.title);
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
    record_meta_with_unclipped_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &resp,
        unclipped_rect,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "button",
            label: &label,
            command_id: Some(if is_calc { "ui.table.openFormulaBar" } else { "table.setVariableRole" }),
            value: Some(serde_json::Value::String(label.clone())),
            enabled: !blocked,
            disabled_reason: blocked.then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    ui.ctx().data_mut(|d| d.insert_temp(Id::unique(("mv_role_anchor", mctx.ds.id.as_str(), col)), rect));
    if resp.clicked() && !blocked {
        if is_calc {
            fstate.aux.active_role_menu_col = None;
            fstate.intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col }));
        } else if !mctx.ds.frozen {
            let opening = fstate.aux.active_role_menu_col != Some(col);
            fstate.aux.active_role_menu_col = opening.then_some(col);
            if opening {
                let frame_id = ui.ctx().cumulative_frame_nr();
                ui.ctx().data_mut(|d| d.insert_temp(Id::unique(("mv_role_open_frame", mctx.ds.id.as_str())), frame_id));
            }
        }
    } else if resp.secondary_clicked() && !blocked && is_calc {
        let opening = fstate.aux.active_role_menu_col != Some(col);
        fstate.aux.active_role_menu_col = opening.then_some(col);
        if opening {
            let frame_id = ui.ctx().cumulative_frame_nr();
            ui.ctx().data_mut(|d| d.insert_temp(Id::unique(("mv_role_open_frame", mctx.ds.id.as_str())), frame_id));
        }
    }
}

pub(super) fn render_mv_role_menu_popup(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let Some(col) = fstate.aux.active_role_menu_col else { return };
    let TableArchetype::MultipleVariables(mv) = &mctx.ds.archetype else {
        fstate.aux.active_role_menu_col = None;
        return;
    };
    let is_calc = mv.variables.get(col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }));
    if mctx.blocked || (!is_calc && mctx.ds.frozen) {
        fstate.aux.active_role_menu_col = None;
        return;
    }
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let scroll_x = fstate.state.scroll_offset_pt.x;
    let (flat, _) = geom.col_span_flat(col);
    #[allow(clippy::cast_precision_loss)]
    let fallback_x0 = (geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - scroll_x)
        .clamp(geom.grid_outer_rect.min.x, (geom.grid_outer_rect.max.x - geom.subcol_w).max(geom.grid_outer_rect.min.x));
    let fallback_anchor = Rect::from_min_max(
        pos2(fallback_x0, (geom.col_header_rect.max.y - tokens.metrics.min_hit_target_pt).max(geom.col_header_rect.min.y)),
        pos2(fallback_x0 + geom.subcol_w, geom.col_header_rect.max.y),
    );
    let anchor_rect = ui.ctx().data(|d| d.get_temp::<Rect>(Id::unique(("mv_role_anchor", mctx.ds.id.as_str(), col)))).unwrap_or(fallback_anchor);
    let opened_this_frame =
        ui.ctx().data(|d| d.get_temp::<u64>(Id::unique(("mv_role_open_frame", mctx.ds.id.as_str())))) == Some(ui.ctx().cumulative_frame_nr());

    let manage_wid = format!("mv_formula.role_manage.{col}");
    if is_calc {
        let edit_wid = format!("mv_formula.role_edit_formula.{col}");
        let unlink_wid = format!("mv_formula.role_unlink.{col}");
        let frozen_reason = mctx.ds.frozen.then_some("Data sheet is frozen (read-only).");
        let entries = [
            ActionMenuEntry::Item(
                ActionMenuItem::menu_item(&edit_wid, tr(mctx.locale, "mv.panel.edit_formula")).command_id(Some("ui.table.openFormulaBar")),
            ),
            ActionMenuEntry::Item(
                ActionMenuItem::menu_item(&unlink_wid, tr(mctx.locale, "cmd.table.unlinkCalculatedVariable"))
                    .command_id(Some("table.unlinkCalculatedVariable"))
                    .enabled(!mctx.ds.frozen)
                    .disabled_reason(frozen_reason),
            ),
            ActionMenuEntry::Separator,
            ActionMenuEntry::Item(ActionMenuItem::menu_item(&manage_wid, "Manage Variables & Levels")),
        ];
        let out = action_menu_popup(
            ui.ctx(),
            tokens,
            fstate.state.record_inspect_meta.then_some(fstate.meta),
            ActionMenuSpec::dropdown(
                Id::unique(("mv_role_popup", mctx.ds.id.as_str(), col)),
                "popup.mv.role",
                "Calculated Variable",
                ActionMenuAnchor::Trigger(anchor_rect),
            )
            .allow_outside_dismiss(!opened_this_frame),
            &entries,
        );
        if let Some(hit) = out.clicked_entry {
            match hit {
                0 => {
                    fstate.intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col }));
                    fstate.aux.active_role_menu_col = None;
                }
                1 if !mctx.ds.frozen => {
                    fstate.intents.push_engine("table.unlinkCalculatedVariable", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col }));
                    fstate.aux.active_role_menu_col = None;
                }
                3 => {
                    fstate.intents.push(UiIntent::OpenManageVariables { sheet_id: mctx.ds.id.clone(), column: Some(col) });
                    fstate.aux.active_role_menu_col = None;
                }
                _ => {}
            }
        } else if out.dismissed {
            fstate.aux.active_role_menu_col = None;
        }
        return;
    }

    let opts =
        [("auto", "Auto (Infer role)"), ("continuous", "NUM · Continuous"), ("categorical", "CAT · Categorical"), ("label", "LBL · Label / ID")];
    let opt_wids: Vec<String> = opts.iter().map(|(slug, _)| format!("mv_formula.role_opt.{col}.{slug}")).collect();

    let mut entries = Vec::with_capacity(opts.len() + 2);
    for ((slug, text), wid) in opts.iter().zip(opt_wids.iter()) {
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item(wid, text).command_id(Some("table.setVariableRole")).value(Some(serde_json::Value::String((*slug).to_owned()))),
        ));
    }
    entries.push(ActionMenuEntry::Separator);
    entries.push(ActionMenuEntry::Item(ActionMenuItem::menu_item(&manage_wid, "Manage Variables & Levels")));

    let out = action_menu_popup(
        ui.ctx(),
        tokens,
        fstate.state.record_inspect_meta.then_some(fstate.meta),
        ActionMenuSpec::dropdown(
            Id::unique(("mv_role_popup", mctx.ds.id.as_str(), col)),
            "popup.mv.role",
            "Variable Role",
            ActionMenuAnchor::Trigger(anchor_rect),
        )
        .allow_outside_dismiss(!opened_this_frame),
        &entries,
    );

    if let Some(hit) = out.clicked_entry {
        if let Some((slug, _)) = opts.get(hit) {
            fstate.intents.push_engine("table.setVariableRole", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col, "role": slug }));
            fstate.aux.active_role_menu_col = None;
        } else if hit == opts.len() + 1 {
            fstate.intents.push(UiIntent::OpenManageVariables { sheet_id: mctx.ds.id.clone(), column: Some(col) });
            fstate.aux.active_role_menu_col = None;
        }
    } else if out.dismissed {
        fstate.aux.active_role_menu_col = None;
    }
}
