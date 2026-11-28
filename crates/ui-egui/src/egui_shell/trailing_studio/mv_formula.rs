//! Multiple-Variables (MV) variable & formula studio surface.

use std::collections::BTreeSet;

use curvebook_doc::{DataSheet, LevelOrderKind, MultipleVariablesTable, VariableColumn, VariableRole, VariableSource};
use curvebook_transform::{DependencyTarget, extract_dependencies, parse_formula, validate_no_formula_cycle};
use egui::{Stroke, Vec2};
use serde_json::json;

use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, OKABE_ITO_SERIES_PALETTE, PendingUiIntents, widgets};
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::helpers::{SCROLL_BAR_RESERVE_PT, SECTION_BTN_H, StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
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
    FormulaFnEntry { name: "count", signature: "count(var)", insert: "count(", summary: "Active numeric cell count" },
    FormulaFnEntry { name: "counta", signature: "counta(var)", insert: "counta(", summary: "Active non-empty numeric or text count" },
    FormulaFnEntry { name: "sum", signature: "sum(var)", insert: "sum(", summary: "Column or row-wise sum" },
    FormulaFnEntry { name: "min", signature: "min(var)", insert: "min(", summary: "Column or row-wise minimum" },
    FormulaFnEntry { name: "max", signature: "max(var)", insert: "max(", summary: "Column or row-wise maximum" },
    FormulaFnEntry { name: "var", signature: "var(var)", insert: "var(", summary: "Sample variance" },
    FormulaFnEntry { name: "geo_mean", signature: "geo_mean(var)", insert: "geo_mean(", summary: "Column geometric mean (x > 0)" },
    FormulaFnEntry { name: "percentile", signature: "percentile(var, p)", insert: "percentile(", summary: "Column p-th percentile (0 to 100)" },
    FormulaFnEntry { name: "countaif", signature: "countaif(var, cond)", insert: "countaif(", summary: "Conditional non-empty cell count" },
    FormulaFnEntry { name: "minif", signature: "minif(var, cond)", insert: "minif(", summary: "Conditional column minimum" },
    FormulaFnEntry { name: "maxif", signature: "maxif(var, cond)", insert: "maxif(", summary: "Conditional column maximum" },
    FormulaFnEntry { name: "sdif", signature: "sdif(var, cond)", insert: "sdif(", summary: "Conditional sample standard deviation" },
    FormulaFnEntry { name: "semif", signature: "semif(var, cond)", insert: "semif(", summary: "Conditional standard error of the mean" },
    FormulaFnEntry { name: "log", signature: "log(x[, base])", insert: "log(", summary: "Base-10 or arbitrary-base logarithm" },
    FormulaFnEntry { name: "log2", signature: "log2(x)", insert: "log2(", summary: "Base-2 logarithm" },
    FormulaFnEntry { name: "exp10", signature: "exp10(x)", insert: "exp10(", summary: "Base-10 antilogarithm 10^x" },
    FormulaFnEntry { name: "pow", signature: "pow(x, y)", insert: "pow(", summary: "Raise x to the power y" },
    FormulaFnEntry { name: "mod", signature: "mod(x, y)", insert: "mod(", summary: "Remainder of x divided by y" },
    FormulaFnEntry { name: "fact", signature: "fact(n)", insert: "fact(", summary: "Integer factorial n! (0 <= n <= 170)" },
    FormulaFnEntry { name: "perc", signature: "perc(x)", insert: "perc(", summary: "Convert percentage x to fraction x / 100" },
    FormulaFnEntry { name: "pi", signature: "pi([n])", insert: "pi(", summary: "Circle constant pi (or n * pi)" },
    FormulaFnEntry { name: "null", signature: "null", insert: "null", summary: "Missing or empty cell value literal" },
    FormulaFnEntry { name: "rad", signature: "rad(deg)", insert: "rad(", summary: "Convert degrees to radians" },
    FormulaFnEntry { name: "deg", signature: "deg(rad)", insert: "deg(", summary: "Convert radians to degrees" },
    FormulaFnEntry { name: "asinh", signature: "asinh(x)", insert: "asinh(", summary: "Inverse hyperbolic sine" },
    FormulaFnEntry { name: "acosh", signature: "acosh(x)", insert: "acosh(", summary: "Inverse hyperbolic cosine (x >= 1)" },
    FormulaFnEntry { name: "atanh", signature: "atanh(x)", insert: "atanh(", summary: "Inverse hyperbolic tangent (|x| < 1)" },
    FormulaFnEntry { name: "floor", signature: "floor(x)", insert: "floor(", summary: "Largest integer <= x" },
    FormulaFnEntry { name: "ceil", signature: "ceil(x)", insert: "ceil(", summary: "Smallest integer >= x" },
    FormulaFnEntry { name: "round", signature: "round(x[, digits])", insert: "round(", summary: "Round x to nearest integer or digits" },
    FormulaFnEntry { name: "trunc", signature: "trunc(x)", insert: "trunc(", summary: "Truncate fractional part toward zero" },
    FormulaFnEntry { name: "clamp", signature: "clamp(x, lo, hi)", insert: "clamp(", summary: "Constrain x to interval [lo, hi]" },
    FormulaFnEntry { name: "sign", signature: "sign(x)", insert: "sign(", summary: "Sign of x (-1, 0, or 1)" },
    FormulaFnEntry { name: "gauss", signature: "gauss(mean, sd[, seed])", insert: "gauss(", summary: "Seeded Gaussian random draw" },
    FormulaFnEntry { name: "rnd", signature: "rnd(lo, hi[, seed])", insert: "rnd(", summary: "Seeded uniform random draw in [lo, hi)" },
    FormulaFnEntry { name: "concatenate", signature: "concatenate(a, b)", insert: "concatenate(", summary: "Join text and numbers into a string" },
    FormulaFnEntry { name: "gamma", signature: "gamma(x)", insert: "gamma(", summary: "Euler gamma function Gamma(x)" },
    FormulaFnEntry { name: "gammaln", signature: "gammaln(x)", insert: "gammaln(", summary: "Natural logarithm of |Gamma(x)|" },
    FormulaFnEntry { name: "igamma", signature: "igamma(a, x)", insert: "igamma(", summary: "Regularized lower incomplete gamma P(a, x)" },
    FormulaFnEntry { name: "igammac", signature: "igammac(a, x)", insert: "igammac(", summary: "Regularized upper incomplete gamma Q(a, x)" },
    FormulaFnEntry { name: "psi", signature: "psi(x)", insert: "psi(", summary: "Digamma function d/dx ln Gamma(x)" },
    FormulaFnEntry { name: "beta", signature: "beta(a, b)", insert: "beta(", summary: "Euler beta function B(a, b)" },
    FormulaFnEntry { name: "ibeta", signature: "ibeta(a, b, x)", insert: "ibeta(", summary: "Regularized incomplete beta I_x(a, b)" },
    FormulaFnEntry { name: "besselj", signature: "besselj(n, x)", insert: "besselj(", summary: "Bessel function of the first kind J_n(x)" },
    FormulaFnEntry { name: "bessely", signature: "bessely(n, x)", insert: "bessely(", summary: "Bessel function of the second kind Y_n(x)" },
    FormulaFnEntry { name: "besseli", signature: "besseli(n, x)", insert: "besseli(", summary: "Modified Bessel function of the first kind I_n(x)" },
    FormulaFnEntry { name: "besselk", signature: "besselk(n, x)", insert: "besselk(", summary: "Modified Bessel function of the second kind K_n(x)" },
    FormulaFnEntry {
        name: "hypgeometricm",
        signature: "hypgeometricm(a, b, z)",
        insert: "hypgeometricm(",
        summary: "Confluent hypergeometric function M(a, b, z)",
    },
    FormulaFnEntry {
        name: "hypgeometricu",
        signature: "hypgeometricu(a, b, z)",
        insert: "hypgeometricu(",
        summary: "Confluent hypergeometric function U(a, b, z)",
    },
    FormulaFnEntry {
        name: "hypgeometricf",
        signature: "hypgeometricf(a, b, c, x)",
        insert: "hypgeometricf(",
        summary: "Gauss hypergeometric function 2F1(a, b; c; x)",
    },
];

