//! Multiple-variables role badges, `Variables...` recoding/ordering drawer, and right-side
//! calculated-variable editor (`R10.8`).

use curvebook_doc::{DataSheet, LevelOrderKind, LevelOrderSpec, MultipleVariablesTable, SheetId, TableArchetype, VariableRole, VariableSource};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{variable_role_badge, variable_role_color};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldEditState, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab,
    emit_text_field, emit_toggle, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::{labeled_meta, selectable_meta};
use super::analyze_picker::scaled_u32;

const DOMAIN_MV_FORMULA: &str = "mv_formula";
const WID_FORMULA_INPUT: &str = "formula_bar.input";
const WID_NAME_INPUT: &str = "mv_formula.name.input";
const WID_FUNC_SEARCH: &str = "mv_formula.func.search";
const WID_RECODE_FROM: &str = "mv_formula.vars.recode_from.input";
const WID_RECODE_TO: &str = "mv_formula.vars.recode_to.input";

/// Transient view state owned by the multiple-variables / formula bar shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MvFormulaViewState {
    pub function_search: String,
    pub show_formula_definitions: bool,
    #[serde(default)]
    pub right_panel_open: bool,
    #[serde(default)]
    pub variables_drawer_open: bool,
    #[serde(default)]
    pub open_role_dropdown_col: Option<usize>,
    #[serde(default)]
    pub badge_scroll_col: usize,
    #[serde(default)]
    pub selected_function_name: Option<String>,
    #[serde(default)]
    pub function_category: String,
    #[serde(default)]
    pub validation_feedback: Option<String>,
    #[serde(default)]
    pub status_notice: Option<String>,
    #[serde(default)]
    pub recode_from: String,
    #[serde(default)]
    pub recode_to: String,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Copy)]
struct FormulaFnSpec {
    name: &'static str,
    signature: &'static str,
    insert_text: &'static str,
    category: &'static str,
    params_hint: &'static str,
    summary: &'static str,
}

const FORMULA_FUNCTIONS: &[FormulaFnSpec] = &[
    FormulaFnSpec {
        name: "abs",
        signature: "abs(x)",
        insert_text: "abs(",
        category: "math",
        params_hint: "x: numeric value or column",
        summary: "Absolute magnitude |x|",
    },
    FormulaFnSpec {
        name: "sqrt",
        signature: "sqrt(x)",
        insert_text: "sqrt(",
        category: "math",
        params_hint: "x: number >= 0",
        summary: "Square root of x",
    },
    FormulaFnSpec {
        name: "cbrt",
        signature: "cbrt(x)",
        insert_text: "cbrt(",
        category: "math",
        params_hint: "x: numeric value",
        summary: "Cube root of x",
    },
    FormulaFnSpec { name: "sqr", signature: "sqr(x)", insert_text: "sqr(", category: "math", params_hint: "x: numeric value", summary: "Square x^2" },
    FormulaFnSpec {
        name: "ln",
        signature: "ln(x)",
        insert_text: "ln(",
        category: "math",
        params_hint: "x: positive number (> 0)",
        summary: "Natural logarithm (base e)",
    },
    FormulaFnSpec {
        name: "log10",
        signature: "log10(x)",
        insert_text: "log10(",
        category: "math",
        params_hint: "x: positive number (> 0)",
        summary: "Base-10 logarithm",
    },
    FormulaFnSpec {
        name: "log2",
        signature: "log2(x)",
        insert_text: "log2(",
        category: "math",
        params_hint: "x: positive number (> 0)",
        summary: "Base-2 logarithm",
    },
    FormulaFnSpec {
        name: "log",
        signature: "log(x, [base])",
        insert_text: "log(",
        category: "math",
        params_hint: "x > 0, optional base > 0 (defaults to e)",
        summary: "Logarithm in natural or custom base",
    },
    FormulaFnSpec {
        name: "exp",
        signature: "exp(x)",
        insert_text: "exp(",
        category: "math",
        params_hint: "x: exponent",
        summary: "Natural exponential e^x",
    },
    FormulaFnSpec {
        name: "exp10",
        signature: "exp10(x)",
        insert_text: "exp10(",
        category: "math",
        params_hint: "x: exponent",
        summary: "Power of ten 10^x",
    },
    FormulaFnSpec {
        name: "pow",
        signature: "pow(x, y)",
        insert_text: "pow(",
        category: "math",
        params_hint: "x: base, y: exponent",
        summary: "Raise x to power y",
    },
    FormulaFnSpec {
        name: "round",
        signature: "round(x, [digits])",
        insert_text: "round(",
        category: "math",
        params_hint: "x: number, optional digits: integer",
        summary: "Round to nearest integer or decimal places",
    },
    FormulaFnSpec {
        name: "floor",
        signature: "floor(x)",
        insert_text: "floor(",
        category: "math",
        params_hint: "x: number",
        summary: "Greatest integer <= x",
    },
    FormulaFnSpec {
        name: "ceil",
        signature: "ceil(x)",
        insert_text: "ceil(",
        category: "math",
        params_hint: "x: number",
        summary: "Least integer >= x",
    },
    FormulaFnSpec {
        name: "clamp",
        signature: "clamp(x, lo, hi)",
        insert_text: "clamp(",
        category: "math",
        params_hint: "x: value, lo: lower bound, hi: upper bound",
        summary: "Clamp x within [lo, hi]",
    },
    FormulaFnSpec {
        name: "logit",
        signature: "logit(p)",
        insert_text: "logit(",
        category: "math",
        params_hint: "p: proportion in (0, 1)",
        summary: "Logit transform ln(p / (1 - p))",
    },
    FormulaFnSpec {
        name: "probit",
        signature: "probit(p)",
        insert_text: "probit(",
        category: "math",
        params_hint: "p: proportion in (0, 1)",
        summary: "Probit transform 5 + inv_norm(p)",
    },
    FormulaFnSpec {
        name: "norm_ppf",
        signature: "norm_ppf(p)",
        insert_text: "norm_ppf(",
        category: "math",
        params_hint: "p: probability in (0, 1)",
        summary: "Standard normal inverse CDF",
    },
    FormulaFnSpec {
        name: "arcsin_sqrt",
        signature: "arcsin_sqrt(p)",
        insert_text: "arcsin_sqrt(",
        category: "math",
        params_hint: "p: proportion in [0, 1]",
        summary: "Angular arcsine square-root transform",
    },
    FormulaFnSpec {
        name: "if",
        signature: "if(cond, t, f)",
        insert_text: "if(",
        category: "logic",
        params_hint: "cond: predicate, t: true branch, f: false branch",
        summary: "Row-wise conditional expression",
    },
    FormulaFnSpec {
        name: "and",
        signature: "and(a, b, ...)",
        insert_text: "and(",
        category: "logic",
        params_hint: "a, b, ...: boolean expressions",
        summary: "Logical conjunction across arguments",
    },
    FormulaFnSpec {
        name: "or",
        signature: "or(a, b, ...)",
        insert_text: "or(",
        category: "logic",
        params_hint: "a, b, ...: boolean expressions",
        summary: "Logical disjunction across arguments",
    },
    FormulaFnSpec {
        name: "not",
        signature: "not(x)",
        insert_text: "not(",
        category: "logic",
        params_hint: "x: boolean expression",
        summary: "Logical negation",
    },
    FormulaFnSpec {
        name: "is_defined",
        signature: "is_defined(x)",
        insert_text: "is_defined(",
        category: "logic",
        params_hint: "x: cell or column reference",
        summary: "1 when value is non-missing, 0 otherwise",
    },
    FormulaFnSpec {
        name: "isnan",
        signature: "isnan(x)",
        insert_text: "isnan(",
        category: "logic",
        params_hint: "x: cell or column reference",
        summary: "1 when value is missing or non-finite",
    },
    FormulaFnSpec {
        name: "coalesce",
        signature: "coalesce(a, b, ...)",
        insert_text: "coalesce(",
        category: "logic",
        params_hint: "a, b, ...: fallback expressions",
        summary: "First non-missing value per row",
    },
    FormulaFnSpec {
        name: "count",
        signature: "count(var)",
        insert_text: "count(",
        category: "stats",
        params_hint: "var: variable column",
        summary: "Active non-missing observation count",
    },
    FormulaFnSpec {
        name: "sum",
        signature: "sum(var)",
        insert_text: "sum(",
        category: "stats",
        params_hint: "var: numeric column or row expressions",
        summary: "Sum of finite values",
    },
    FormulaFnSpec {
        name: "mean",
        signature: "mean(var)",
        insert_text: "mean(",
        category: "stats",
        params_hint: "var: numeric column or row expressions",
        summary: "Arithmetic mean",
    },
    FormulaFnSpec {
        name: "sd",
        signature: "sd(var)",
        insert_text: "sd(",
        category: "stats",
        params_hint: "var: numeric column",
        summary: "Sample standard deviation (n - 1)",
    },
    FormulaFnSpec {
        name: "sem",
        signature: "sem(var)",
        insert_text: "sem(",
        category: "stats",
        params_hint: "var: numeric column",
        summary: "Standard error of the mean",
    },
    FormulaFnSpec {
        name: "median",
        signature: "median(var)",
        insert_text: "median(",
        category: "stats",
        params_hint: "var: numeric column",
        summary: "Sample median (50th percentile)",
    },
    FormulaFnSpec {
        name: "iqr",
        signature: "iqr(var)",
        insert_text: "iqr(",
        category: "stats",
        params_hint: "var: numeric column",
        summary: "Interquartile range (Q3 - Q1)",
    },
    FormulaFnSpec {
        name: "geo_mean",
        signature: "geo_mean(var)",
        insert_text: "geo_mean(",
        category: "stats",
        params_hint: "var: strictly positive numeric column",
        summary: "Geometric mean exp(mean(ln(var)))",
    },
    FormulaFnSpec {
        name: "percentile",
        signature: "percentile(var, p)",
        insert_text: "percentile(",
        category: "stats",
        params_hint: "var: numeric column, p: percentage 0..100",
        summary: "Linear-interpolated p-th percentile",
    },
    FormulaFnSpec {
        name: "countif",
        signature: "countif(cond)",
        insert_text: "countif(",
        category: "stats",
        params_hint: "cond: boolean condition expression",
        summary: "Count rows where condition is true",
    },
    FormulaFnSpec {
        name: "sumif",
        signature: "sumif(cond, var)",
        insert_text: "sumif(",
        category: "stats",
        params_hint: "cond: predicate, var: numeric column",
        summary: "Conditional sum over matching rows",
    },
    FormulaFnSpec {
        name: "meanif",
        signature: "meanif(cond, var)",
        insert_text: "meanif(",
        category: "stats",
        params_hint: "cond: predicate, var: numeric column",
        summary: "Conditional mean over matching rows",
    },
    FormulaFnSpec {
        name: "zscore",
        signature: "zscore(var)",
        insert_text: "zscore(",
        category: "window",
        params_hint: "var: numeric column",
        summary: "Standardized (x - mean) / sd",
    },
    FormulaFnSpec {
        name: "row_number",
        signature: "row_number()",
        insert_text: "row_number()",
        category: "window",
        params_hint: "(no parameters)",
        summary: "1-based row index",
    },
    FormulaFnSpec {
        name: "rank",
        signature: "rank(var)",
        insert_text: "rank(",
        category: "window",
        params_hint: "var: numeric column",
        summary: "Average-tie competition rank",
    },
    FormulaFnSpec {
        name: "dense_rank",
        signature: "dense_rank(var)",
        insert_text: "dense_rank(",
        category: "window",
        params_hint: "var: numeric column",
        summary: "Gap-free dense rank",
    },
    FormulaFnSpec {
        name: "lag",
        signature: "lag(var, [k])",
        insert_text: "lag(",
        category: "window",
        params_hint: "var: column, optional k: non-negative integer offset",
        summary: "Value k rows earlier (default 1)",
    },
    FormulaFnSpec {
        name: "lead",
        signature: "lead(var, [k])",
        insert_text: "lead(",
        category: "window",
        params_hint: "var: column, optional k: non-negative integer offset",
        summary: "Value k rows ahead (default 1)",
    },
    FormulaFnSpec {
        name: "diff",
        signature: "diff(var, [k])",
        insert_text: "diff(",
        category: "window",
        params_hint: "var: column, optional k: lag offset",
        summary: "Row difference x[i] - x[i - k]",
    },
    FormulaFnSpec {
        name: "cumsum",
        signature: "cumsum(var)",
        insert_text: "cumsum(",
        category: "window",
        params_hint: "var: numeric column",
        summary: "Cumulative running sum",
    },
    FormulaFnSpec {
        name: "cummean",
        signature: "cummean(var)",
        insert_text: "cummean(",
        category: "window",
        params_hint: "var: numeric column",
        summary: "Cumulative running arithmetic mean",
    },
    FormulaFnSpec {
        name: "moving_avg",
        signature: "moving_avg(var, w)",
        insert_text: "moving_avg(",
        category: "window",
        params_hint: "var: numeric column, w: integer width >= 1",
        summary: "Trailing moving average over w rows",
    },
];

fn mv_tr(locale: &str, key: &str, fallback: &'static str) -> &'static str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
}

fn custom_action(payload: impl Into<String>) -> WidgetAction {
    WidgetAction::Custom { domain: String::from(DOMAIN_MV_FORMULA), payload: payload.into() }
}

fn active_mv_sheet(app: &UiApp) -> Option<(SheetId, DataSheet)> {
    let doc = app.document();
    let sid = app.grid.formula_bar.target_sheet_id.as_ref().or(doc.active_sheet_id.as_ref())?;
    let ds = doc.data_sheet(sid).filter(|s| matches!(s.archetype, TableArchetype::MultipleVariables(_)))?;
    Some((sid.clone(), ds.clone()))
}

