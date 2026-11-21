//! Multiple-Variables role chips, role-override popup menu, and calculated-column operand
//! highlighting (`R15.4` / `R16.7`).

use std::collections::HashSet;

use curvebook_doc::{DataSheet, TableArchetype, VariableColumn, VariableRole, VariableSource};
use egui::{Align, Id, Order, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::inspect::ZLayer;

use super::super::FrameInspectEntryInput;
use super::shared::{MultiGridCtx, MultiGridFrameState, multi_text_style, record_meta_if_enabled, render_ellipsized_text};

pub(super) fn active_mv_formula_operands(app: &UiApp, ds: &DataSheet) -> HashSet<usize> {
    let mut out = HashSet::new();
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else { return out };
    let formula_opt = if app.grid.formula_bar.active && !app.grid.formula_bar.input.trim().is_empty() {
        Some(app.grid.formula_bar.input.as_str())
    } else if let Some(active) = app.grid.tool.active {
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
    let alpha = if is_calc || !var.role_inferred { 0.22 } else { 0.10 };
    ui.painter().rect_filled(rect, 2.0, chip_col.linear_multiply(alpha));
    let stroke_w = if !is_calc && var.role_inferred { 1.0 } else { 1.5 };
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(stroke_w, chip_col), StrokeKind::Inside);
    render_ellipsized_text(ui, rect.shrink2(vec2(4.0, 0.0)), &label, multi_text_style(11.5, false, chip_col, Align::Center, is_calc, false));
    let acc = format!("Variable {} role: {label}", var.title);
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
    record_meta_if_enabled(
        ui,
        fstate.meta,
        fstate.state.record_inspect_meta,
        &resp,
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
    if resp.clicked() && !blocked {
        if is_calc {
            fstate.aux.active_role_menu_col = None;
            fstate.intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col }));
        } else {
            fstate.aux.active_role_menu_col = if fstate.aux.active_role_menu_col == Some(col) { None } else { Some(col) };
        }
    }
}

pub(super) fn render_mv_role_menu_popup(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let Some(col) = fstate.aux.active_role_menu_col else { return };
    if mctx.blocked || !matches!(mctx.ds.archetype, TableArchetype::MultipleVariables(_)) {
        fstate.aux.active_role_menu_col = None;
        return;
    }
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let scroll_x = fstate.state.scroll_offset_pt.x;
    let (flat, _) = geom.col_span_flat(col);
    #[allow(clippy::cast_precision_loss)]
    let x0 = (geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - scroll_x)
        .clamp(geom.grid_outer_rect.min.x, (geom.grid_outer_rect.max.x - 180.0).max(geom.grid_outer_rect.min.x));
    let pos = pos2(x0, geom.col_header_rect.max.y + 2.0);
    egui::Area::new(Id::unique(("mv_role_popup", mctx.ds.id.as_str(), col))).fixed_pos(pos).order(Order::Foreground).show(ui.ctx(), |pui| {
        egui::Frame::popup(pui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).show(pui, |m| {
            m.set_min_width(172.0);
            for (slug, text) in [
                ("auto", "Auto (Infer role)"),
                ("continuous", "NUM · Continuous"),
                ("categorical", "CAT · Categorical"),
                ("label", "LBL · Label / ID"),
            ] {
                let wid = format!("mv_formula.role_opt.{col}.{slug}");
                let r = m.button(text);
                record_meta_if_enabled(
                    m,
                    fstate.meta,
                    true,
                    &r,
                    FrameInspectEntryInput {
                        widget_id: &wid,
                        role: "menuitem",
                        label: text,
                        command_id: Some("table.setVariableRole"),
                        value: Some(serde_json::Value::String(slug.to_owned())),
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::FloatingOverlay,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if r.clicked() {
                    fstate.intents.push_engine("table.setVariableRole", json!({ "sheet_id": mctx.ds.id.as_str(), "col": col, "role": slug }));
                    fstate.aux.active_role_menu_col = None;
                }
            }
            m.separator();
            let mwid = format!("mv_formula.role_manage.{col}");
            let mr = m.button("Manage Variables & Levels…");
            record_meta_if_enabled(
                m,
                fstate.meta,
                true,
                &mr,
                FrameInspectEntryInput {
                    widget_id: &mwid,
                    role: "menuitem",
                    label: "Manage Variables & Levels…",
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::FloatingOverlay,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if mr.clicked() {
                fstate.intents.push_open_manage_variables(mctx.ds.id.clone(), Some(col));
                fstate.aux.active_role_menu_col = None;
            }
        });
    });
}
