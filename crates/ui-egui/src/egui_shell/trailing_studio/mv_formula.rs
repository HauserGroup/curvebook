//! Multiple-Variables (MV) variable & formula studio surface.

use std::collections::BTreeSet;

use curvebook_doc::{DataSheet, LevelOrderKind, MultipleVariablesTable, VariableColumn, VariableRole, VariableSource};
use curvebook_transform::{DependencyTarget, extract_dependencies, parse_formula, validate_no_formula_cycle};
use egui::{Stroke, Vec2};
use serde_json::json;

use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, OKABE_ITO_SERIES_PALETTE, PendingUiIntents, widgets};
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
use super::{MvOperandHighlight, TrailingStudioContext, TrailingStudioEguiState};

/// Extract colored column operand highlights from an MV formula expression in deterministic palette order.
#[must_use]
pub fn resolve_formula_operand_highlights(vars: &[VariableColumn], formula: &str) -> Vec<MvOperandHighlight> {
    let Ok(expr) = parse_formula(formula) else {
        return Vec::new();
    };
    let deps = extract_dependencies(&expr);
    let mut seen_cols = BTreeSet::new();
    let mut out = Vec::new();

    for dep in deps {
        let resolved_idx = match dep {
            DependencyTarget::ColumnIndex(idx) if idx < vars.len() => Some(idx),
            DependencyTarget::ColumnName(ref name) => {
                let needle = name.trim();
                vars.iter().position(|v| v.title.trim().eq_ignore_ascii_case(needle))
            }
            DependencyTarget::InfoConstant(_) | DependencyTarget::ColumnIndex(_) => None,
        };
        if let Some(idx) = resolved_idx
            && seen_cols.insert(idx)
        {
            let swatch = OKABE_ITO_SERIES_PALETTE[idx % OKABE_ITO_SERIES_PALETTE.len()];
            let [r, g, b, _] = swatch.to_array();
            out.push(MvOperandHighlight {
                variable_index: idx,
                variable_title: vars[idx].title.clone(),
                color_rgb: [r, g, b],
                color_hex: format!("#{r:02X}{g:02X}{b:02X}"),
            });
        }
    }
    out
}

struct FormulaFnEntry {
    name: &'static str,
    signature: &'static str,
    insert: &'static str,
    summary: &'static str,
}

const STUDIO_FORMULA_FUNCS: &[FormulaFnEntry] = &[
    FormulaFnEntry { name: "ln", signature: "ln(x)", insert: "ln(", summary: "Natural logarithm base e" },
    FormulaFnEntry { name: "log10", signature: "log10(x)", insert: "log10(", summary: "Common base-10 logarithm" },
    FormulaFnEntry { name: "exp", signature: "exp(x)", insert: "exp(", summary: "Exponential e^x" },
    FormulaFnEntry { name: "sqrt", signature: "sqrt(x)", insert: "sqrt(", summary: "Square root of x >= 0" },
    FormulaFnEntry { name: "abs", signature: "abs(x)", insert: "abs(", summary: "Absolute magnitude |x|" },
    FormulaFnEntry { name: "mean", signature: "mean(var)", insert: "mean(", summary: "Column arithmetic mean" },
    FormulaFnEntry { name: "sd", signature: "sd(var)", insert: "sd(", summary: "Sample standard deviation" },
    FormulaFnEntry { name: "sem", signature: "sem(var)", insert: "sem(", summary: "Standard error of the mean" },
    FormulaFnEntry { name: "median", signature: "median(var)", insert: "median(", summary: "Sample median (50th pct)" },
    FormulaFnEntry { name: "iqr", signature: "iqr(var)", insert: "iqr(", summary: "Interquartile range Q3 - Q1" },
    FormulaFnEntry { name: "if", signature: "if(cond, t, f)", insert: "if(", summary: "Conditional branch per row" },
    FormulaFnEntry { name: "coalesce", signature: "coalesce(a, b)", insert: "coalesce(", summary: "First non-missing value" },
    FormulaFnEntry { name: "Info", signature: "Info(\"Key\")", insert: "Info(\"", summary: "Linked InfoSheet constant" },
    FormulaFnEntry { name: "countif", signature: "countif(cond)", insert: "countif(", summary: "Conditional row count" },
    FormulaFnEntry { name: "sumif", signature: "sumif(var, cond)", insert: "sumif(", summary: "Conditional column sum" },
    FormulaFnEntry { name: "meanif", signature: "meanif(var, cond)", insert: "meanif(", summary: "Conditional column mean" },
];

/// Return the canonical formula signature shown by the trailing Formula Studio (`#14`).
#[must_use]
pub fn studio_formula_signature(name: &str) -> Option<&'static str> {
    STUDIO_FORMULA_FUNCS.iter().find(|entry| entry.name == name).map(|entry| entry.signature)
}