fn active_any_sheet_id(app: &UiApp) -> Option<SheetId> {
    let doc = app.document();
    app.grid.formula_bar.target_sheet_id.clone().or_else(|| doc.active_sheet_id.clone()).filter(|id| doc.data_sheet(id).is_some())
}

fn format_column_ref(title: &str) -> String {
    let trimmed = title.trim();
    let simple = !trimmed.is_empty()
        && trimmed
            .chars()
            .enumerate()
            .all(|(i, c)| (i == 0 && (c.is_ascii_alphabetic() || c == '_')) || (i > 0 && (c.is_ascii_alphanumeric() || c == '_')));
    if simple { trimmed.to_owned() } else { format!("[{trimmed}]") }
}

fn sync_text_buffer(app: &mut UiApp, wid: &str, text: &str) {
    app.interaction.text_field_buffers.insert(wid.to_owned(), TextFieldEditState::new(text, false));
}

fn clear_formula_text_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove(WID_FORMULA_INPUT);
    app.interaction.text_field_buffers.remove(WID_NAME_INPUT);
}

fn insert_snippet_into_formula(app: &mut UiApp, snippet: &str) {
    let doc = app.document().clone();
    if !app.grid.formula_bar.active
        && let Some((_, ds)) = active_mv_sheet(app)
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        let fallback_col = app.grid.tool.active.map(|c| c.col).filter(|c| *c < mv.variables.len()).unwrap_or(0);
        let name_buf = app.interaction.text_field_buffers.get(WID_NAME_INPUT).map(|b| b.text.clone()).unwrap_or_default();
        if !name_buf.trim().is_empty() && mv.find_variable(&name_buf).is_none() {
            app.grid.formula_bar.open_for_new_column(&ds, name_buf, Some(&doc));
        } else if fallback_col < mv.variables.len() {
            app.grid.formula_bar.open_for_column(&ds, fallback_col, Some(&doc));
        }
    }

    let base_text = app.interaction.text_field_buffers.get(WID_FORMULA_INPUT).map_or_else(|| app.grid.formula_bar.input.clone(), |b| b.text.clone());

    let mut edit = app.interaction.text_field_buffers.get(WID_FORMULA_INPUT).cloned().unwrap_or_else(|| TextFieldEditState::new(&base_text, false));

    if edit.text.trim().is_empty() {
        edit = TextFieldEditState::new("= ", false);
    }
    edit.insert_text(snippet);
    let updated_text = edit.text.clone();
    let cursor_bytes = updated_text.char_indices().nth(edit.cursor_char).map_or(updated_text.len(), |(b, _)| b);
    app.interaction.text_field_buffers.insert(String::from(WID_FORMULA_INPUT), edit);
    app.grid.formula_bar.active = true;
    app.grid.formula_bar.set_input(updated_text, Some(cursor_bytes), &doc);
    app.focus_widget_id = Some(String::from(WID_FORMULA_INPUT));
}

fn sync_buffers_into_formula_bar(app: &mut UiApp) {
    let doc = app.document().clone();
    if let Some(name_buf) = app.interaction.text_field_buffers.get(WID_NAME_INPUT).map(|b| b.text.clone()) {
        app.grid.formula_bar.target_title = name_buf;
    }
    if let Some(formula_buf) = app.interaction.text_field_buffers.get(WID_FORMULA_INPUT).map(|b| b.text.clone())
        && formula_buf != app.grid.formula_bar.input
    {
        app.grid.formula_bar.set_input(formula_buf, None, &doc);
    }
}

fn commit_formula_from_action(app: &mut UiApp, state: &mut MvFormulaViewState) -> Result<UiEventOutcome, String> {
    sync_buffers_into_formula_bar(app);
    let fb = &app.grid.formula_bar;
    if !fb.active {
        let msg = String::from("Formula bar is not active.");
        state.validation_feedback = Some(msg.clone());
        return Err(msg);
    }
    if let Some(err) = fb.parse_error.clone() {
        state.validation_feedback = Some(err.clone());
        return Err(err);
    }
    if !fb.is_valid {
        let msg = String::from("Calculated variable formula cannot be empty or invalid.");
        state.validation_feedback = Some(msg.clone());
        return Err(msg);
    }
    let Some(sheet_id) = active_any_sheet_id(app) else {
        return Err(String::from("No active data sheet for formula bar."));
    };
    let intent = app.grid.formula_bar.commit_intent(&sheet_id)?;
    let outcome = app.execute_command(&intent.command_id, intent.params).map_err(|err| err.to_string())?;
    app.grid.formula_bar.cancel();
    clear_formula_text_buffers(app);
    if matches!(app.focus_widget_id.as_deref(), Some(WID_FORMULA_INPUT | WID_NAME_INPUT)) {
        app.focus_widget_id = None;
    }
    state.validation_feedback = None;
    state.status_notice = Some(String::from("Calculated variable applied."));
    if state.variables_drawer_open
        && let Some(ds) = app.document().data_sheet(&sheet_id).cloned()
    {
        app.grid.variables_dialog.sync_from_sheet(&ds);
    }
    let payload = serde_json::to_value(outcome).map_err(|e| format!("serialize outcome: {e}"))?;
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("mv_formula.committed"),
        command_id: Some(String::from("table.setCalculatedVariable")),
        command_result: Some(payload),
    })
}

fn cancel_formula_from_action(app: &mut UiApp, state: &mut MvFormulaViewState) -> UiEventOutcome {
    app.grid.formula_bar.cancel();
    clear_formula_text_buffers(app);
    if matches!(app.focus_widget_id.as_deref(), Some(WID_FORMULA_INPUT | WID_NAME_INPUT)) {
        app.focus_widget_id = None;
    }
    state.right_panel_open = false;
    state.validation_feedback = None;
    UiEventOutcome { handled: true, action: String::from("mv_formula.cancelled"), ..Default::default() }
}

fn open_editor_for_column(app: &mut UiApp, state: &mut MvFormulaViewState, col: usize) -> Result<UiEventOutcome, String> {
    let doc = app.document().clone();
    let Some((sheet_id, ds)) = active_mv_sheet(app) else {
        return Err(String::from("Formula editor requires an active MultipleVariables sheet."));
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err(String::from("Formula editor requires an active MultipleVariables sheet."));
    };
    if col >= mv.variables.len() {
        return Err(format!("Variable column {col} out of range."));
    }
    app.grid.formula_bar.open_for_column(&ds, col, Some(&doc));
    sync_text_buffer(app, WID_FORMULA_INPUT, &app.grid.formula_bar.input.clone());
    sync_text_buffer(app, WID_NAME_INPUT, &app.grid.formula_bar.target_title.clone());
    state.right_panel_open = true;
    state.open_role_dropdown_col = None;
    state.validation_feedback = app.grid.formula_bar.parse_error.clone();
    app.focus_widget_id = Some(String::from(WID_FORMULA_INPUT));
    Ok(UiEventOutcome {
        handled: true,
        action: format!("mv_formula.open_col:{col}"),
        command_id: Some(String::from("ui.table.openFormulaBar")),
        command_result: Some(json!({ "sheet_id": sheet_id.as_str(), "column": col })),
    })
}

fn open_editor_for_new_column(app: &mut UiApp, state: &mut MvFormulaViewState) -> Result<UiEventOutcome, String> {
    let doc = app.document().clone();
    let Some((_, ds)) = active_mv_sheet(app) else {
        return Err(String::from("Calculated variables require an active MultipleVariables sheet."));
    };
    app.grid.formula_bar.open_for_new_column(&ds, "", Some(&doc));
    sync_text_buffer(app, WID_FORMULA_INPUT, &app.grid.formula_bar.input.clone());
    sync_text_buffer(app, WID_NAME_INPUT, "");
    state.right_panel_open = true;
    state.open_role_dropdown_col = None;
    state.validation_feedback = None;
    app.focus_widget_id = Some(String::from(WID_NAME_INPUT));
    Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.new_calc"), ..Default::default() })
}

fn execute_role_change(app: &mut UiApp, state: &mut MvFormulaViewState, col: usize, role: &str) -> Result<UiEventOutcome, String> {
    state.open_role_dropdown_col = None;
    if role == "calculated" {
        return open_editor_for_column(app, state, col);
    }
    let Some((sheet_id, _)) = active_mv_sheet(app) else {
        return Err(String::from("Role updates require an active MultipleVariables sheet."));
    };
    let outcome = app
        .execute_command("table.setVariableRole", json!({ "sheet_id": sheet_id.as_str(), "column": col, "role": role }))
        .map_err(|err| err.to_string())?;
    if let Some(ds) = app.document().data_sheet(&sheet_id).cloned()
        && app.grid.variables_dialog.selected_column == Some(col)
    {
        app.grid.variables_dialog.sync_from_sheet(&ds);
    }
    let payload = serde_json::to_value(outcome).map_err(|e| format!("serialize role outcome: {e}"))?;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("mv_formula.set_role:{col}:{role}"),
        command_id: Some(String::from("table.setVariableRole")),
        command_result: Some(payload),
    })
}

fn role_wire_name(role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => "continuous",
        VariableRole::Categorical => "categorical",
        VariableRole::Label => "label",
        VariableRole::Calculated => "calculated",
    }
}

fn handle_role_lock_toggle(app: &mut UiApp, state: &mut MvFormulaViewState, col: usize) -> Result<UiEventOutcome, String> {
    state.open_role_dropdown_col = None;
    let Some((_, ds)) = active_mv_sheet(app) else {
        return Err(String::from("Role lock requires an active MultipleVariables sheet."));
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Err(String::from("Role lock requires an active MultipleVariables sheet."));
    };
    let Some(var) = mv.variables.get(col) else {
        return Err(format!("Variable column {col} out of range."));
    };
    if matches!(var.source, VariableSource::Calculated { .. }) {
        state.status_notice = Some(String::from("Calculated variable role is formula-locked."));
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.role_lock_calculated"), ..Default::default() });
    }
    let next_role = if var.role_inferred { role_wire_name(var.role) } else { "auto" };
    execute_role_change(app, state, col, next_role)
}

fn ensure_variables_dialog_synced(app: &mut UiApp) -> Option<(SheetId, DataSheet)> {
    let (sheet_id, ds) = active_mv_sheet(app)?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return None;
    };
    let max_cols = mv.variables.len();
    if max_cols == 0 {
        return None;
    }
    let valid_col = app.grid.variables_dialog.selected_column.filter(|&c| c < max_cols);
    if !app.grid.variables_dialog.open || valid_col.is_none() {
        let initial_col = app.grid.tool.active.map(|c| c.col).filter(|&c| c < max_cols).unwrap_or(0);
        app.grid.variables_dialog.open_for_column(&ds, initial_col);
    } else {
        app.grid.variables_dialog.sync_from_sheet(&ds);
    }
    Some((sheet_id, ds))
}

fn apply_categorical_order(app: &mut UiApp, state: &mut MvFormulaViewState) -> Result<UiEventOutcome, String> {
    let Some((sheet_id, _)) = active_mv_sheet(app) else {
        return Err(String::from("Categorical ordering requires an active MultipleVariables sheet."));
    };
    let Some(intent) = app.grid.variables_dialog.build_order_command(&sheet_id) else {
        return Err(String::from("No variable column selected for level ordering."));
    };
    let outcome = app.execute_command(&intent.command_id, intent.params).map_err(|err| err.to_string())?;
    if let Some(ds) = app.document().data_sheet(&sheet_id).cloned() {
        app.grid.variables_dialog.sync_from_sheet(&ds);
    }
    state.status_notice = Some(String::from("Categorical level ordering updated."));
    let payload = serde_json::to_value(outcome).map_err(|e| format!("serialize order outcome: {e}"))?;
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("mv_formula.vars.order_applied"),
        command_id: Some(String::from("table.setCategoricalOrder")),
        command_result: Some(payload),
    })
}

fn stage_recode_inputs(app: &mut UiApp, state: &mut MvFormulaViewState) -> bool {
    let from = app.interaction.text_field_buffers.get(WID_RECODE_FROM).map_or_else(|| state.recode_from.clone(), |b| b.text.clone());
    let to = app.interaction.text_field_buffers.get(WID_RECODE_TO).map_or_else(|| state.recode_to.clone(), |b| b.text.clone());
    let from_trimmed = from.trim();
    let to_trimmed = to.trim();
    if from_trimmed.is_empty() || to_trimmed.is_empty() {
        return false;
    }
    app.grid.variables_dialog.add_recode_mapping(from_trimmed, to_trimmed);
    state.recode_from = from_trimmed.to_owned();
    state.recode_to = to_trimmed.to_owned();
    true
}

