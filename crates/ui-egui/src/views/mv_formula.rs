//! Multiple-variables formula bar helpers and categorical level reordering (`R10.8`).

use curvebook_doc::SheetId;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;

const WID_FORMULA_INPUT: &str = "formula_bar.input";
const WID_NAME_INPUT: &str = "mv_formula.name.input";

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
    pub selected_level_name: Option<String>,
    #[serde(default)]
    pub status_notice: Option<String>,
    #[serde(default)]
    pub recode_from: String,
    #[serde(default)]
    pub recode_to: String,
    #[serde(default)]
    pub revision: u64,
}

const FORMULA_FUNCTION_SIGNATURES: &[(&str, &str)] = &[
    ("abs", "abs(x)"),
    ("sqrt", "sqrt(x)"),
    ("cbrt", "cbrt(x)"),
    ("sqr", "sqr(x)"),
    ("ln", "ln(x)"),
    ("log10", "log10(x)"),
    ("log2", "log2(x)"),
    ("log", "log(x, base)"),
    ("exp", "exp(x)"),
    ("exp10", "exp10(x)"),
    ("pow", "pow(x, y)"),
    ("round", "round(x, digits)"),
    ("floor", "floor(x)"),
    ("ceil", "ceil(x)"),
    ("clamp", "clamp(x, lo, hi)"),
    ("logit", "logit(p)"),
    ("probit", "probit(p)"),
    ("norm_ppf", "norm_ppf(p)"),
    ("arcsin_sqrt", "arcsin_sqrt(p)"),
    ("if", "if(cond, t, f)"),
    ("and", "and(a, b)"),
    ("or", "or(a, b)"),
    ("not", "not(cond)"),
    ("is_defined", "is_defined(x)"),
    ("isnan", "isnan(x)"),
    ("coalesce", "coalesce(a, b)"),
    ("count", "count(var)"),
    ("sum", "sum(var)"),
    ("mean", "mean(var)"),
    ("sd", "sd(var)"),
    ("sem", "sem(var)"),
    ("median", "median(var)"),
    ("iqr", "iqr(var)"),
    ("geo_mean", "geo_mean(var)"),
    ("percentile", "percentile(var, p)"),
    ("countif", "countif(cond)"),
    ("sumif", "sumif(var, cond)"),
    ("meanif", "meanif(var, cond)"),
    ("zscore", "zscore(var)"),
    ("row_number", "row_number()"),
    ("rank", "rank(var)"),
    ("dense_rank", "dense_rank(var)"),
    ("lag", "lag(var, k)"),
    ("lead", "lead(var, k)"),
    ("diff", "diff(var, k)"),
    ("cumsum", "cumsum(var)"),
    ("cummean", "cummean(var)"),
    ("moving_avg", "moving_avg(var, w)"),
];

/// Return the canonical signature string displayed for a Multiple Variables formula function (`#14`).
#[must_use]
pub fn formula_function_signature(name: &str) -> Option<&'static str> {
    FORMULA_FUNCTION_SIGNATURES.iter().find(|(fn_name, _)| *fn_name == name).map(|(_, sig)| *sig)
}

fn active_any_sheet_id(app: &UiApp) -> Option<SheetId> {
    let doc = app.document();
    app.grid.formula_bar.target_sheet_id.clone().or_else(|| doc.active_sheet_id.clone()).filter(|id| doc.data_sheet(id).is_some())
}

fn clear_formula_text_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove(WID_FORMULA_INPUT);
    app.interaction.text_field_buffers.remove(WID_NAME_INPUT);
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

fn commit_validation_error(state: &mut MvFormulaViewState, msg: String) -> Result<UiEventOutcome, String> {
    state.validation_feedback = Some(msg.clone());
    if state.right_panel_open {
        Ok(UiEventOutcome {
            handled: true,
            action: String::from("mv_formula.validation_error"),
            command_result: Some(json!({ "committed": false, "validation_error": msg })),
            ..Default::default()
        })
    } else {
        Err(msg)
    }
}

pub(crate) fn commit_from_textbox_enter(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    let mut state = std::mem::take(&mut app.view_shards.mv_formula);
    let res = commit_formula_from_action(app, &mut state);
    app.view_shards.mv_formula = state;
    res
}

fn commit_formula_from_action(app: &mut UiApp, state: &mut MvFormulaViewState) -> Result<UiEventOutcome, String> {
    sync_buffers_into_formula_bar(app);
    let fb = &app.grid.formula_bar;
    if !fb.active {
        return commit_validation_error(state, String::from("Formula bar is not active."));
    }
    if let Some(err) = fb.parse_error.clone() {
        return commit_validation_error(state, err);
    }
    if !fb.is_valid {
        return commit_validation_error(state, String::from("Calculated variable formula cannot be empty or invalid."));
    }
    let Some(sheet_id) = active_any_sheet_id(app) else {
        return commit_validation_error(state, String::from("No active data sheet for formula bar."));
    };
    let intent = match app.grid.formula_bar.commit_intent(&sheet_id) {
        Ok(intent) => intent,
        Err(err) => return commit_validation_error(state, err),
    };
    let outcome = match app.execute_command(&intent.command_id, intent.params) {
        Ok(out) => out,
        Err(err) => return commit_validation_error(state, err.to_string()),
    };
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LevelMoveOp {
    Top,
    Up,
    Down,
    End,
}

/// Resolve the selected index in `levels` matching `selected`, defaulting to `0` when non-empty.
pub(crate) fn resolve_selected_level_idx(levels: &[String], selected: Option<&str>) -> Option<usize> {
    if levels.is_empty() {
        return None;
    }
    selected.and_then(|name| levels.iter().position(|lvl| lvl == name)).or(Some(0))
}

/// Stepwise reorder the selected level in `levels`, updating `selected` to follow the moved item.
pub(crate) fn apply_explicit_level_move(levels: &mut Vec<String>, selected: &mut Option<String>, op: LevelMoveOp) -> bool {
    let Some(idx) = resolve_selected_level_idx(levels, selected.as_deref()) else {
        return false;
    };
    let moved_name = levels[idx].clone();
    let changed = match op {
        LevelMoveOp::Top if idx > 0 => {
            let item = levels.remove(idx);
            levels.insert(0, item);
            true
        }
        LevelMoveOp::Up if idx > 0 => {
            levels.swap(idx, idx - 1);
            true
        }
        LevelMoveOp::Down if idx + 1 < levels.len() => {
            levels.swap(idx, idx + 1);
            true
        }
        LevelMoveOp::End if idx + 1 < levels.len() => {
            let item = levels.remove(idx);
            levels.push(item);
            true
        }
        _ => false,
    };
    *selected = Some(moved_name);
    changed
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `mv_formula` domain.
pub fn handle_custom_action(_app: &mut UiApp, state: &mut MvFormulaViewState, _payload: &str) -> Result<UiEventOutcome, String> {
    state.revision = state.revision.wrapping_add(1);
    Ok(UiEventOutcome::default())
}