fn role_label(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "Continuous",
        VariableRole::Categorical => "Categorical",
        VariableRole::Label => "Label",
        VariableRole::Calculated => "Calculated",
    }
}

pub(super) fn render_mv_formula_studio(
    ui: &mut egui::Ui,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let editable = ctx_in.enabled && !ds.frozen;
    let frozen_reason = ds.frozen.then_some("data sheet is frozen (read-only)");
    let col_count = mv.variables.len();
    if col_count > 0 && state.mv_selected_col >= col_count {
        state.mv_selected_col = 0;
    }
    let active_col = state.mv_selected_col;

    // Sync formula & title drafts cleanly whenever selected sheet or column changes.
    if state.mv_synced_sheet.as_ref() != Some(&ds.id) || state.mv_synced_col != Some(active_col) {
        state.mv_synced_sheet = Some(ds.id.clone());
        state.mv_synced_col = Some(active_col);
        if let Some(var) = mv.variables.get(active_col) {
            state.mv_title_draft = var.title.clone();
            state.mv_formula_draft = match &var.source {
                VariableSource::Calculated { formula } => formula.clone(),
                VariableSource::Entered => String::new(),
            };
        }
    }

    // Keep live colored operand column outline highlights synced with the active formula!
    let effective_formula = if !state.mv_formula_draft.trim().is_empty() {
        state.mv_formula_draft.as_str()
    } else if let Some(VariableSource::Calculated { formula }) = mv.variables.get(active_col).map(|v| &v.source) {
        formula.as_str()
    } else {
        ""
    };
    state.mv_operand_highlights = resolve_formula_operand_highlights(&mv.variables, effective_formula);

    // Variable selector row
    widgets::subheader(ui, tr(locale, "mv.panel.var_name"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let max_w = (ui.available_width() * 0.48).max(60.0);
        for (idx, var) in mv.variables.iter().enumerate() {
            let hl = state.mv_operand_highlights.iter().find(|h| h.variable_index == idx).map(MvOperandHighlight::color32);
            let wid = format!("inspector.mv.col.{idx}");
            let r = studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: &var.title,
                    tooltip: &var.title,
                    max_w,
                    selected: idx == active_col,
                    enabled: ctx_in.enabled,
                    border_override: hl,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            );
            if r {
                state.mv_selected_col = idx;
                state.mv_synced_col = None;
            }
        }
    });

    ui.add_space(4.0);
    let sub_tabs = [tr(locale, "mv.editor_btn"), "Role & Levels"];
    if let Some(next_sub) = widgets::tab_bar(ui, &sub_tabs, state.mv_sub_tab) {
        state.mv_sub_tab = next_sub;
    }
    ui.add_space(4.0);
    if ds.frozen {
        ui.colored_label(tokens.text_muted, "Frozen (read-only): formula and role edits are locked.");
        ui.add_space(2.0);
    }

    if state.mv_sub_tab == 0 {
        let mut resp_title = egui::TextEdit::singleline(&mut state.mv_title_draft).desired_width(ui.available_width()).hint_text("Variable title");
        if !editable {
            resp_title = resp_title.interactive(false);
        }
        ui.add(resp_title);

        ui.add_space(4.0);
        widgets::subheader(ui, "f(x) =");
        let mut formula_edit = egui::TextEdit::singleline(&mut state.mv_formula_draft).desired_width(ui.available_width()).hint_text("= [Dose] * 2");
        if !editable {
            formula_edit = formula_edit.interactive(false);
        }
        let f_resp = ui.add(formula_edit);
        if f_resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
            meta.record_response(
                ui,
                &f_resp,
                FrameInspectEntryInput {
                    widget_id: "inspector.mv.formula_input",
                    role: "textbox",
                    label: "f(x) formula",
                    command_id: Some("table.setCalculatedVariable"),
                    value: Some(json!(state.mv_formula_draft)),
                    enabled: editable,
                    disabled_reason: frozen_reason,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }

        // Recompute operand highlights after user edit inside same frame
        state.mv_operand_highlights = resolve_formula_operand_highlights(&mv.variables, &state.mv_formula_draft);

        // Live validation + cycle badge
        let formula_status = if state.mv_formula_draft.trim().is_empty() {
            Ok("Enter an expression referencing [Columns]".to_owned())
        } else if active_col < col_count {
            validate_no_formula_cycle(mv, active_col, &state.mv_formula_draft)
                .map(|()| format!("Valid formula • {} operand highlight(s)", state.mv_operand_highlights.len()))
                .map_err(|e| e.to_string())
        } else {
            parse_formula(&state.mv_formula_draft).map(|_| String::from("Valid formula")).map_err(|e| e.to_string())
        };
        match &formula_status {
            Ok(msg) => ui.label(egui::RichText::new(msg).size(tokens.metrics.font_ui_small_pt).color(tokens.link_green)),
            Err(err) => ui.label(egui::RichText::new(err).size(tokens.metrics.font_ui_small_pt).color(tokens.link_red)),
        };

        // Colored operand column outline chips
        if !state.mv_operand_highlights.is_empty() {
            ui.add_space(2.0);
            widgets::dim_label(ui, "Operand Highlights");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (i, hl) in state.mv_operand_highlights.iter().enumerate() {
                    let lbl = format!("[{}] {}", hl.variable_title, hl.color_hex);
                    let wid = format!("inspector.mv.operand_highlight.{i}");
                    let _ = studio_chip(
                        ui,
                        tokens,
                        meta,
                        &StudioChipSpec {
                            label: &lbl,
                            tooltip: &format!("Outlined operand column `{}` ({})", hl.variable_title, hl.color_hex),
                            max_w: ui.available_width(),
                            selected: false,
                            enabled: true,
                            border_override: Some(hl.color32()),
                            widget_id: &wid,
                            role: "text",
                            command_id: None,
                            disabled_reason: None,
                        },
                    );
                }
            });
        }

        let can_apply = editable && !state.mv_formula_draft.trim().is_empty() && formula_status.is_ok();
        let apply_reason = frozen_reason.or_else(|| (!can_apply).then_some("enter a valid non-circular formula"));
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: "Apply Formula",
                tooltip: "Save calculated variable formula (table.setCalculatedVariable)",
                selected: false,
                enabled: can_apply,
                primary: true,
                widget_id: "inspector.mv.apply_formula",
                command_id: Some("table.setCalculatedVariable"),
                disabled_reason: apply_reason,
            },
        ) {
            let title =
                if state.mv_title_draft.trim().is_empty() { format!("Calc {}", col_count + 1) } else { state.mv_title_draft.trim().to_owned() };
            let mut params = json!({
                "sheet_id": ds.id.as_str(),
                "title": title,
                "formula": state.mv_formula_draft.trim(),
            });
            if active_col < col_count
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("col"), json!(active_col));
            }
            intents.push_engine("table.setCalculatedVariable", params);
        }

        ui.add_space(6.0);
        widgets::subheader(ui, tr(locale, "mv.panel.column_vars"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            let max_w = (ui.available_width() * 0.48).max(56.0);
            for (idx, var) in mv.variables.iter().enumerate() {
                let hl = state.mv_operand_highlights.iter().find(|h| h.variable_index == idx).map(MvOperandHighlight::color32);
                let wid = format!("inspector.mv.insert_var.{idx}");
                let r = studio_chip(
                    ui,
                    tokens,
                    meta,
                    &StudioChipSpec {
                        label: &format!("[{}]", var.title),
                        tooltip: "Insert variable reference into formula",
                        max_w,
                        selected: false,
                        enabled: editable,
                        border_override: hl,
                        widget_id: &wid,
                        role: "button",
                        command_id: None,
                        disabled_reason: frozen_reason,
                    },
                );
                if r {
                    if !state.mv_formula_draft.is_empty() && !state.mv_formula_draft.ends_with(['(', ' ', '+', '-', '*', '/']) {
                        state.mv_formula_draft.push(' ');
                    }
                    state.mv_formula_draft.push('[');
                    state.mv_formula_draft.push_str(&var.title);
                    state.mv_formula_draft.push(']');
                }
            }
        });

        ui.add_space(4.0);
        widgets::subheader(ui, tr(locale, "mv.panel.func_ref"));
        ui.add(egui::TextEdit::singleline(&mut state.mv_fn_filter).desired_width(ui.available_width()).hint_text("Search functions..."));
        let filter_lower = state.mv_fn_filter.trim().to_ascii_lowercase();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (idx, func) in STUDIO_FORMULA_FUNCS.iter().enumerate() {
                if !filter_lower.is_empty() && !func.name.contains(&filter_lower) && !func.summary.to_ascii_lowercase().contains(&filter_lower) {
                    continue;
                }
                let wid = format!("inspector.mv.fn.{idx}");
                let r = studio_chip(
                    ui,
                    tokens,
                    meta,
                    &StudioChipSpec {
                        label: func.name,
                        tooltip: func.summary,
                        max_w: 84.0,
                        selected: state.mv_selected_fn == func.name,
                        enabled: editable,
                        border_override: None,
                        widget_id: &wid,
                        role: "button",
                        command_id: None,
                        disabled_reason: frozen_reason,
                    },
                );
                if r {
                    state.mv_selected_fn = String::from(func.name);
                    state.mv_formula_draft.push_str(func.insert);
                }
            }
        });

        if let Some(spec) = STUDIO_FORMULA_FUNCS.iter().find(|f| f.name == state.mv_selected_fn) {
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.divider))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(spec.signature).size(tokens.metrics.font_ui_small_pt).strong().color(tokens.text_strong));
                    ui.label(egui::RichText::new(spec.summary).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
                });
        }
    } else if let Some(var) = mv.variables.get(active_col) {
        // Role & Levels sub-tab
        let cur_role = var.role;
        ui.label(egui::RichText::new(format!("{} ({})", var.title, role_label(cur_role))).size(12.0).strong().color(tokens.text_strong));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (slug, lbl, r_kind) in [
                ("continuous", "Continuous", VariableRole::Continuous),
                ("categorical", "Categorical", VariableRole::Categorical),
                ("label", "Label", VariableRole::Label),
            ] {
                let wid = format!("inspector.mv.role.{slug}");
                let role_enabled = editable && cur_role != VariableRole::Calculated;
                let role_reason =
                    frozen_reason.or_else(|| (cur_role == VariableRole::Calculated).then_some("calculated columns lock Calculated role"));
                let r = studio_chip(
                    ui,
                    tokens,
                    meta,
                    &StudioChipSpec {
                        label: lbl,
                        tooltip: "Assign variable semantic role",
                        max_w: 88.0,
                        selected: cur_role == r_kind,
                        enabled: role_enabled,
                        border_override: None,
                        widget_id: &wid,
                        role: "button",
                        command_id: Some("table.setVariableRole"),
                        disabled_reason: role_reason,
                    },
                );
                if r {
                    intents.push_engine("table.setVariableRole", json!({ "sheet_id": ds.id.as_str(), "col": active_col, "role": slug }));
                }
            }
        });

        if cur_role == VariableRole::Categorical {
            ui.add_space(6.0);
            widgets::subheader(ui, tr(locale, "mv.vars.order_mode"));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for (order_slug, lbl, active) in [
                    ("entry_order", "Entry", matches!(var.level_order.order, LevelOrderKind::EntryOrder)),
                    ("alphabetical", "A-Z", matches!(var.level_order.order, LevelOrderKind::Alphabetical)),
                    ("frequency_descending", "Freq ↓", matches!(var.level_order.order, LevelOrderKind::FrequencyDescending)),
                    ("frequency_ascending", "Freq ↑", matches!(var.level_order.order, LevelOrderKind::FrequencyAscending)),
                ] {
                    let wid = format!("inspector.mv.order.{order_slug}");
                    let r = studio_chip(
                        ui,
                        tokens,
                        meta,
                        &StudioChipSpec {
                            label: lbl,
                            tooltip: "Set categorical level ordering",
                            max_w: 74.0,
                            selected: active,
                            enabled: editable,
                            border_override: None,
                            widget_id: &wid,
                            role: "button",
                            command_id: Some("table.setCategoricalOrder"),
                            disabled_reason: frozen_reason,
                        },
                    );
                    if r {
                        intents.push_engine(
                            "table.setCategoricalOrder",
                            json!({ "sheet_id": ds.id.as_str(), "col": active_col, "order": order_slug, "reversed": var.level_order.reversed }),
                        );
                    }
                }
            });

            ui.add_space(4.0);
            let mut from_edit =
                egui::TextEdit::singleline(&mut state.mv_recode_from).desired_width(ui.available_width()).hint_text("Recode from level");
            if !editable {
                from_edit = from_edit.interactive(false);
            }
            ui.add(from_edit);
            let mut to_edit = egui::TextEdit::singleline(&mut state.mv_recode_to).desired_width(ui.available_width()).hint_text("Recode to level");
            if !editable {
                to_edit = to_edit.interactive(false);
            }
            ui.add(to_edit);
            let can_recode = editable && !state.mv_recode_from.trim().is_empty() && !state.mv_recode_to.trim().is_empty();
            let recode_reason = frozen_reason.or_else(|| (!can_recode).then_some("enter source and target level names"));
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "Recode Level",
                    tooltip: "Apply categorical level recoding (table.recodeLevels)",
                    selected: false,
                    enabled: can_recode,
                    primary: false,
                    widget_id: "inspector.mv.recode_apply",
                    command_id: Some("table.recodeLevels"),
                    disabled_reason: recode_reason,
                },
            ) {
                let mut map = serde_json::Map::new();
                map.insert(state.mv_recode_from.trim().to_owned(), json!(state.mv_recode_to.trim()));
                intents.push_engine("table.recodeLevels", json!({ "sheet_id": ds.id.as_str(), "col": active_col, "mappings": map }));
            }
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Manage All Variables…",
            tooltip: "Open full Variable Manager & Formula Bar",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.mv.manage_vars",
            command_id: Some("ui.table.openFormulaBar"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": active_col }));
    }
}