fn handle_variables_action(app: &mut UiApp, state: &mut MvFormulaViewState, sub: &str) -> Result<Option<UiEventOutcome>, String> {
    if sub == "toggle_vars_drawer" {
        state.open_role_dropdown_col = None;
        state.variables_drawer_open = !state.variables_drawer_open;
        app.grid.variables_dialog.open = state.variables_drawer_open;
        if state.variables_drawer_open {
            let _ = ensure_variables_dialog_synced(app);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_drawer_toggled"), ..Default::default() }));
    }
    if sub == "vars_close" {
        state.variables_drawer_open = false;
        app.grid.variables_dialog.open = false;
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_closed"), ..Default::default() }));
    }
    if let Some(raw) = sub.strip_prefix("vars_select_col:")
        && let Ok(col) = raw.parse::<usize>()
        && let Some((_, ds)) = active_mv_sheet(app)
    {
        app.grid.variables_dialog.open_for_column(&ds, col);
        app.interaction.text_field_buffers.remove(WID_RECODE_FROM);
        app.interaction.text_field_buffers.remove(WID_RECODE_TO);
        state.recode_from.clear();
        state.recode_to.clear();
        return Ok(Some(UiEventOutcome { handled: true, action: format!("mv_formula.vars_select_col:{col}"), ..Default::default() }));
    }
    if let Some(kind_str) = sub.strip_prefix("vars_order_kind:") {
        let kind = match kind_str {
            "entry" => LevelOrderKind::EntryOrder,
            "alpha" => LevelOrderKind::Alphabetical,
            "freq_desc" => LevelOrderKind::FrequencyDescending,
            "freq_asc" => LevelOrderKind::FrequencyAscending,
            _ => return Ok(None),
        };
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let spec = LevelOrderSpec { order: kind, reversed: app.grid.variables_dialog.level_order.reversed };
        app.grid.variables_dialog.set_level_order(spec, Some(&ds));
        return apply_categorical_order(app, state).map(Some);
    }
    if sub == "vars_order_reverse" {
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let mut spec = app.grid.variables_dialog.level_order.clone();
        spec.reversed = !spec.reversed;
        app.grid.variables_dialog.set_level_order(spec, Some(&ds));
        return apply_categorical_order(app, state).map(Some);
    }
    if sub == "vars_order_apply" {
        let _ = ensure_variables_dialog_synced(app);
        return apply_categorical_order(app, state).map(Some);
    }
    if let Some(raw) = sub.strip_prefix("vars_pick_level:")
        && let Ok(idx) = raw.parse::<usize>()
    {
        let _ = ensure_variables_dialog_synced(app);
        if let Some(level) = app.grid.variables_dialog.ordered_levels.get(idx).map(|l| l.level.clone()) {
            state.recode_from.clone_from(&level);
            sync_text_buffer(app, WID_RECODE_FROM, &level);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: format!("mv_formula.vars_pick_level:{idx}"), ..Default::default() }));
    }
    if let Some(raw) = sub.strip_prefix("vars_level_up:")
        && let Ok(idx) = raw.parse::<usize>()
    {
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let mut levels: Vec<String> = app.grid.variables_dialog.ordered_levels.iter().map(|l| l.level.clone()).collect();
        if idx > 0 && idx < levels.len() {
            levels.swap(idx, idx - 1);
            app.grid.variables_dialog.set_level_order(LevelOrderSpec { order: LevelOrderKind::Explicit(levels), reversed: false }, Some(&ds));
            return apply_categorical_order(app, state).map(Some);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_level_up_noop"), ..Default::default() }));
    }
    if let Some(raw) = sub.strip_prefix("vars_level_dn:")
        && let Ok(idx) = raw.parse::<usize>()
    {
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let mut levels: Vec<String> = app.grid.variables_dialog.ordered_levels.iter().map(|l| l.level.clone()).collect();
        if idx + 1 < levels.len() {
            levels.swap(idx, idx + 1);
            app.grid.variables_dialog.set_level_order(LevelOrderSpec { order: LevelOrderKind::Explicit(levels), reversed: false }, Some(&ds));
            return apply_categorical_order(app, state).map(Some);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_level_dn_noop"), ..Default::default() }));
    }
    if let Some(raw) = sub.strip_prefix("vars_level_ref:")
        && let Ok(idx) = raw.parse::<usize>()
    {
        let (sheet_id, _) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let col = app.grid.variables_dialog.selected_column.ok_or_else(|| String::from("No column selected."))?;
        let ref_level =
            app.grid.variables_dialog.ordered_levels.get(idx).map(|l| l.level.clone()).ok_or_else(|| String::from("Level index out of range."))?;
        let outcome = app
            .execute_command("table.setCategoricalOrder", json!({ "sheet_id": sheet_id.as_str(), "column": col, "reference_level": ref_level }))
            .map_err(|err| err.to_string())?;
        if let Some(ds) = app.document().data_sheet(&sheet_id).cloned() {
            app.grid.variables_dialog.sync_from_sheet(&ds);
        }
        let payload = serde_json::to_value(outcome).map_err(|e| format!("serialize ref outcome: {e}"))?;
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: format!("mv_formula.vars_level_ref:{idx}"),
            command_id: Some(String::from("table.setCategoricalOrder")),
            command_result: Some(payload),
        }));
    }
    if sub == "vars_recode_stage" {
        let _ = ensure_variables_dialog_synced(app);
        let staged = stage_recode_inputs(app, state);
        if staged {
            app.interaction.text_field_buffers.remove(WID_RECODE_FROM);
            app.interaction.text_field_buffers.remove(WID_RECODE_TO);
            state.recode_from.clear();
            state.recode_to.clear();
        }
        return Ok(Some(UiEventOutcome { handled: true, action: format!("mv_formula.vars_recode_stage:{staged}"), ..Default::default() }));
    }
    if sub == "vars_recode_clear" {
        app.grid.variables_dialog.recode_mappings.clear();
        app.interaction.text_field_buffers.remove(WID_RECODE_FROM);
        app.interaction.text_field_buffers.remove(WID_RECODE_TO);
        state.recode_from.clear();
        state.recode_to.clear();
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_recode_cleared"), ..Default::default() }));
    }
    if sub == "vars_recode_apply" {
        let (sheet_id, _) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        if app.grid.variables_dialog.recode_mappings.is_empty() {
            let _ = stage_recode_inputs(app, state);
        }
        let Some(intent) = app.grid.variables_dialog.build_recode_command(&sheet_id) else {
            return Err(String::from("Enter both From and To level labels before recoding."));
        };
        let outcome = app.execute_command(&intent.command_id, intent.params).map_err(|err| err.to_string())?;
        app.grid.variables_dialog.recode_mappings.clear();
        app.interaction.text_field_buffers.remove(WID_RECODE_FROM);
        app.interaction.text_field_buffers.remove(WID_RECODE_TO);
        state.recode_from.clear();
        state.recode_to.clear();
        if let Some(ds) = app.document().data_sheet(&sheet_id).cloned() {
            app.grid.variables_dialog.sync_from_sheet(&ds);
        }
        state.status_notice = Some(String::from("Recoded categorical levels."));
        let payload = serde_json::to_value(outcome).map_err(|e| format!("serialize recode outcome: {e}"))?;
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: String::from("mv_formula.vars_recode_applied"),
            command_id: Some(String::from("table.recodeLevels")),
            command_result: Some(payload),
        }));
    }
    Ok(None)
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `mv_formula` domain.
///
/// # Errors
/// Returns an error string when an invoked formula/role/recoding command fails validation.
pub fn handle_custom_action(app: &mut UiApp, state: &mut MvFormulaViewState, payload: &str) -> Result<UiEventOutcome, String> {
    state.revision = state.revision.wrapping_add(1);
    if payload == "panel_focus" {
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.panel_focus"), ..Default::default() });
    }
    if payload == "toggle_panel" {
        state.open_role_dropdown_col = None;
        state.right_panel_open = !state.right_panel_open;
        if state.right_panel_open
            && !app.grid.formula_bar.active
            && let Some((_, ds)) = active_mv_sheet(app)
            && let TableArchetype::MultipleVariables(mv) = &ds.archetype
            && !mv.variables.is_empty()
        {
            let col = app.grid.tool.active.map(|c| c.col).filter(|&c| c < mv.variables.len()).unwrap_or(0);
            let doc = app.document().clone();
            app.grid.formula_bar.open_for_column(&ds, col, Some(&doc));
            sync_text_buffer(app, WID_FORMULA_INPUT, &app.grid.formula_bar.input.clone());
            sync_text_buffer(app, WID_NAME_INPUT, &app.grid.formula_bar.target_title.clone());
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.panel_toggled"), ..Default::default() });
    }
    if payload == "close_panel" {
        state.right_panel_open = false;
        state.open_role_dropdown_col = None;
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.panel_closed"), ..Default::default() });
    }
    if payload == "new_calc" {
        return open_editor_for_new_column(app, state);
    }
    if payload == "commit" {
        return commit_formula_from_action(app, state);
    }
    if payload == "cancel" {
        return Ok(cancel_formula_from_action(app, state));
    }
    if payload == "toggle_defs" {
        state.show_formula_definitions = !state.show_formula_definitions;
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.defs_toggled"), ..Default::default() });
    }
    if payload == "func_search_clear" {
        state.function_search.clear();
        app.interaction.text_field_buffers.remove(WID_FUNC_SEARCH);
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.func_search_cleared"), ..Default::default() });
    }
    if payload == "role_strip_prev" {
        state.badge_scroll_col = state.badge_scroll_col.saturating_sub(1);
        state.open_role_dropdown_col = None;
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.role_strip_prev"), ..Default::default() });
    }
    if payload == "role_strip_next" {
        state.badge_scroll_col = state.badge_scroll_col.saturating_add(1);
        state.open_role_dropdown_col = None;
        return Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.role_strip_next"), ..Default::default() });
    }
    if let Some(raw_col) = payload.strip_prefix("open_col:")
        && let Ok(col) = raw_col.parse::<usize>()
    {
        return open_editor_for_column(app, state, col);
    }
    if let Some(raw_col) = payload.strip_prefix("insert_col:")
        && let Ok(col) = raw_col.parse::<usize>()
    {
        if let Some((_, ds)) = active_mv_sheet(app)
            && let TableArchetype::MultipleVariables(mv) = &ds.archetype
            && let Some(var) = mv.variables.get(col)
        {
            let token = format_column_ref(&var.title);
            insert_snippet_into_formula(app, &token);
            return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.insert_col:{col}"), ..Default::default() });
        }
        return Ok(UiEventOutcome { handled: false, action: String::from("mv_formula.insert_col_missing"), ..Default::default() });
    }
    if let Some(name) = payload.strip_prefix("insert_info:") {
        let token = format!("Info(\"{name}\")");
        insert_snippet_into_formula(app, &token);
        return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.insert_info:{name}"), ..Default::default() });
    }
    if let Some(fn_name) = payload.strip_prefix("insert_fn:") {
        if let Some(spec) = FORMULA_FUNCTIONS.iter().find(|f| f.name == fn_name) {
            state.selected_function_name = Some(spec.name.to_owned());
            insert_snippet_into_formula(app, spec.insert_text);
            return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.insert_fn:{fn_name}"), ..Default::default() });
        }
        return Ok(UiEventOutcome { handled: false, action: String::from("mv_formula.insert_fn_unknown"), ..Default::default() });
    }
    if let Some(cat) = payload.strip_prefix("func_cat:") {
        state.function_category = if cat == "all" { String::new() } else { cat.to_owned() };
        return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.func_cat:{cat}"), ..Default::default() });
    }
    if let Some(raw_col) = payload.strip_prefix("toggle_role_menu:")
        && let Ok(col) = raw_col.parse::<usize>()
    {
        state.open_role_dropdown_col = if state.open_role_dropdown_col == Some(col) { None } else { Some(col) };
        return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.toggle_role_menu:{col}"), ..Default::default() });
    }
    if let Some(rest) = payload.strip_prefix("set_role:")
        && let Some((raw_col, role)) = rest.split_once(':')
        && let Ok(col) = raw_col.parse::<usize>()
    {
        return execute_role_change(app, state, col, role);
    }
    if let Some(raw_col) = payload.strip_prefix("toggle_role_lock:")
        && let Ok(col) = raw_col.parse::<usize>()
    {
        return handle_role_lock_toggle(app, state, col);
    }
    if let Some(vars_out) = handle_variables_action(app, state, payload)? {
        return Ok(vars_out);
    }
    Ok(UiEventOutcome::default())
}

/// Desired pixel width for a docked right-side calculated variable editor panel.
#[must_use]
pub fn right_panel_desired_width(app: &UiApp, ds: &DataSheet, sheet_rect: LayoutRectPx) -> u32 {
    right_panel_desired_width_for_scale(app, ds, sheet_rect, 1.0)
}

/// HiDPI-scaled right panel width reservation for the calculated-variable editor dock.
#[must_use]
pub fn right_panel_desired_width_for_scale(app: &UiApp, ds: &DataSheet, sheet_rect: LayoutRectPx, scale: f64) -> u32 {
    if !matches!(ds.archetype, TableArchetype::MultipleVariables(_)) {
        return 0;
    }
    let st = &app.view_shards.mv_formula;
    if !st.right_panel_open && !st.variables_drawer_open {
        return 0;
    }
    let min_w = scaled_u32(220.0, scale, 180, 440);
    let max_w = scaled_u32(308.0, scale, 260, 620);
    (sheet_rect.width / 2).clamp(min_w, max_w).min(sheet_rect.width.saturating_sub(16))
}

fn effective_commit_ok(builder: &LayoutBuilder<'_>, ds: &DataSheet) -> (bool, Option<String>) {
    let fb = &builder.app.grid.formula_bar;
    let typed_name = builder.app.interaction.text_field_buffers.get(WID_NAME_INPUT).map(|b| b.text.trim());
    let has_target = fb.target_column.is_some() || typed_name.unwrap_or(fb.target_title.trim()).is_empty().eq(&false);
    let typed_formula = builder.app.interaction.text_field_buffers.get(WID_FORMULA_INPUT).map(|b| b.text.as_str());
    let non_empty_formula = typed_formula.map_or(fb.is_valid, |raw| !raw.trim().strip_prefix('=').unwrap_or(raw.trim()).trim().is_empty());
    let commit_ok = !ds.frozen && fb.active && has_target && fb.is_valid && fb.parse_error.is_none() && non_empty_formula;
    let reason = if ds.frozen {
        Some(String::from("Data sheet is frozen (read-only)."))
    } else if !fb.active {
        Some(String::from("Formula bar is inactive."))
    } else if !has_target {
        Some(String::from("Specify a variable name or target column."))
    } else if let Some(err) = &fb.parse_error {
        Some(err.clone())
    } else if !fb.is_valid || !non_empty_formula {
        Some(String::from("Enter a valid formula expression."))
    } else {
        None
    };
    (commit_ok, reason)
}

/// Emit the calculated-variable formula bar strip when active or on a `MultipleVariables` sheet,
/// returning its allocated pixel height (`52`, `24`, or `0`).
pub fn build_formula_bar_row(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, sheet_rect: LayoutRectPx, show_formula: bool) -> u32 {
    let scale = builder.scale();
    if !show_formula || sheet_rect.height <= scaled_u32(80.0, scale, 70, 160) || sheet_rect.width < scaled_u32(120.0, scale, 100, 240) {
        return 0;
    }
    // In split-pane mode, prevent duplicate singleton formula bar/overlay node IDs.
    if builder.tree.nodes.iter().any(|n| n.id.as_str() == WID_FORMULA_INPUT || n.id.as_str() == "mv_formula.top_summary") {
        return 0;
    }

    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return build_legacy_formula_strip(builder, ds, sheet_rect);
    };

    let state = builder.app.view_shards.mv_formula.clone();
    let show_badges_row = sheet_rect.height >= scaled_u32(160.0, scale, 130, 320) && sheet_rect.width >= scaled_u32(240.0, scale, 200, 480);
    let top_row_h = scaled_u32(22.0, scale, 18, 44);
    let badge_row_h = scaled_u32(22.0, scale, 18, 44);
    let formula_h = if show_badges_row { top_row_h + badge_row_h + scaled_u32(8.0, scale, 6, 16) } else { top_row_h + 2 };

    build_mv_top_toolbar_row(builder, ds, sheet_rect, &state);
    if show_badges_row {
        let badges_y = sheet_rect.y + i32::try_from(top_row_h + 4).unwrap_or(26);
        let badges_rect = LayoutRectPx::new(sheet_rect.x + 4, badges_y, sheet_rect.width.saturating_sub(8), badge_row_h);
        build_mv_column_badges(builder, ds, mv, badges_rect, &state);
    }

    let body_top = sheet_rect.y + i32::try_from(formula_h).unwrap_or(52) + 2;
    let body_h = sheet_rect.height.saturating_sub(formula_h + 4);
    if body_h >= scaled_u32(120.0, scale, 100, 240) && sheet_rect.width >= scaled_u32(240.0, scale, 200, 480) {
        let panel_w = (sheet_rect.width / 2)
            .clamp(scaled_u32(224.0, scale, 190, 448), scaled_u32(308.0, scale, 260, 620))
            .min(sheet_rect.width.saturating_sub(12));
        let min_drawer_gap = scaled_u32(224.0, scale, 190, 448);
        let narrow_conflict = state.right_panel_open && state.variables_drawer_open && sheet_rect.width < panel_w + min_drawer_gap;
        if state.right_panel_open {
            let panel_x = if builder.tree.is_rtl { sheet_rect.x + 4 } else { sheet_rect.right() - i32::try_from(panel_w).unwrap_or(280) - 4 };
            let panel_rect = LayoutRectPx::new(panel_x, body_top, panel_w, body_h);
            build_calculated_variable_panel(builder, ds, mv, panel_rect, &state);
        }
        if state.variables_drawer_open && !narrow_conflict {
            let drawer_w = if state.right_panel_open {
                sheet_rect.width.saturating_sub(panel_w + 14).clamp(scaled_u32(212.0, scale, 180, 424), scaled_u32(276.0, scale, 230, 552))
            } else {
                (sheet_rect.width / 2)
                    .clamp(scaled_u32(228.0, scale, 190, 456), scaled_u32(304.0, scale, 260, 608))
                    .min(sheet_rect.width.saturating_sub(12))
            };
            let drawer_x = if state.right_panel_open {
                if builder.tree.is_rtl {
                    sheet_rect.x + i32::try_from(panel_w + 10).unwrap_or(290)
                } else {
                    sheet_rect.right() - i32::try_from(panel_w + drawer_w + 10).unwrap_or(560)
                }
            } else if builder.tree.is_rtl {
                sheet_rect.x + 4
            } else {
                sheet_rect.right() - i32::try_from(drawer_w).unwrap_or(280) - 4
            };
            let drawer_rect = LayoutRectPx::new(drawer_x, body_top, drawer_w, body_h);
            build_variables_drawer(builder, ds, mv, drawer_rect, &state);
        }
    }

    if let Some(open_col) = state.open_role_dropdown_col.filter(|&c| c < mv.variables.len()) {
        emit_role_dropdown_popup(builder, ds, mv, open_col, sheet_rect);
    }

    formula_h
}