/// Return the canonical formula signature shown by the trailing Formula Studio (`#14`).
#[must_use]
pub fn studio_formula_signature(name: &str) -> Option<&'static str> {
    STUDIO_FORMULA_FUNCS.iter().find(|entry| entry.name == name).map(|entry| entry.signature)
}

fn unique_new_calc_title(mv: &MultipleVariablesTable, draft: &str) -> String {
    let base = draft.trim();
    let candidate = if base.is_empty() { format!("Calc {}", mv.variables.len() + 1) } else { base.to_owned() };
    if mv.find_variable(&candidate).is_none() {
        return candidate;
    }
    for n in 2..10_000 {
        let numbered = format!("{candidate} ({n})");
        if mv.find_variable(&numbered).is_none() {
            return numbered;
        }
    }
    format!("Calc {}", mv.variables.len() + 1)
}

fn role_label(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "Continuous",
        VariableRole::Categorical => "Categorical",
        VariableRole::Label => "Label",
        VariableRole::Calculated => "Calculated",
    }
}

fn render_mv_variable_selector(
    ui: &mut egui::Ui,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    active_col: usize,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
) {
    let tokens = ctx_in.tokens;
    let editable = ctx_in.enabled && !ds.frozen;
    let frozen_reason = ds.frozen.then_some("data sheet is frozen (read-only)");
    let col_count = mv.variables.len();
    let creating_new_col = active_col == col_count;
    widgets::subheader(ui, tr(tokens.locale, "mv.panel.var_name"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
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
                    tooltip: "",
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
                state.mv_synced_sheet = Some(ds.id.clone());
                state.mv_synced_col = Some(idx);
                state.mv_synced_source = Some(var.source.clone());
                state.mv_title_draft = var.title.clone();
                state.mv_formula_draft = match &var.source {
                    VariableSource::Calculated { formula } => formula.clone(),
                    VariableSource::Entered => String::new(),
                };
            }
        }
        let new_col_clicked = studio_chip(
            ui,
            tokens,
            meta,
            &StudioChipSpec {
                label: "+ New",
                tooltip: "Create a new calculated variable column without overwriting existing data",
                max_w: 68.0,
                selected: creating_new_col,
                enabled: editable,
                border_override: None,
                widget_id: "inspector.mv.new_calc_col",
                role: "button",
                command_id: None,
                disabled_reason: frozen_reason,
            },
        );
        if new_col_clicked {
            state.mv_selected_col = col_count;
            state.mv_synced_sheet = Some(ds.id.clone());
            state.mv_synced_col = Some(col_count);
            state.mv_synced_source = None;
            state.mv_title_draft = unique_new_calc_title(mv, "");
            state.mv_formula_draft.clear();
            state.mv_sub_tab = 0;
        }
    });
}