fn build_legacy_formula_strip(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, sheet_rect: LayoutRectPx) -> u32 {
    let scale = builder.scale();
    let formula_h = scaled_u32(24.0, scale, 20, 48);
    let btn_w = scaled_u32(54.0, scale, 46, 108);
    let input_w = sheet_rect.width.saturating_sub(btn_w * 2 + 16).max(40);
    let fx_rect = LayoutRectPx::new(sheet_rect.x + 4, sheet_rect.y + 2, input_w, formula_h - 4);
    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_FORMULA_INPUT,
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: fx_rect,
            label: "Formula Bar",
            fallback_text: &builder.app.grid.formula_bar.input,
            keyboard_path: "F2",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    let (commit_ok, reason) = effective_commit_ok(builder, ds);
    let commit_en =
        if commit_ok { ControlEnablement::enabled() } else { ControlEnablement::disabled(reason.as_deref().unwrap_or("Formula bar inactive")) };
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "formula_bar.commit",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: LayoutRectPx::new(fx_rect.right() + 4, sheet_rect.y + 2, btn_w, formula_h - 4),
            label: mv_tr(&builder.tree.locale, "btn.apply", "Apply"),
            keyboard_path: "Enter",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: commit_en,
            action: custom_action("commit"),
        },
    );
    let cancel_en =
        if builder.app.grid.formula_bar.active { ControlEnablement::enabled() } else { ControlEnablement::disabled("Formula bar inactive") };
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "formula_bar.cancel",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: LayoutRectPx::new(fx_rect.right() + i32::try_from(btn_w + 8).unwrap_or(62), sheet_rect.y + 2, btn_w, formula_h - 4),
            label: mv_tr(&builder.tree.locale, "btn.cancel", "Cancel"),
            keyboard_path: "Escape",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: cancel_en,
            action: custom_action("cancel"),
        },
    );
    formula_h
}

fn build_mv_top_toolbar_row(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, sheet_rect: LayoutRectPx, state: &MvFormulaViewState) {
    let scale = builder.scale();
    let row_y = sheet_rect.y + 2;
    let row_h = scaled_u32(22.0, scale, 18, 44);
    let compact = sheet_rect.width < scaled_u32(540.0, scale, 460, 1080);
    let w_new = scaled_u32(if compact { 46.0 } else { 58.0 }, scale, 40, 120);
    let w_vars = scaled_u32(if compact { 66.0 } else { 84.0 }, scale, 58, 176);
    let w_defs = scaled_u32(if compact { 48.0 } else { 58.0 }, scale, 42, 120);
    let w_edit = scaled_u32(if compact { 52.0 } else { 66.0 }, scale, 46, 140);
    let btn_gap = scaled_u32(4.0, scale, 3, 8);
    let right_group_w = w_new + w_vars + w_defs + w_edit + btn_gap.saturating_mul(3);
    let right_x = sheet_rect.right() - i32::try_from(right_group_w + 4).unwrap_or(260);
    let left_avail = u32::try_from((right_x - (sheet_rect.x + 8)).max(0)).unwrap_or(0);

    if !state.right_panel_open && left_avail >= scaled_u32(120.0, scale, 100, 240) {
        let btn_w = if left_avail >= scaled_u32(220.0, scale, 180, 440) { scaled_u32(48.0, scale, 40, 96) } else { scaled_u32(38.0, scale, 32, 76) };
        let input_w = left_avail.saturating_sub(btn_w * 2 + 8).max(scaled_u32(36.0, scale, 32, 80));
        let fx_rect = LayoutRectPx::new(sheet_rect.x + 4, row_y, input_w, row_h);
        emit_text_field(
            builder,
            TextFieldSpec {
                id: WID_FORMULA_INPUT,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: fx_rect,
                label: "Formula Bar",
                fallback_text: &builder.app.grid.formula_bar.input,
                keyboard_path: "F2",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        let (commit_ok, reason) = effective_commit_ok(builder, ds);
        let commit_en =
            if commit_ok { ControlEnablement::enabled() } else { ControlEnablement::disabled(reason.as_deref().unwrap_or("Formula bar inactive")) };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "formula_bar.commit",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(fx_rect.right() + 4, row_y, btn_w, row_h),
                label: mv_tr(&builder.tree.locale, "btn.apply", "Apply"),
                keyboard_path: "Enter",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: commit_en,
                action: custom_action("commit"),
            },
        );
        let cancel_en =
            if builder.app.grid.formula_bar.active { ControlEnablement::enabled() } else { ControlEnablement::disabled("Formula bar inactive") };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "formula_bar.cancel",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(fx_rect.right() + i32::try_from(btn_w + 8).unwrap_or(52), row_y, btn_w, row_h),
                label: mv_tr(&builder.tree.locale, "btn.cancel", "Cancel"),
                keyboard_path: "Escape",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: cancel_en,
                action: custom_action("cancel"),
            },
        );
    } else if left_avail >= 40 {
        let fb = &builder.app.grid.formula_bar;
        let target_lbl = if !fb.target_title.trim().is_empty() {
            format!("fx: {}", fb.target_title.trim())
        } else {
            mv_tr(&builder.tree.locale, "inspector.calculated_variable", "Calculated Variable").to_owned()
        };
        let sum_rect = LayoutRectPx::new(sheet_rect.x + 4, row_y, left_avail, row_h);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from("mv_formula.top_summary"),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: sum_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, target_lbl.clone(), FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        let size_px = (11.5 * scale).clamp(9.0, 24.0);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: sum_rect,
                pad_x: 4,
                text: &target_lbl,
                style: &UiTextStyle::ui_bold(size_px),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let mut bx = right_x.max(sheet_rect.x + 4);
    let loc = builder.tree.locale.clone();
    let new_en = if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() };
    let btns: [(&str, &str, bool, ControlEnablement, &str); 4] = [
        ("mv_formula.new_calc", mv_tr(&loc, "mv.new_calc", "+ Calc"), false, new_en, "new_calc"),
        (
            "mv_formula.variables_btn",
            mv_tr(&loc, "mv.variables_btn", "Variables..."),
            state.variables_drawer_open,
            ControlEnablement::enabled(),
            "toggle_vars_drawer",
        ),
        (
            "mv_formula.defs_toggle",
            mv_tr(&loc, "mv.defs_toggle", "fx Defs"),
            state.show_formula_definitions,
            ControlEnablement::enabled(),
            "toggle_defs",
        ),
        ("mv_formula.panel_toggle", mv_tr(&loc, "mv.editor_btn", "fx Editor"), state.right_panel_open, ControlEnablement::enabled(), "toggle_panel"),
    ];
    let widths = [w_new, w_vars, w_defs, w_edit];
    for ((id, lbl, selected, en, payload), w) in btns.into_iter().zip(widths) {
        if bx + i32::try_from(w).unwrap_or(50) > sheet_rect.right() - 2 {
            break;
        }
        emit_button(
            builder,
            ButtonWidgetSpec {
                id,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(bx, row_y, w, row_h),
                label: lbl,
                keyboard_path: id,
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: en,
                action: custom_action(payload),
            },
        );
        bx += i32::try_from(w + btn_gap).unwrap_or(54);
    }
}

/// Emit interactive clickable variable-type role badges (`Continuous`, `Categorical`, `Label`, `Calculated`)
/// with lock/unlock toggles across `rect`.
pub fn build_mv_column_badges(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    rect: LayoutRectPx,
    state: &MvFormulaViewState,
) {
    let scale = builder.scale();
    if rect.width < scaled_u32(80.0, scale, 70, 160) || rect.height < scaled_u32(18.0, scale, 16, 36) || mv.variables.is_empty() {
        return;
    }
    let total = mv.variables.len();
    let start_col = state.badge_scroll_col.min(total.saturating_sub(1));
    let lock_w = scaled_u32(22.0, scale, 18, 44);
    let badge_overhead = lock_w + 6;
    let min_badge_w = scaled_u32(76.0, scale, 64, 152);
    let min_slot_w = min_badge_w + badge_overhead;
    let max_slot_w = scaled_u32(158.0, scale, 130, 316);
    let nav_w =
        if total > 1 && (start_col > 0 || rect.width < (total as u32).saturating_mul(min_slot_w)) { scaled_u32(20.0, scale, 18, 40) } else { 0 };
    let mut cx = rect.x;
    let right_limit = rect.right() - i32::try_from(nav_w).unwrap_or(0);

    if nav_w > 0 {
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "mv_formula.role_strip.prev",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(cx, rect.y, nav_w, rect.height),
                label: "<",
                keyboard_path: "mv_formula.role_strip.prev",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: if start_col > 0 { ControlEnablement::enabled() } else { ControlEnablement::disabled("At first variable column") },
                action: custom_action("role_strip_prev"),
            },
        );
        cx += i32::try_from(nav_w + 3).unwrap_or(23);
    }

    let remaining_w = u32::try_from((right_limit - cx).max(0)).unwrap_or(0);
    let visible_count = mv.variables.len().saturating_sub(start_col).max(1);
    let item_slot_w = (remaining_w / (visible_count as u32)).clamp(min_slot_w, max_slot_w);
    let badge_w = item_slot_w.saturating_sub(badge_overhead).max(min_badge_w);
    let size_px = (10.5 * scale).clamp(8.5, 22.0);
    let style = UiTextStyle::ui_sans(size_px);

    for (col_idx, var) in mv.variables.iter().enumerate().skip(start_col) {
        if cx + i32::try_from(badge_w + lock_w + 2).unwrap_or(100) > right_limit {
            break;
        }
        let is_open = state.open_role_dropdown_col == Some(col_idx);
        let role_color = variable_role_color(var.role, &builder.app.theme);
        let badge_rect = LayoutRectPx::new(cx, rect.y, badge_w, rect.height);
        let badge_id = format!("mv_formula.role_badge.{col_idx}");
        let badge_tag = variable_role_badge(var.role);
        let text = match (&var.source, state.show_formula_definitions) {
            (VariableSource::Calculated { formula }, true) => format!("{}: ={formula}", var.title),
            _ => format!("{}: {badge_tag}", var.title),
        };
        let st = widget_state(builder, &badge_id, ControlEnablement::enabled());
        let mut a11y = AccessibilityMeta::new(WidgetRole::ComboBox, format!("Variable role for {}", var.title), FocusScope::Workspace)
            .with_keyboard_path(badge_id.clone());
        a11y.value_text = Some(role_wire_name(var.role).to_owned());
        a11y.expanded = Some(is_open);

        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(badge_id),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: badge_rect,
            a11y,
            state: st.clone(),
            action: custom_action(format!("toggle_role_menu:{col_idx}")),
        });
        let bg = if is_open || st.hovered { builder.palette.header } else { builder.palette.surface };
        let border = if is_open { builder.palette.accent } else { role_color };
        builder.paint_fill(ZLayer::Workspace, Some(&wid), badge_rect, bg);
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), badge_rect, border, 1);
        builder.paint_fill(
            ZLayer::Workspace,
            Some(&wid),
            LayoutRectPx::new(badge_rect.x + 1, badge_rect.y + 1, 4, badge_rect.height.saturating_sub(2)),
            role_color,
        );

        let arrow_reserve = scaled_u32(17.0, scale, 14, 34);
        let txt_rect = LayoutRectPx::new(badge_rect.x + 5, badge_rect.y, badge_rect.width.saturating_sub(arrow_reserve), badge_rect.height);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: txt_rect,
                pad_x: 3,
                text: &text,
                style: &style,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        let ax = badge_rect.right() - i32::try_from(scaled_u32(8.0, scale, 6, 16)).unwrap_or(8);
        let ay = badge_rect.center()[1];
        builder.paint_line(ZLayer::Workspace, Some(&wid), [ax - 3, ay - 1], [ax, ay + 2], builder.palette.text, 1);
        builder.paint_line(ZLayer::Workspace, Some(&wid), [ax, ay + 2], [ax + 3, ay - 1], builder.palette.text, 1);

        let lock_id = format!("mv_formula.role_lock.{col_idx}");
        let lock_rect = LayoutRectPx::new(badge_rect.right() + 2, rect.y, lock_w, rect.height);
        let is_calc = matches!(var.source, VariableSource::Calculated { .. });
        let lock_en = if ds.frozen {
            ControlEnablement::disabled("Data sheet is frozen (read-only).")
        } else if is_calc {
            ControlEnablement::disabled("Calculated variable role is formula-locked.")
        } else {
            ControlEnablement::enabled()
        };
        let locked = !var.role_inferred || is_calc;
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &lock_id,
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: lock_rect,
                label: if locked { "L" } else { "A" },
                keyboard_path: &lock_id,
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: locked,
                shortcut: None,
                enablement: lock_en,
                action: custom_action(format!("toggle_role_lock:{col_idx}")),
            },
        );
        cx = lock_rect.right() + 4;
    }

    if nav_w > 0 {
        let next_x = rect.right() - i32::try_from(nav_w).unwrap_or(20);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: "mv_formula.role_strip.next",
                parent: None,
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(next_x, rect.y, nav_w, rect.height),
                label: ">",
                keyboard_path: "mv_formula.role_strip.next",
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected: false,
                shortcut: None,
                enablement: if start_col + 1 < total { ControlEnablement::enabled() } else { ControlEnablement::disabled("At last variable column") },
                action: custom_action("role_strip_next"),
            },
        );
    }
}

fn emit_role_dropdown_popup(builder: &mut LayoutBuilder<'_>, ds: &DataSheet, mv: &MultipleVariablesTable, col: usize, sheet_rect: LayoutRectPx) {
    let Some(var) = mv.variables.get(col) else {
        return;
    };
    let scale = builder.scale();
    let row_h = scaled_u32(22.0, scale, 18, 44);
    let pop_w = scaled_u32(190.0, scale, 160, 380).min(sheet_rect.width.saturating_sub(8));
    let anchor = builder
        .tree
        .node(&format!("mv_formula.role_badge.{col}"))
        .map_or(LayoutRectPx::new(sheet_rect.x + 4, sheet_rect.y + 48, 120, row_h), |n| n.bounds.rect_px);
    let is_calc = matches!(var.source, VariableSource::Calculated { .. });
    let loc = builder.tree.locale.clone();
    let opts: [(&str, &str, bool); 5] = [
        ("continuous", mv_tr(&loc, "mv.role.continuous", "Continuous"), !ds.frozen && !is_calc),
        ("categorical", mv_tr(&loc, "mv.role.categorical", "Categorical"), !ds.frozen && !is_calc),
        ("label", mv_tr(&loc, "mv.role.label", "Label"), !ds.frozen && !is_calc),
        ("calculated", mv_tr(&loc, "mv.role.calculated", "Calculated"), !ds.frozen),
        ("auto", mv_tr(&loc, "mv.role.auto", "Auto-infer"), !ds.frozen && !is_calc),
    ];
    let pop_h = (opts.len() as u32) * row_h + 6;
    let pop_x = anchor.x.min(sheet_rect.right() - i32::try_from(pop_w).unwrap_or(190) - 2).max(sheet_rect.x + 2);
    let pop_y = (anchor.bottom() + 2).min(sheet_rect.bottom() - i32::try_from(pop_h).unwrap_or(116) - 2).max(sheet_rect.y + 2);
    let pop_rect = LayoutRectPx::new(pop_x, pop_y, pop_w, pop_h);

    let mut a11y = AccessibilityMeta::new(WidgetRole::ContextMenu, format!("Role menu for {}", var.title), FocusScope::PopupMenu)
        .with_keyboard_path(format!("mv_formula.role_popup.{col}"));
    a11y.expanded = Some(true);
    let pop_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("mv_formula.role_popup.{col}")),
        parent: None,
        z_layer: ZLayer::DropdownMenu,
        rect_px: pop_rect,
        a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::DropdownMenu, Some(&pop_wid), pop_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::DropdownMenu, Some(&pop_wid), pop_rect, builder.palette.accent, 1);

    let size_px = (11.5 * scale).clamp(9.0, 24.0);
    let style = UiTextStyle::ui_sans(size_px);
    for (idx, (key, label, enabled)) in opts.into_iter().enumerate() {
        let item_rect =
            LayoutRectPx::new(pop_rect.x + 3, pop_rect.y + 3 + i32::try_from(idx as u32 * row_h).unwrap_or(0), pop_w.saturating_sub(6), row_h - 2);
        let item_id = format!("mv_formula.role_opt.{col}.{key}");
        let selected = match key {
            "continuous" => var.role == VariableRole::Continuous && !var.role_inferred,
            "categorical" => var.role == VariableRole::Categorical && !var.role_inferred,
            "label" => var.role == VariableRole::Label && !var.role_inferred,
            "calculated" => var.role == VariableRole::Calculated,
            "auto" => var.role_inferred && !is_calc,
            _ => false,
        };
        let en = if enabled {
            ControlEnablement::enabled()
        } else if ds.frozen {
            ControlEnablement::disabled("Data sheet is frozen (read-only).")
        } else {
            ControlEnablement::disabled("Calculated variable role is bound to its formula.")
        };
        let st = widget_state(builder, &item_id, en.clone());
        let mut item_a11y = selectable_meta(WidgetRole::MenuItem, label, FocusScope::PopupMenu, &item_id, selected);
        item_a11y.checked = Some(selected);
        let item_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item_id),
            parent: Some(pop_wid.clone()),
            z_layer: ZLayer::DropdownMenu,
            rect_px: item_rect,
            a11y: item_a11y,
            state: st.clone(),
            action: if en.enabled { custom_action(format!("set_role:{col}:{key}")) } else { WidgetAction::NoOp },
        });
        let bg = if selected {
            builder.palette.accent
        } else if st.hovered && en.enabled {
            builder.palette.header
        } else {
            builder.palette.surface
        };
        let fg = if !en.enabled {
            builder.palette.muted
        } else if selected {
            Rgba8::WHITE
        } else {
            builder.palette.text
        };
        builder.paint_fill(ZLayer::DropdownMenu, Some(&item_wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &item_wid,
            FittedTextSpec {
                z_layer: ZLayer::DropdownMenu,
                rect: item_rect,
                pad_x: 6,
                text: label,
                style: &style,
                color: fg,
                scroll_container: None,
            },
        );
    }
}

/// Emit the full right-side Calculated Variable Editor Panel (`R10.8` `:696`).
pub fn build_calculated_variable_panel(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    panel_rect: LayoutRectPx,
    state: &MvFormulaViewState,
) {
    let scale = builder.scale();
    if panel_rect.width < scaled_u32(180.0, scale, 150, 360) || panel_rect.height < scaled_u32(200.0, scale, 170, 400) {
        return;
    }
    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.panel"),
        parent: None,
        z_layer: ZLayer::FloatingOverlay,
        rect_px: panel_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Calculated Variable Editor", FocusScope::Workspace, "F2"),
        state: WidgetVisualState::default(),
        action: custom_action("panel_focus"),
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&panel_wid), panel_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&panel_wid), panel_rect, builder.palette.accent, 1);

    let inner = panel_rect.inset(scaled_u32(6.0, scale, 4, 12));
    let loc = builder.tree.locale.clone();
    let size_px = (11.0 * scale).clamp(8.5, 24.0);
    let lbl_style = UiTextStyle::ui_bold(size_px);
    let text_style = UiTextStyle::ui_sans(size_px);
    let lbl_h = scaled_u32(18.0, scale, 16, 36);
    let hdr_h = scaled_u32(20.0, scale, 18, 40);
    let row_h = scaled_u32(22.0, scale, 18, 44);
    let gap_h = scaled_u32(4.0, scale, 3, 8);
    let hdr_step = i32::try_from(hdr_h + gap_h).unwrap_or(24);
    let lbl_step = i32::try_from(lbl_h).unwrap_or(18);
    let row_step = i32::try_from(row_h + gap_h).unwrap_or(26);
    let close_w = scaled_u32(22.0, scale, 18, 44);
    let mut cy = inner.y;

    // 1. Header + close
    let hdr_w = inner.width.saturating_sub(close_w + gap_h);
    emit_overlay_label(
        builder,
        &panel_wid,
        "mv_formula.panel_title",
        LayoutRectPx::new(inner.x, cy, hdr_w, hdr_h),
        mv_tr(&loc, "inspector.calculated_variable", "Calculated Variable"),
        &lbl_style,
        builder.palette.text,
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.panel_close",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.right() - i32::try_from(close_w).unwrap_or(22), cy, close_w, hdr_h),
            label: "X",
            keyboard_path: "Escape",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("close_panel"),
        },
    );
    cy += hdr_step;

    // 2. Variable Name input
    emit_overlay_label(
        builder,
        &panel_wid,
        "mv_formula.name_lbl",
        LayoutRectPx::new(inner.x, cy, inner.width, lbl_h),
        mv_tr(&loc, "mv.panel.var_name", "Variable Name"),
        &text_style,
        builder.palette.muted,
    );
    cy += lbl_step;
    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_NAME_INPUT,
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, inner.width, row_h),
            label: mv_tr(&loc, "mv.panel.var_name", "Variable Name"),
            fallback_text: &builder.app.grid.formula_bar.target_title,
            keyboard_path: WID_NAME_INPUT,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(301),
            multiline: false,
            enablement: if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() },
        },
    );
    cy += row_step;

    // 3. Live Formula input
    emit_overlay_label(
        builder,
        &panel_wid,
        "mv_formula.formula_lbl",
        LayoutRectPx::new(inner.x, cy, inner.width, lbl_h),
        mv_tr(&loc, "mv.panel.live_formula", "Live Formula"),
        &text_style,
        builder.palette.muted,
    );
    cy += lbl_step;
    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_FORMULA_INPUT,
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, inner.width, row_h),
            label: mv_tr(&loc, "mv.panel.live_formula", "Live Formula"),
            fallback_text: &builder.app.grid.formula_bar.input,
            keyboard_path: "F2",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(302),
            multiline: false,
            enablement: if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() },
        },
    );
    cy += row_step;

    // 4. Live syntax / circularity validation banner
    let fb = &builder.app.grid.formula_bar;
    let (commit_ok, reason) = effective_commit_ok(builder, ds);
    let (val_msg, val_bg, val_border, val_fg) = if let Some(err) = &fb.parse_error {
        (err.clone(), Rgba8::rgba(220, 38, 38, 28), builder.palette.link_red, builder.palette.link_red)
    } else if fb.is_valid && commit_ok {
        (
            mv_tr(&loc, "mv.panel.valid_formula", "Formula valid").to_owned(),
            Rgba8::rgba(22, 163, 74, 28),
            builder.palette.link_green,
            builder.palette.link_green,
        )
    } else if fb.is_valid {
        (
            reason.clone().unwrap_or_else(|| String::from("Enter variable name")),
            builder.palette.surface,
            builder.palette.border,
            builder.palette.muted,
        )
    } else {
        (
            state.validation_feedback.clone().unwrap_or_else(|| String::from("Type = expression or pick a column/function")),
            builder.palette.surface,
            builder.palette.border,
            builder.palette.muted,
        )
    };
    let val_rect = LayoutRectPx::new(inner.x, cy, inner.width, row_h);
    let val_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.validation_status"),
        parent: Some(panel_wid.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: val_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Banner, val_msg.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&val_wid), val_rect, val_bg);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&val_wid), val_rect, val_border, 1);
    paint_text_fitted(
        builder,
        &val_wid,
        FittedTextSpec {
            z_layer: ZLayer::FloatingOverlay,
            rect: val_rect,
            pad_x: 5,
            text: &val_msg,
            style: &text_style,
            color: val_fg,
            scroll_container: None,
        },
    );
    cy += row_step;

    // 5. Apply / Cancel / Defs row
    let act_w = ((inner.width.saturating_sub(8)) / 3).max(scaled_u32(46.0, scale, 40, 96));
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "formula_bar.commit",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, act_w, row_h),
            label: mv_tr(&loc, "btn.apply", "Apply"),
            keyboard_path: "Enter",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(303),
            primary: commit_ok,
            selected: false,
            shortcut: None,
            enablement: if commit_ok {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled(reason.as_deref().unwrap_or("Formula incomplete"))
            },
            action: custom_action("commit"),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "formula_bar.cancel",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from(act_w + 4).unwrap_or(52), cy, act_w, row_h),
            label: mv_tr(&loc, "btn.cancel", "Cancel"),
            keyboard_path: "Escape",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(304),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("cancel"),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.panel_defs_toggle",
            parent: Some(panel_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from((act_w + 4) * 2).unwrap_or(104), cy, inner.width.saturating_sub((act_w + 4) * 2), row_h),
            label: mv_tr(&loc, "mv.defs_toggle", "fx Defs"),
            keyboard_path: "mv_formula.panel_defs_toggle",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(305),
            primary: false,
            selected: state.show_formula_definitions,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("toggle_defs"),
        },
    );
    cy += row_step;

    // 6. Optional Formula Definitions summary when toggle enabled
    let rem_total = u32::try_from((inner.bottom() - cy).max(0)).unwrap_or(0);
    if state.show_formula_definitions && rem_total >= scaled_u32(150.0, scale, 120, 300) {
        let defs_box_h = (rem_total / 4).clamp(scaled_u32(44.0, scale, 36, 88), scaled_u32(72.0, scale, 56, 144));
        emit_overlay_label(
            builder,
            &panel_wid,
            "mv_formula.defs_lbl",
            LayoutRectPx::new(inner.x, cy, inner.width, lbl_h),
            mv_tr(&loc, "mv.panel.defs_heading", "Formula Definitions"),
            &lbl_style,
            builder.palette.text,
        );
        cy += lbl_step;
        build_formula_definitions_list(builder, &panel_wid, mv, LayoutRectPx::new(inner.x, cy, inner.width, defs_box_h), &text_style);
        cy += i32::try_from(defs_box_h + gap_h).unwrap_or(48);
    }

    // 7. Clickable Column-Variable List
    let rem_after_defs = u32::try_from((inner.bottom() - cy).max(0)).unwrap_or(0);
    if rem_after_defs < scaled_u32(100.0, scale, 84, 200) {
        return;
    }
    let cols_box_h = (rem_after_defs * 32 / 100).clamp(scaled_u32(48.0, scale, 40, 96), scaled_u32(108.0, scale, 84, 216));
    emit_overlay_label(
        builder,
        &panel_wid,
        "mv_formula.cols_lbl",
        LayoutRectPx::new(inner.x, cy, inner.width, lbl_h),
        mv_tr(&loc, "mv.panel.column_vars", "Column Variables"),
        &lbl_style,
        builder.palette.text,
    );
    cy += lbl_step;
    build_clickable_columns_list(builder, &panel_wid, mv, fb.target_column, LayoutRectPx::new(inner.x, cy, inner.width, cols_box_h), &text_style);
    cy += i32::try_from(cols_box_h + gap_h).unwrap_or(52);

    // 8. Searchable Function Reference with Parameter Hints
    if inner.bottom() - cy >= i32::try_from(scaled_u32(92.0, scale, 76, 184)).unwrap_or(92) {
        build_function_reference_section(
            builder,
            &panel_wid,
            LayoutRectPx::new(inner.x, cy, inner.width, u32::try_from((inner.bottom() - cy).max(0)).unwrap_or(0)),
            state,
            &lbl_style,
            &text_style,
        );
    }
}