fn sync_mv_formula_drafts(ds: &DataSheet, mv: &MultipleVariablesTable, active_col: usize, state: &mut TrailingStudioEguiState) {
    let col_count = mv.variables.len();
    let current_source = mv.variables.get(active_col).map(|v| &v.source);
    let sheet_or_col_changed = state.mv_synced_sheet.as_ref() != Some(&ds.id) || state.mv_synced_col != Some(active_col);
    let source_changed = match (state.mv_synced_source.as_ref(), current_source) {
        (Some(prev), Some(curr)) => prev != curr,
        (None, Some(VariableSource::Calculated { .. })) | (Some(_), None) => true,
        (None, Some(VariableSource::Entered)) | (None, None) => false,
    };
    if sheet_or_col_changed || source_changed {
        state.mv_synced_sheet = Some(ds.id.clone());
        state.mv_synced_col = Some(active_col);
        state.mv_synced_source = current_source.cloned();
        if let Some(var) = mv.variables.get(active_col) {
            if sheet_or_col_changed || matches!(var.source, VariableSource::Calculated { .. }) {
                state.mv_title_draft = var.title.clone();
            }
            state.mv_formula_draft = match &var.source {
                VariableSource::Calculated { formula } => formula.clone(),
                VariableSource::Entered => String::new(),
            };
        } else {
            state.mv_title_draft = format!("Calc {}", col_count + 1);
            state.mv_formula_draft.clear();
        }
    } else if state.mv_synced_source.is_none() && current_source.is_some() {
        state.mv_synced_source = current_source.cloned();
    }
}