fn emit_overlay_label(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    id: &str,
    rect: LayoutRectPx,
    text: &str,
    style: &UiTextStyle,
    color: Rgba8,
) {
    if rect.width < 12 || rect.height < 12 {
        return;
    }
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, text, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(builder, &wid, FittedTextSpec { z_layer: ZLayer::FloatingOverlay, rect, pad_x: 2, text, style, color, scroll_container: None });
}

fn build_formula_definitions_list(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    mv: &MultipleVariablesTable,
    rect: LayoutRectPx,
    style: &UiTextStyle,
) {
    let defs: Vec<(usize, String)> = mv
        .variables
        .iter()
        .enumerate()
        .filter_map(|(idx, v)| match &v.source {
            VariableSource::Calculated { formula } => Some((idx, format!("{} = {formula}", v.title))),
            VariableSource::Entered => None,
        })
        .collect();
    let scale = builder.scale();
    let row_h = scaled_u32(20.0, scale, 18, 40);
    let sb_reserve = scaled_u32(14.0, scale, 12, 28);
    let content_h = (defs.len().max(1) as u32) * row_h + 4;
    let scroll_id = String::from("scroll.mv_formula.defs");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.defs_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Formula Definitions", FocusScope::Workspace, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(4));
    builder.push_clip(clip);
    if defs.is_empty() {
        paint_text_fitted(
            builder,
            &list_wid,
            FittedTextSpec {
                z_layer: ZLayer::FloatingOverlay,
                rect: LayoutRectPx::new(clip.x, clip.y, clip.width, row_h.saturating_sub(2)),
                pad_x: 4,
                text: "No calculated variables defined yet.",
                style,
                color: builder.palette.muted,
                scroll_container: Some(&scroll_id),
            },
        );
    } else {
        let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
        for (pos, (col_idx, summary)) in defs.iter().enumerate() {
            let ry = clip.y + i32::try_from(pos as u32 * row_h).unwrap_or(0) - y_scroll;
            if ry + i32::try_from(row_h).unwrap_or(20) <= clip.y || ry >= clip.bottom() {
                continue;
            }
            let item_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h - 2);
            let item_id = format!("mv_formula.def_item.{col_idx}");
            let st = widget_state(builder, &item_id, ControlEnablement::enabled());
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(item_id.as_str()),
                parent: Some(list_wid.clone()),
                z_layer: ZLayer::FloatingOverlay,
                rect_px: item_rect,
                a11y: labeled_meta(WidgetRole::Button, summary.clone(), FocusScope::Workspace, &item_id),
                state: st.clone(),
                action: custom_action(format!("open_col:{col_idx}")),
            });
            builder.paint_fill(
                ZLayer::FloatingOverlay,
                Some(&wid),
                item_rect,
                if st.hovered { builder.palette.header } else { builder.palette.surface },
            );
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::FloatingOverlay,
                    rect: item_rect,
                    pad_x: 4,
                    text: summary,
                    style,
                    color: builder.palette.link_green,
                    scroll_container: Some(&scroll_id),
                },
            );
        }
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::FloatingOverlay, &scroll_meta, Some(list_wid));
}

fn build_clickable_columns_list(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    mv: &MultipleVariablesTable,
    target_col: Option<usize>,
    rect: LayoutRectPx,
    style: &UiTextStyle,
) {
    let mut items: Vec<(String, String, ControlEnablement, String)> = mv
        .variables
        .iter()
        .enumerate()
        .map(|(col_idx, var)| {
            let token = format_column_ref(&var.title);
            let lbl = format!("{token} ({})", variable_role_badge(var.role));
            let en = if Some(col_idx) == target_col {
                ControlEnablement::disabled("Cannot self-reference target variable.")
            } else {
                ControlEnablement::enabled()
            };
            (format!("mv_formula.insert_col.{col_idx}"), lbl, en, format!("insert_col:{col_idx}"))
        })
        .collect();
    for info in &builder.app.document().info_sheets {
        for (c_idx, c) in info.constants.iter().enumerate() {
            items.push((
                format!("mv_formula.insert_info.{}.{c_idx}", info.id.as_str()),
                format!("Info(\"{}\")", c.name),
                ControlEnablement::enabled(),
                format!("insert_info:{}", c.name),
            ));
        }
    }

    let scale = builder.scale();
    let row_h = scaled_u32(20.0, scale, 18, 40);
    let sb_reserve = scaled_u32(14.0, scale, 12, 28);
    let content_h = (items.len().max(1) as u32) * row_h + 4;
    let scroll_id = String::from("scroll.mv_formula.cols");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.cols_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Column Variables", FocusScope::Workspace, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    for (idx, (id, lbl, en, payload)) in items.iter().enumerate() {
        let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(20) <= clip.y || ry >= clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h - 2);
        let st = widget_state(builder, id, en.clone());
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(id.as_str()),
            parent: Some(list_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect_px: item_rect,
            a11y: labeled_meta(WidgetRole::Button, lbl.clone(), FocusScope::Workspace, id.as_str()),
            state: st.clone(),
            action: if en.enabled { custom_action(payload.as_str()) } else { WidgetAction::NoOp },
        });
        let bg = if !en.enabled {
            builder.palette.panel
        } else if st.hovered {
            builder.palette.header
        } else {
            builder.palette.surface
        };
        let fg = if en.enabled { builder.palette.text } else { builder.palette.muted };
        builder.paint_fill(ZLayer::FloatingOverlay, Some(&wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::FloatingOverlay,
                rect: item_rect,
                pad_x: 5,
                text: lbl,
                style,
                color: fg,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::FloatingOverlay, &scroll_meta, Some(list_wid));
}

fn build_function_reference_section(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    state: &MvFormulaViewState,
    lbl_style: &UiTextStyle,
    style: &UiTextStyle,
) {
    let scale = builder.scale();
    let lbl_h = scaled_u32(18.0, scale, 16, 36);
    let cat_h = scaled_u32(20.0, scale, 18, 40);
    let cat_step = i32::try_from(cat_h + scaled_u32(2.0, scale, 2, 6)).unwrap_or(22);
    let row_h_ctl = scaled_u32(22.0, scale, 18, 44);
    let row_step = i32::try_from(row_h_ctl + scaled_u32(2.0, scale, 2, 6)).unwrap_or(24);
    let loc = builder.tree.locale.clone();
    let mut cy = rect.y;
    emit_overlay_label(
        builder,
        parent,
        "mv_formula.func_lbl",
        LayoutRectPx::new(rect.x, cy, rect.width, lbl_h),
        mv_tr(&loc, "mv.panel.func_ref", "Function Reference"),
        lbl_style,
        builder.palette.text,
    );
    cy += i32::try_from(lbl_h).unwrap_or(18);

    let cats = [("all", "All"), ("math", "Math"), ("stats", "Stats"), ("window", "Win"), ("logic", "Logic")];
    let cat_w = ((rect.width.saturating_sub(12)) / 5).max(scaled_u32(36.0, scale, 32, 72));
    for (idx, (cat_id, cat_lbl)) in cats.iter().enumerate() {
        let cx = rect.x + i32::try_from(idx as u32 * (cat_w + 3)).unwrap_or(0);
        if cx + i32::try_from(cat_w).unwrap_or(36) > rect.right() {
            break;
        }
        let selected = (cat_id == &"all" && state.function_category.is_empty()) || state.function_category == *cat_id;
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("mv_formula.func_cat.{cat_id}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::FloatingOverlay,
                rect: LayoutRectPx::new(cx, cy, cat_w, cat_h),
                label: cat_lbl,
                keyboard_path: &format!("mv_formula.func_cat.{cat_id}"),
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("func_cat:{cat_id}")),
            },
        );
    }
    cy += cat_step;

    let clr_w = scaled_u32(24.0, scale, 20, 46);
    let search_w = rect.width.saturating_sub(clr_w + 4).max(40);
    let raw_query = builder.app.interaction.text_field_buffers.get(WID_FUNC_SEARCH).map_or(state.function_search.as_str(), |b| b.text.as_str());
    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_FUNC_SEARCH,
            parent: Some(parent.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(rect.x, cy, search_w, row_h_ctl),
            label: mv_tr(&loc, "mv.panel.search_funcs", "Search functions"),
            fallback_text: &state.function_search,
            keyboard_path: WID_FUNC_SEARCH,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(306),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.func.clear",
            parent: Some(parent.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(rect.x + i32::try_from(search_w + 4).unwrap_or(44), cy, clr_w, row_h_ctl),
            label: "X",
            keyboard_path: "mv_formula.func.clear",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("func_search_clear"),
        },
    );
    cy += row_step;

    let q_lower = raw_query.trim().to_ascii_lowercase();
    let filtered: Vec<&FormulaFnSpec> = FORMULA_FUNCTIONS
        .iter()
        .filter(|f| {
            (state.function_category.is_empty() || f.category == state.function_category)
                && (q_lower.is_empty()
                    || f.name.to_ascii_lowercase().contains(&q_lower)
                    || f.summary.to_ascii_lowercase().contains(&q_lower)
                    || f.params_hint.to_ascii_lowercase().contains(&q_lower))
        })
        .collect();

    let hint_spec = state
        .selected_function_name
        .as_deref()
        .and_then(|sel| FORMULA_FUNCTIONS.iter().find(|f| f.name == sel))
        .or_else(|| filtered.first().copied());
    let hint_text = hint_spec.map_or_else(
        || format!("{}: Select a function", mv_tr(&loc, "mv.panel.param_hint", "Parameter Hint")),
        |f| format!("{} — {}", f.signature, f.params_hint),
    );
    let hint_rect = LayoutRectPx::new(rect.x, cy, rect.width, row_h_ctl);
    let hint_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.param_hint"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: hint_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, hint_text.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&hint_wid), hint_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&hint_wid), hint_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &hint_wid,
        FittedTextSpec {
            z_layer: ZLayer::FloatingOverlay,
            rect: hint_rect,
            pad_x: 4,
            text: &hint_text,
            style,
            color: builder.palette.accent,
            scroll_container: None,
        },
    );
    cy += row_step;

    let list_h = u32::try_from((rect.bottom() - cy).max(0)).unwrap_or(0);
    if list_h < scaled_u32(24.0, scale, 20, 48) {
        return;
    }
    let list_rect = LayoutRectPx::new(rect.x, cy, rect.width, list_h);
    let row_h = scaled_u32(20.0, scale, 18, 40);
    let sb_reserve = scaled_u32(14.0, scale, 12, 28);
    let content_h = (filtered.len().max(1) as u32) * row_h + 4;
    let scroll_id = String::from("scroll.mv_formula.funcs");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), list_rect, [list_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.func_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: list_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Formula Functions", FocusScope::Workspace, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&list_wid), list_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&list_wid), list_rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(list_rect.x + 2, list_rect.y + 2, list_rect.width.saturating_sub(sb_reserve), list_rect.height.saturating_sub(4));
    builder.push_clip(clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    for (idx, spec) in filtered.iter().enumerate() {
        let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
        if ry + i32::try_from(row_h).unwrap_or(20) <= clip.y || ry >= clip.bottom() {
            continue;
        }
        let item_rect = LayoutRectPx::new(clip.x, ry, clip.width, row_h - 2);
        let item_id = format!("mv_formula.func.{}", spec.name);
        let selected = state.selected_function_name.as_deref() == Some(spec.name);
        let st = widget_state(builder, &item_id, ControlEnablement::enabled());
        let lbl = format!("{} — {}", spec.signature, spec.summary);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(item_id.as_str()),
            parent: Some(list_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect_px: item_rect,
            a11y: selectable_meta(WidgetRole::Button, lbl.clone(), FocusScope::Workspace, &item_id, selected),
            state: st.clone(),
            action: custom_action(format!("insert_fn:{}", spec.name)),
        });
        let bg = if selected {
            builder.palette.accent
        } else if st.hovered {
            builder.palette.header
        } else {
            builder.palette.surface
        };
        let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
        builder.paint_fill(ZLayer::FloatingOverlay, Some(&wid), item_rect, bg);
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::FloatingOverlay,
                rect: item_rect,
                pad_x: 4,
                text: &lbl,
                style,
                color: fg,
                scroll_container: Some(&scroll_id),
            },
        );
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::FloatingOverlay, &scroll_meta, Some(list_wid));
}

/// Emit the `Variables...` column-role, level-recoding, and categorical-ordering drawer (`R10.8` `:695`).
pub fn build_variables_drawer(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    mv: &MultipleVariablesTable,
    drawer_rect: LayoutRectPx,
    state: &MvFormulaViewState,
) {
    let scale = builder.scale();
    if drawer_rect.width < scaled_u32(180.0, scale, 150, 360) || drawer_rect.height < scaled_u32(220.0, scale, 180, 440) || mv.variables.is_empty() {
        return;
    }
    let drawer_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.vars_drawer"),
        parent: None,
        z_layer: ZLayer::FloatingOverlay,
        rect_px: drawer_rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Variables & Level Recoding", FocusScope::Workspace, "mv_formula.vars_drawer"),
        state: WidgetVisualState::default(),
        action: custom_action("panel_focus"),
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&drawer_wid), drawer_rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&drawer_wid), drawer_rect, builder.palette.accent, 1);

    let inner = drawer_rect.inset(scaled_u32(6.0, scale, 4, 12));
    let loc = builder.tree.locale.clone();
    let size_px = (11.0 * scale).clamp(8.5, 24.0);
    let lbl_style = UiTextStyle::ui_bold(size_px);
    let text_style = UiTextStyle::ui_sans(size_px);
    let hdr_h = scaled_u32(20.0, scale, 18, 40);
    let lbl_h = scaled_u32(18.0, scale, 16, 36);
    let row_h = scaled_u32(22.0, scale, 18, 44);
    let gap_h = scaled_u32(4.0, scale, 3, 8);
    let close_w = scaled_u32(22.0, scale, 18, 44);
    let step_24 = i32::try_from(hdr_h + gap_h).unwrap_or(24);
    let step_18 = i32::try_from(lbl_h).unwrap_or(18);
    let step_26 = i32::try_from(row_h + gap_h).unwrap_or(26);
    let mut cy = inner.y;

    emit_overlay_label(
        builder,
        &drawer_wid,
        "mv_formula.vars.title",
        LayoutRectPx::new(inner.x, cy, inner.width.saturating_sub(close_w + gap_h), hdr_h),
        mv_tr(&loc, "mv.vars.title", "Variables & Level Recoding"),
        &lbl_style,
        builder.palette.text,
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.close",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.right() - i32::try_from(close_w).unwrap_or(22), cy, close_w, hdr_h),
            label: "X",
            keyboard_path: "Escape",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("vars_close"),
        },
    );
    cy += step_24;

    let vd = &builder.app.grid.variables_dialog;
    let sel_col = vd.selected_column.filter(|&c| c < mv.variables.len()).unwrap_or(0);
    let Some(var) = mv.variables.get(sel_col) else {
        return;
    };

    // Column pills (up to what fits, centered around sel_col if many)
    let col_count = mv.variables.len();
    let max_pills = (inner.width / scaled_u32(54.0, scale, 46, 108)).clamp(1, col_count as u32) as usize;
    let start_idx = if col_count <= max_pills { 0 } else { sel_col.saturating_sub(max_pills / 2).min(col_count - max_pills) };
    let shown = (col_count - start_idx).min(max_pills);
    let pill_w = ((inner.width.saturating_sub((shown as u32).saturating_sub(1) * 3)) / (shown as u32).max(1)).max(scaled_u32(36.0, scale, 32, 72));
    for (offset, c_idx) in (start_idx..start_idx + shown).enumerate() {
        if let Some(col_var) = mv.variables.get(c_idx) {
            let px = inner.x + i32::try_from((offset as u32) * (pill_w + 3)).unwrap_or(0);
            emit_tab(
                builder,
                ButtonWidgetSpec {
                    id: &format!("mv_formula.vars.col.{c_idx}"),
                    parent: Some(drawer_wid.clone()),
                    z_layer: ZLayer::FloatingOverlay,
                    rect: LayoutRectPx::new(px, cy, pill_w, hdr_h),
                    label: &col_var.title,
                    keyboard_path: &format!("mv_formula.vars.col.{c_idx}"),
                    focus_scope: FocusScope::Workspace,
                    tab_order: None,
                    primary: false,
                    selected: c_idx == sel_col,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("vars_select_col:{c_idx}")),
                },
            );
        }
    }
    cy += step_24;

    // Role + Lock strip inside drawer
    let is_calc = matches!(var.source, VariableSource::Calculated { .. });
    let role_w = ((inner.width.saturating_sub(12)) / 4).max(scaled_u32(38.0, scale, 32, 76));
    let role_items: [(&str, &str, bool); 3] = [
        ("continuous", "Cont", var.role == VariableRole::Continuous && !var.role_inferred),
        ("categorical", "Cat", var.role == VariableRole::Categorical && !var.role_inferred),
        ("label", "Label", var.role == VariableRole::Label && !var.role_inferred),
    ];
    for (idx, (key, lbl, selected)) in role_items.into_iter().enumerate() {
        let en = if ds.frozen {
            ControlEnablement::disabled("Data sheet is frozen (read-only).")
        } else if is_calc {
            ControlEnablement::disabled("Calculated variable role is formula-locked.")
        } else {
            ControlEnablement::enabled()
        };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("mv_formula.vars.role.{key}"),
                parent: Some(drawer_wid.clone()),
                z_layer: ZLayer::FloatingOverlay,
                rect: LayoutRectPx::new(inner.x + i32::try_from((idx as u32) * (role_w + 4)).unwrap_or(0), cy, role_w, hdr_h),
                label: lbl,
                keyboard_path: &format!("mv_formula.vars.role.{key}"),
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: en,
                action: custom_action(format!("set_role:{sel_col}:{key}")),
            },
        );
    }
    let locked = !var.role_inferred || is_calc;
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.lock_toggle",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(
                inner.x + i32::try_from(3 * (role_w + 4)).unwrap_or(120),
                cy,
                inner.width.saturating_sub(3 * (role_w + 4)),
                hdr_h,
            ),
            label: if locked { "Locked" } else { "Auto" },
            keyboard_path: "mv_formula.vars.lock_toggle",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: locked,
            shortcut: None,
            enablement: if ds.frozen {
                ControlEnablement::disabled("Data sheet is frozen (read-only).")
            } else if is_calc {
                ControlEnablement::disabled("Calculated variable role is formula-locked.")
            } else {
                ControlEnablement::enabled()
            },
            action: custom_action(format!("toggle_role_lock:{sel_col}")),
        },
    );
    cy += step_24;

    // Level ordering buttons
    emit_overlay_label(
        builder,
        &drawer_wid,
        "mv_formula.vars.order_lbl",
        LayoutRectPx::new(inner.x, cy, inner.width, lbl_h),
        mv_tr(&loc, "mv.vars.order_mode", "Level Ordering"),
        &lbl_style,
        builder.palette.text,
    );
    cy += step_18;
    let ord_spec = if vd.selected_column == Some(sel_col) { &vd.level_order } else { &var.level_order };
    let ord_modes: [(&str, &str, bool); 4] = [
        ("entry", mv_tr(&loc, "mv.vars.order_entry", "Entry"), matches!(ord_spec.order, LevelOrderKind::EntryOrder)),
        ("alpha", mv_tr(&loc, "mv.vars.order_alpha", "A-Z"), matches!(ord_spec.order, LevelOrderKind::Alphabetical)),
        ("freq_desc", mv_tr(&loc, "mv.vars.order_freq_desc", "Freq-"), matches!(ord_spec.order, LevelOrderKind::FrequencyDescending)),
        ("freq_asc", mv_tr(&loc, "mv.vars.order_freq_asc", "Freq+"), matches!(ord_spec.order, LevelOrderKind::FrequencyAscending)),
    ];
    let ord_w = ((inner.width.saturating_sub(9)) / 4).max(scaled_u32(36.0, scale, 32, 72));
    for (idx, (key, lbl, selected)) in ord_modes.into_iter().enumerate() {
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("mv_formula.vars.order.{key}"),
                parent: Some(drawer_wid.clone()),
                z_layer: ZLayer::FloatingOverlay,
                rect: LayoutRectPx::new(inner.x + i32::try_from((idx as u32) * (ord_w + 3)).unwrap_or(0), cy, ord_w, hdr_h),
                label: lbl,
                keyboard_path: &format!("mv_formula.vars.order.{key}"),
                focus_scope: FocusScope::Workspace,
                tab_order: None,
                primary: false,
                selected,
                shortcut: None,
                enablement: if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() },
                action: custom_action(format!("vars_order_kind:{key}")),
            },
        );
    }
    cy += step_24;

    let rev_w = inner.width.saturating_sub(scaled_u32(86.0, scale, 72, 172)).max(scaled_u32(80.0, scale, 68, 160));
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: "mv_formula.vars.order.reversed",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, rev_w, hdr_h),
            label: mv_tr(&loc, "mv.vars.reverse_order", "Reverse Order"),
            keyboard_path: "mv_formula.vars.order.reversed",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            checked: ord_spec.reversed,
            is_radio: false,
            enablement: if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() },
            scroll_container: None,
            action: custom_action("vars_order_reverse"),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.order.apply",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from(rev_w + 4).unwrap_or(84), cy, inner.width.saturating_sub(rev_w + 4), hdr_h),
            label: mv_tr(&loc, "mv.vars.apply_order", "Apply Order"),
            keyboard_path: "mv_formula.vars.order.apply",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() },
            action: custom_action("vars_order_apply"),
        },
    );
    cy += step_24;

    // Distinct levels list + Up/Down/Ref controls + Recode block
    let recode_block_h = scaled_u32(136.0, scale, 110, 272);
    let levels_h = u32::try_from((inner.bottom() - cy).max(0)).unwrap_or(0).saturating_sub(recode_block_h);
    if levels_h >= scaled_u32(44.0, scale, 36, 88) {
        let levels = if vd.selected_column == Some(sel_col) {
            vd.preview_recode(ds).map_or_else(|| var.ordered_levels_with(ord_spec), |(_, lvls)| lvls)
        } else {
            var.ordered_levels()
        };
        let levels_rect = LayoutRectPx::new(inner.x, cy, inner.width, levels_h);
        build_vars_levels_list(builder, &drawer_wid, ds, is_calc, &levels, levels_rect, &text_style, mv_tr(&loc, "mv.vars.set_ref", "Ref"));
        cy += i32::try_from(levels_h + gap_h).unwrap_or(48);
    }

    // Bulk Recoding section
    let recode_en = if ds.frozen {
        ControlEnablement::disabled("Data sheet is frozen (read-only).")
    } else if is_calc {
        ControlEnablement::disabled("Cannot recode levels of a calculated variable.")
    } else {
        ControlEnablement::enabled()
    };
    let half_w = (inner.width.saturating_sub(4)) / 2;
    emit_overlay_label(
        builder,
        &drawer_wid,
        "mv_formula.vars.recode_from_lbl",
        LayoutRectPx::new(inner.x, cy, half_w, lbl_h),
        mv_tr(&loc, "mv.vars.recode_from", "From Level"),
        &text_style,
        builder.palette.muted,
    );
    emit_overlay_label(
        builder,
        &drawer_wid,
        "mv_formula.vars.recode_to_lbl",
        LayoutRectPx::new(inner.x + i32::try_from(half_w + 4).unwrap_or(80), cy, inner.width.saturating_sub(half_w + 4), lbl_h),
        mv_tr(&loc, "mv.vars.recode_to", "To Level"),
        &text_style,
        builder.palette.muted,
    );
    cy += step_18;

    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_RECODE_FROM,
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, half_w, row_h),
            label: mv_tr(&loc, "mv.vars.recode_from", "From Level"),
            fallback_text: &state.recode_from,
            keyboard_path: WID_RECODE_FROM,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(320),
            multiline: false,
            enablement: recode_en.clone(),
        },
    );
    emit_text_field(
        builder,
        TextFieldSpec {
            id: WID_RECODE_TO,
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from(half_w + 4).unwrap_or(80), cy, inner.width.saturating_sub(half_w + 4), row_h),
            label: mv_tr(&loc, "mv.vars.recode_to", "To Level"),
            fallback_text: &state.recode_to,
            keyboard_path: WID_RECODE_TO,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(321),
            multiline: false,
            enablement: recode_en.clone(),
        },
    );
    cy += step_26;

    let b3_w = ((inner.width.saturating_sub(8)) / 3).max(scaled_u32(46.0, scale, 40, 96));
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.recode_stage",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x, cy, b3_w, row_h),
            label: mv_tr(&loc, "mv.vars.stage_recode", "Stage"),
            keyboard_path: "mv_formula.vars.recode_stage",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: recode_en.clone(),
            action: custom_action("vars_recode_stage"),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.recode_clear",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from(b3_w + 4).unwrap_or(50), cy, b3_w, row_h),
            label: "Clear",
            keyboard_path: "mv_formula.vars.recode_clear",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: false,
            shortcut: None,
            enablement: ControlEnablement::enabled(),
            action: custom_action("vars_recode_clear"),
        },
    );
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "mv_formula.vars.recode_apply",
            parent: Some(drawer_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect: LayoutRectPx::new(inner.x + i32::try_from((b3_w + 4) * 2).unwrap_or(100), cy, inner.width.saturating_sub((b3_w + 4) * 2), row_h),
            label: mv_tr(&loc, "mv.vars.apply_recode", "Apply Recode"),
            keyboard_path: "mv_formula.vars.recode_apply",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: true,
            selected: false,
            shortcut: None,
            enablement: recode_en,
            action: custom_action("vars_recode_apply"),
        },
    );
    cy += step_26;

    let preview_msg = if !vd.recode_mappings.is_empty() {
        let changed = vd.preview_recode(ds).map_or(0, |(cnt, _)| cnt);
        format!("Staged {} rule(s): {changed} cell(s) will update", vd.recode_mappings.len())
    } else if let Some(notice) = &state.status_notice {
        notice.clone()
    } else {
        String::from("Click a level or type From -> To to recode")
    };
    emit_overlay_label(
        builder,
        &drawer_wid,
        "mv_formula.vars.recode_preview",
        LayoutRectPx::new(inner.x, cy, inner.width, hdr_h),
        &preview_msg,
        &text_style,
        builder.palette.muted,
    );
}