fn render_mv_function_reference(
    ui: &mut egui::Ui,
    ds: &DataSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let editable = ctx_in.enabled && !ds.frozen;
    let frozen_reason = ds.frozen.then_some("data sheet is frozen (read-only)");
    ui.add_space(tokens.metrics.space_xs);
    widgets::subheader(ui, tr(locale, "mv.panel.func_ref"));
    let filter_hint = tr(locale, "mv.panel.search_funcs");
    let mut fn_filter_edit = egui::TextEdit::singleline(&mut state.mv_fn_filter).desired_width(ui.available_width()).hint_text(filter_hint);
    if !ctx_in.enabled {
        fn_filter_edit = fn_filter_edit.interactive(false);
    }
    let fn_filter_resp = ui.add(fn_filter_edit);
    let fn_filter_ak_label = filter_hint.to_owned();
    let fn_filter_enabled = ctx_in.enabled;
    fn_filter_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, fn_filter_enabled, fn_filter_ak_label.clone()));
    if fn_filter_resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
        meta.record_response(
            ui,
            &fn_filter_resp,
            FrameInspectEntryInput {
                widget_id: "inspector.mv.fn_filter",
                role: "textbox",
                label: filter_hint,
                command_id: None,
                value: Some(json!(state.mv_fn_filter)),
                enabled: ctx_in.enabled,
                disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    let filter_lower = state.mv_fn_filter.trim().to_ascii_lowercase();
    let fn_scroll_h = 4.0 * (SECTION_BTN_H + tokens.metrics.space_xs);
    let fn_inner_max_w = (ui.available_width() - SCROLL_BAR_RESERVE_PT).max(120.0);
    let chip_max_w = fn_inner_max_w.min(132.0);
    ui.add_space(tokens.metrics.space_xs);
    ui.scope(|ui| {
        ui.spacing_mut().scroll.floating = false;
        egui::ScrollArea::vertical().id_salt("egui_shell.mv_fn_ref.scroll").max_height(fn_scroll_h).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_max_width(fn_inner_max_w);
            let mut matched_any = false;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
                for (idx, func) in STUDIO_FORMULA_FUNCS.iter().enumerate() {
                    if !filter_lower.is_empty()
                        && !func.name.to_ascii_lowercase().contains(&filter_lower)
                        && !func.signature.to_ascii_lowercase().contains(&filter_lower)
                        && !func.summary.to_ascii_lowercase().contains(&filter_lower)
                    {
                        continue;
                    }
                    matched_any = true;
                    let wid = format!("inspector.mv.fn.{idx}");
                    let r = studio_chip(
                        ui,
                        tokens,
                        meta,
                        &StudioChipSpec {
                            label: func.name,
                            tooltip: func.summary,
                            max_w: chip_max_w,
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
            if !matched_any {
                widgets::dim_label(ui, tr(locale, "mv.panel.no_matching_funcs"));
            }
        });
    });

    if let Some(spec) = STUDIO_FORMULA_FUNCS.iter().find(|f| f.name == state.mv_selected_fn) {
        egui::Frame::NONE.fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.divider)).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(
            ui,
            |ui| {
                ui.label(egui::RichText::new(spec.signature).size(tokens.metrics.font_ui_small_pt).strong().color(tokens.text_strong));
                ui.label(egui::RichText::new(spec.summary).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
            },
        );
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
    // Allow `state.mv_selected_col == col_count` to represent creating a brand-new calculated column (`AUD-09-18`).
    if state.mv_selected_col > col_count {
        state.mv_selected_col = 0;
    }
    let active_col = state.mv_selected_col;

    // Sync formula & title drafts cleanly whenever selected sheet, column, or column source changes
    // (including unlinking a Calculated column into Entered values or undoing back to Calculated).
    sync_mv_formula_drafts(ds, mv, active_col, state);

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
    render_mv_variable_selector(ui, ds, mv, active_col, state, ctx_in, meta);
    let active_col = state.mv_selected_col.min(col_count);
    let creating_new_col = active_col == col_count;

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
        let syntax_status = if state.mv_formula_draft.trim().is_empty() {
            Ok("Enter an expression referencing [Columns]".to_owned())
        } else {
            parse_formula(&state.mv_formula_draft)
                .map(|_| format!("Valid formula • {} operand highlight(s)", state.mv_operand_highlights.len()))
                .map_err(|e| e.to_string())
        };
        let formula_status = if state.mv_formula_draft.trim().is_empty() || creating_new_col {
            syntax_status.clone()
        } else {
            validate_no_formula_cycle(mv, active_col, &state.mv_formula_draft)
                .map(|()| format!("Valid formula • {} operand highlight(s)", state.mv_operand_highlights.len()))
                .map_err(|e| e.to_string())
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
                ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
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

        let is_entered_col = mv.variables.get(active_col).is_some_and(|v| matches!(v.source, VariableSource::Entered));
        let is_calculated_col =
            !creating_new_col && mv.variables.get(active_col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }));
        let can_apply = editable && !state.mv_formula_draft.trim().is_empty() && formula_status.is_ok();
        let apply_reason = frozen_reason.or_else(|| (!can_apply).then_some("enter a valid non-circular formula"));
        let apply_label = if creating_new_col { "Add Calculated Column" } else { "Apply Formula" };
        ui.add_space(4.0);
        if studio_full_button(
            ui,
            tokens,
            meta,
            &StudioButtonSpec {
                label: apply_label,
                tooltip: if creating_new_col {
                    "Append a new calculated variable column (table.setCalculatedVariable)"
                } else {
                    "Save calculated variable formula onto the selected column (table.setCalculatedVariable)"
                },
                selected: false,
                enabled: can_apply,
                primary: true,
                widget_id: "inspector.mv.apply_formula",
                command_id: Some("table.setCalculatedVariable"),
                disabled_reason: apply_reason,
            },
        ) {
            let title = if creating_new_col {
                unique_new_calc_title(mv, &state.mv_title_draft)
            } else if state.mv_title_draft.trim().is_empty() {
                format!("Calc {}", col_count + 1)
            } else {
                state.mv_title_draft.trim().to_owned()
            };
            let mut params = json!({
                "sheet_id": ds.id.as_str(),
                "title": title,
                "formula": state.mv_formula_draft.trim(),
            });
            if !creating_new_col
                && active_col < col_count
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("col"), json!(active_col));
            }
            intents.push_engine("table.setCalculatedVariable", params);
        }

        // Offer one-click unlinking on Calculated columns to freeze computed cells into static Entered data.
        if is_calculated_col {
            ui.add_space(tokens.metrics.space_xs);
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: tr(locale, "mv.panel.unlink_formula"),
                    tooltip: tr(locale, "mv.panel.unlink_formula_tooltip"),
                    selected: false,
                    enabled: editable,
                    primary: false,
                    widget_id: "inspector.mv.unlink_formula",
                    command_id: Some("table.unlinkCalculatedVariable"),
                    disabled_reason: frozen_reason,
                },
            ) {
                intents.push_engine(
                    "table.unlinkCalculatedVariable",
                    json!({
                        "sheet_id": ds.id.as_str(),
                        "col": active_col,
                    }),
                );
            }
        }

        // Always provide a safe create-new-column path when an Entered column is focused so users
        // never overwrite entered data in-place or hit self-cycle errors (`AUD-09-18`).
        if is_entered_col {
            let can_add_new = editable && !state.mv_formula_draft.trim().is_empty() && syntax_status.is_ok();
            let add_reason = frozen_reason.or_else(|| (!can_add_new).then_some("enter a valid formula expression"));
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: "Add as New Calculated Column",
                    tooltip: "Append a new calculated column without overwriting the focused entered column",
                    selected: false,
                    enabled: can_add_new,
                    primary: false,
                    widget_id: "inspector.mv.add_new_formula",
                    command_id: Some("table.setCalculatedVariable"),
                    disabled_reason: add_reason,
                },
            ) {
                let title = unique_new_calc_title(mv, &state.mv_title_draft);
                intents.push_engine(
                    "table.setCalculatedVariable",
                    json!({
                        "sheet_id": ds.id.as_str(),
                        "title": title,
                        "formula": state.mv_formula_draft.trim(),
                    }),
                );
            }
        }

        ui.add_space(6.0);
        widgets::subheader(ui, tr(locale, "mv.panel.column_vars"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
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

        render_mv_function_reference(ui, ds, state, ctx_in, meta);
    } else {
        render_mv_role_and_levels_tab(ui, ds, mv, state, ctx_in, meta, intents);
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: "Manage All Variables",
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

fn render_mv_role_and_levels_tab(
    ui: &mut egui::Ui,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let active_col = state.mv_selected_col;
    let Some(var) = mv.variables.get(active_col) else {
        return;
    };
    let tokens = ctx_in.tokens;
    let locale = tokens.locale;
    let editable = ctx_in.enabled && !ds.frozen;
    let frozen_reason = ds.frozen.then_some("data sheet is frozen (read-only)");
    let cur_role = var.role;
    ui.label(egui::RichText::new(format!("{} ({})", var.title, role_label(cur_role))).size(12.0).strong().color(tokens.text_strong));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
        for (slug, lbl, r_kind) in [
            ("continuous", "Continuous", VariableRole::Continuous),
            ("categorical", "Categorical", VariableRole::Categorical),
            ("label", "Label", VariableRole::Label),
        ] {
            let wid = format!("inspector.mv.role.{slug}");
            let role_enabled = editable && cur_role != VariableRole::Calculated;
            let role_reason = frozen_reason.or_else(|| (cur_role == VariableRole::Calculated).then_some("calculated columns lock Calculated role"));
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
            ui.spacing_mut().item_spacing = Vec2::new(tokens.metrics.space_xs, tokens.metrics.space_xs);
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
        let mut from_edit = egui::TextEdit::singleline(&mut state.mv_recode_from).desired_width(ui.available_width()).hint_text("Recode from level");
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