#[allow(clippy::too_many_arguments)]
fn build_vars_levels_list(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    ds: &DataSheet,
    is_calc: bool,
    levels: &[curvebook_doc::CategoricalLevelSummary],
    rect: LayoutRectPx,
    style: &UiTextStyle,
    ref_label: &str,
) {
    let scale = builder.scale();
    let row_h = scaled_u32(22.0, scale, 18, 44);
    let sb_reserve = scaled_u32(14.0, scale, 12, 28);
    let content_h = (levels.len().max(1) as u32) * row_h + 4;
    let scroll_id = String::from("scroll.mv_formula.vars_levels");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let list_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("mv_formula.vars.levels_list"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Categorical Levels", FocusScope::Workspace, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&list_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&list_wid), rect, builder.palette.border, 1);

    let clip = LayoutRectPx::new(rect.x + 2, rect.y + 2, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(4));
    builder.push_clip(clip);
    if levels.is_empty() {
        paint_text_fitted(
            builder,
            &list_wid,
            FittedTextSpec {
                z_layer: ZLayer::FloatingOverlay,
                rect: LayoutRectPx::new(clip.x, clip.y, clip.width, row_h.saturating_sub(4)),
                pad_x: 4,
                text: "No categorical levels in column.",
                style,
                color: builder.palette.muted,
                scroll_container: Some(&scroll_id),
            },
        );
    } else {
        let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
        let ord_en = if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() };
        let up_w = scaled_u32(22.0, scale, 18, 44);
        let dn_w = scaled_u32(22.0, scale, 18, 44);
        let ref_w = scaled_u32(34.0, scale, 28, 68);
        let lbl_w = clip.width.saturating_sub(up_w + dn_w + ref_w + 6).max(scaled_u32(40.0, scale, 32, 80));
        for (idx, lvl) in levels.iter().enumerate() {
            let ry = clip.y + i32::try_from(idx as u32 * row_h).unwrap_or(0) - y_scroll;
            if ry + i32::try_from(row_h).unwrap_or(22) <= clip.y || ry >= clip.bottom() {
                continue;
            }
            let row_h_inner = row_h - 2;
            let lbl_rect = LayoutRectPx::new(clip.x, ry, lbl_w, row_h_inner);
            let lbl_text = if lvl.excluded_count > 0 {
                format!("{} ({}, -{})", lvl.level, lvl.included_count, lvl.excluded_count)
            } else {
                format!("{} ({})", lvl.level, lvl.included_count)
            };
            let lvl_id = format!("mv_formula.vars.level.{idx}");
            let lvl_en = if is_calc { ControlEnablement::disabled("Calculated column cannot be recoded.") } else { ControlEnablement::enabled() };
            let st = widget_state(builder, &lvl_id, lvl_en.clone());
            let lvl_wid = builder.push_node(NodeSpec {
                id: WidgetId::from(lvl_id.as_str()),
                parent: Some(list_wid.clone()),
                z_layer: ZLayer::FloatingOverlay,
                rect_px: lbl_rect,
                a11y: labeled_meta(WidgetRole::Button, lbl_text.clone(), FocusScope::Workspace, &lvl_id),
                state: st.clone(),
                action: if lvl_en.enabled { custom_action(format!("vars_pick_level:{idx}")) } else { WidgetAction::NoOp },
            });
            builder.paint_fill(
                ZLayer::FloatingOverlay,
                Some(&lvl_wid),
                lbl_rect,
                if st.hovered { builder.palette.header } else { builder.palette.surface },
            );
            paint_text_fitted(
                builder,
                &lvl_wid,
                FittedTextSpec {
                    z_layer: ZLayer::FloatingOverlay,
                    rect: lbl_rect,
                    pad_x: 4,
                    text: &lbl_text,
                    style,
                    color: builder.palette.text,
                    scroll_container: Some(&scroll_id),
                },
            );

            let up_x = lbl_rect.right() + 2;
            let up_en = if !ord_en.enabled {
                ord_en.clone()
            } else if idx == 0 {
                ControlEnablement::disabled("Already first level")
            } else {
                ControlEnablement::enabled()
            };
            emit_scrolled_mini_btn(
                builder,
                &list_wid,
                &scroll_id,
                &format!("mv_formula.vars.level_up.{idx}"),
                LayoutRectPx::new(up_x, ry, up_w, row_h_inner),
                "^",
                up_en,
                format!("vars_level_up:{idx}"),
                style,
            );

            let dn_x = up_x + i32::try_from(up_w + 2).unwrap_or(24);
            let dn_en = if !ord_en.enabled {
                ord_en.clone()
            } else if idx + 1 >= levels.len() {
                ControlEnablement::disabled("Already last level")
            } else {
                ControlEnablement::enabled()
            };
            emit_scrolled_mini_btn(
                builder,
                &list_wid,
                &scroll_id,
                &format!("mv_formula.vars.level_dn.{idx}"),
                LayoutRectPx::new(dn_x, ry, dn_w, row_h_inner),
                "v",
                dn_en,
                format!("vars_level_dn:{idx}"),
                style,
            );

            let ref_x = dn_x + i32::try_from(dn_w + 2).unwrap_or(24);
            emit_scrolled_mini_btn(
                builder,
                &list_wid,
                &scroll_id,
                &format!("mv_formula.vars.level_ref.{idx}"),
                LayoutRectPx::new(ref_x, ry, ref_w, row_h_inner),
                ref_label,
                ord_en.clone(),
                format!("vars_level_ref:{idx}"),
                style,
            );
        }
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::FloatingOverlay, &scroll_meta, Some(list_wid));
}

#[allow(clippy::too_many_arguments)]
fn emit_scrolled_mini_btn(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    scroll_id: &str,
    id: &str,
    rect: LayoutRectPx,
    label: &str,
    en: ControlEnablement,
    payload: String,
    style: &UiTextStyle,
) {
    let st = widget_state(builder, id, en.clone());
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Button, label, FocusScope::Workspace, id),
        state: st.clone(),
        action: if en.enabled { custom_action(payload) } else { WidgetAction::NoOp },
    });
    let bg = if !en.enabled {
        builder.palette.panel
    } else if st.hovered {
        builder.palette.header
    } else {
        builder.palette.surface
    };
    let fg = if en.enabled { builder.palette.text } else { builder.palette.muted };
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&wid), rect, bg);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&wid), rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec { z_layer: ZLayer::FloatingOverlay, rect, pad_x: 3, text: label, style, color: fg, scroll_container: Some(scroll_id) },
    );
}

/// Emit the per-column `fx` button inside a `MultipleVariables` data column header.
pub fn emit_mv_column_fx_button(
    builder: &mut LayoutBuilder<'_>,
    ds: &DataSheet,
    col: usize,
    lbl_rect: LayoutRectPx,
    h_rect: LayoutRectPx,
    fx_w: u32,
) {
    if fx_w == 0 {
        return;
    }
    let id = format!("formula_bar.open_col.{col}");
    if builder.tree.nodes.iter().any(|n| n.id.as_str() == id) {
        return;
    }
    let fx_rect = LayoutRectPx::new(lbl_rect.right(), h_rect.y, fx_w, h_rect.height);
    let vis_fx = fx_rect.intersect(builder.active_clip_rect());
    if vis_fx.width < 4 || vis_fx.height < 4 {
        return;
    }
    let en = if ds.frozen { ControlEnablement::disabled("Data sheet is frozen (read-only).") } else { ControlEnablement::enabled() };
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &id,
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: fx_rect,
            label: "fx",
            keyboard_path: "F2",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: builder.app.grid.formula_bar.active && builder.app.grid.formula_bar.target_column == Some(col),
            shortcut: None,
            enablement: en,
            action: custom_action(format!("open_col:{col}")),
        },
    );
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{DataSheet, MultipleVariablesTable, ProjectDocument, SheetId, TableArchetype};
    use curvebook_engine::Session;

    use crate::UiApp;
    use crate::human_driver::HumanPathDriver;
    use crate::interaction::ViewportMetrics;

    fn build_mv_cols_driver(cols: usize, viewport: ViewportMetrics) -> Result<HumanPathDriver, String> {
        let mv = MultipleVariablesTable::new(cols, 6).map_err(|e| e.to_string())?;
        let ds = DataSheet::new("mv-strip", "Strip Test", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
        let mut doc = ProjectDocument::new("MV Strip Test").map_err(|e| e.to_string())?;
        doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
        doc.select_in_navigator(&SheetId::from("mv-strip")).map_err(|e| e.to_string())?;
        Ok(HumanPathDriver::new(UiApp::from_session(Session::from_document(doc)), viewport))
    }

    #[test]
    fn mv_role_strip_handles_five_plus_columns_at_1280x800_and_scrolls_when_overflowing() -> Result<(), String> {
        // 5 columns at 1280x800: role_badge.4 must be visibly drawn (or reachable via role_strip.next) with clean parity.
        let mut d5 = build_mv_cols_driver(5, ViewportMetrics::new(1280.0, 800.0, 1.0))?;
        assert!(d5.verify_parity().is_clean(), "5-col MV parity violations: {:?}", d5.verify_parity().violations);
        if d5.find_widget("mv_formula.role_badge.4").is_none_or(|n| !n.drawn_in_frame) {
            assert!(d5.find_widget("mv_formula.role_strip.next").is_some_and(|n| n.drawn_in_frame && n.state.enablement.enabled));
            let _ = d5.click_widget("mv_formula.role_strip.next")?;
        }
        assert!(d5.find_widget("mv_formula.role_badge.4").is_some_and(|n| n.drawn_in_frame));

        // 8 columns at 1280x800: must emit enabled role_strip.next and scroll smoothly to column 7.
        let mut d8 = build_mv_cols_driver(8, ViewportMetrics::new(1280.0, 800.0, 1.0))?;
        assert!(d8.verify_parity().is_clean(), "8-col MV parity violations: {:?}", d8.verify_parity().violations);
        for _ in 0..8 {
            if d8.find_widget("mv_formula.role_badge.7").is_some_and(|n| n.drawn_in_frame) {
                break;
            }
            let next_node = d8.find_widget("mv_formula.role_strip.next").ok_or_else(|| String::from("missing mv_formula.role_strip.next"))?;
            assert!(next_node.drawn_in_frame && next_node.state.enablement.enabled);
            let _ = d8.click_widget("mv_formula.role_strip.next")?;
        }
        assert!(d8.find_widget("mv_formula.role_badge.7").is_some_and(|n| n.drawn_in_frame));
        assert!(d8.verify_parity().is_clean());
        Ok(())
    }
}
