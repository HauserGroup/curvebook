//! Multiple-variables role badges, `Variables...` recoding/ordering drawer, and right-side
//! calculated-variable editor (`R10.8`).

use curvebook_doc::{DataSheet, LevelOrderKind, LevelOrderSpec, SheetId, TableArchetype, VariableRole, VariableSource};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::{TextFieldEditState, UiEventOutcome};

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

#[derive(Debug, Clone, Copy)]
struct FormulaFnSpec {
    name: &'static str,
    insert_text: &'static str,
}

const FORMULA_FUNCTIONS: &[FormulaFnSpec] = &[
    FormulaFnSpec { name: "abs", insert_text: "abs(" },
    FormulaFnSpec { name: "sqrt", insert_text: "sqrt(" },
    FormulaFnSpec { name: "cbrt", insert_text: "cbrt(" },
    FormulaFnSpec { name: "sqr", insert_text: "sqr(" },
    FormulaFnSpec { name: "ln", insert_text: "ln(" },
    FormulaFnSpec { name: "log10", insert_text: "log10(" },
    FormulaFnSpec { name: "log2", insert_text: "log2(" },
    FormulaFnSpec { name: "log", insert_text: "log(" },
    FormulaFnSpec { name: "exp", insert_text: "exp(" },
    FormulaFnSpec { name: "exp10", insert_text: "exp10(" },
    FormulaFnSpec { name: "pow", insert_text: "pow(" },
    FormulaFnSpec { name: "round", insert_text: "round(" },
    FormulaFnSpec { name: "floor", insert_text: "floor(" },
    FormulaFnSpec { name: "ceil", insert_text: "ceil(" },
    FormulaFnSpec { name: "clamp", insert_text: "clamp(" },
    FormulaFnSpec { name: "logit", insert_text: "logit(" },
    FormulaFnSpec { name: "probit", insert_text: "probit(" },
    FormulaFnSpec { name: "norm_ppf", insert_text: "norm_ppf(" },
    FormulaFnSpec { name: "arcsin_sqrt", insert_text: "arcsin_sqrt(" },
    FormulaFnSpec { name: "if", insert_text: "if(" },
    FormulaFnSpec { name: "and", insert_text: "and(" },
    FormulaFnSpec { name: "or", insert_text: "or(" },
    FormulaFnSpec { name: "not", insert_text: "not(" },
    FormulaFnSpec { name: "is_defined", insert_text: "is_defined(" },
    FormulaFnSpec { name: "isnan", insert_text: "isnan(" },
    FormulaFnSpec { name: "coalesce", insert_text: "coalesce(" },
    FormulaFnSpec { name: "count", insert_text: "count(" },
    FormulaFnSpec { name: "sum", insert_text: "sum(" },
    FormulaFnSpec { name: "mean", insert_text: "mean(" },
    FormulaFnSpec { name: "sd", insert_text: "sd(" },
    FormulaFnSpec { name: "sem", insert_text: "sem(" },
    FormulaFnSpec { name: "median", insert_text: "median(" },
    FormulaFnSpec { name: "iqr", insert_text: "iqr(" },
    FormulaFnSpec { name: "geo_mean", insert_text: "geo_mean(" },
    FormulaFnSpec { name: "percentile", insert_text: "percentile(" },
    FormulaFnSpec { name: "countif", insert_text: "countif(" },
    FormulaFnSpec { name: "sumif", insert_text: "sumif(" },
    FormulaFnSpec { name: "meanif", insert_text: "meanif(" },
    FormulaFnSpec { name: "zscore", insert_text: "zscore(" },
    FormulaFnSpec { name: "row_number", insert_text: "row_number()" },
    FormulaFnSpec { name: "rank", insert_text: "rank(" },
    FormulaFnSpec { name: "dense_rank", insert_text: "dense_rank(" },
    FormulaFnSpec { name: "lag", insert_text: "lag(" },
    FormulaFnSpec { name: "lead", insert_text: "lead(" },
    FormulaFnSpec { name: "diff", insert_text: "diff(" },
    FormulaFnSpec { name: "cumsum", insert_text: "cumsum(" },
    FormulaFnSpec { name: "cummean", insert_text: "cummean(" },
    FormulaFnSpec { name: "moving_avg", insert_text: "moving_avg(" },
];

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
    state.validation_feedback = None;
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
        LevelMoveOp::Top => {
            if idx == 0 {
                false
            } else {
                let item = levels.remove(idx);
                levels.insert(0, item);
                true
            }
        }
        LevelMoveOp::Up => {
            if idx == 0 {
                false
            } else {
                levels.swap(idx, idx - 1);
                true
            }
        }
        LevelMoveOp::Down => {
            if idx + 1 >= levels.len() {
                false
            } else {
                levels.swap(idx, idx + 1);
                true
            }
        }
        LevelMoveOp::End => {
            if idx + 1 >= levels.len() {
                false
            } else {
                let item = levels.remove(idx);
                levels.push(item);
                true
            }
        }
    };
    *selected = Some(moved_name);
    changed
}

fn move_selected_level_in_drawer(app: &mut UiApp, state: &mut MvFormulaViewState, op: LevelMoveOp) -> Result<UiEventOutcome, String> {
    let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
    let mut levels: Vec<String> = app.grid.variables_dialog.ordered_levels.iter().map(|l| l.level.clone()).collect();
    let len = levels.len();
    if apply_explicit_level_move(&mut levels, &mut state.selected_level_name, op) {
        let new_idx = resolve_selected_level_idx(&levels, state.selected_level_name.as_deref());
        app.grid.variables_dialog.set_level_order(LevelOrderSpec { order: LevelOrderKind::Explicit(levels), reversed: false }, Some(&ds));
        let out = apply_categorical_order(app, state)?;
        if len > 1 {
            match (app.focus_widget_id.as_deref(), new_idx) {
                (Some("mv_formula.vars.move_top"), Some(0)) => app.focus_widget_id = Some(String::from("mv_formula.vars.move_end")),
                (Some("mv_formula.vars.move_up"), Some(0)) => app.focus_widget_id = Some(String::from("mv_formula.vars.move_down")),
                (Some("mv_formula.vars.move_down"), Some(i)) if i + 1 >= len => {
                    app.focus_widget_id = Some(String::from("mv_formula.vars.move_up"));
                }
                (Some("mv_formula.vars.move_end"), Some(i)) if i + 1 >= len => {
                    app.focus_widget_id = Some(String::from("mv_formula.vars.move_top"));
                }
                _ => {}
            }
        }
        return Ok(out);
    }
    Ok(UiEventOutcome { handled: true, action: String::from("mv_formula.vars_level_move_noop"), ..Default::default() })
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
        state.selected_level_name = None;
        return Ok(Some(UiEventOutcome { handled: true, action: format!("mv_formula.vars_select_col:{col}"), ..Default::default() }));
    }
    if let Some(kind_str) = sub.strip_prefix("vars_order_kind:") {
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let kind = match kind_str {
            "entry" => LevelOrderKind::EntryOrder,
            "alpha" => LevelOrderKind::Alphabetical,
            "freq_desc" => LevelOrderKind::FrequencyDescending,
            "freq_asc" => LevelOrderKind::FrequencyAscending,
            "explicit" => {
                let levels: Vec<String> = app.grid.variables_dialog.ordered_levels.iter().map(|l| l.level.clone()).collect();
                LevelOrderKind::Explicit(levels)
            }
            _ => return Ok(None),
        };
        let reversed = if matches!(kind, LevelOrderKind::Explicit(_)) { false } else { app.grid.variables_dialog.level_order.reversed };
        let spec = LevelOrderSpec { order: kind, reversed };
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
            state.selected_level_name = Some(level.clone());
            state.recode_from.clone_from(&level);
            sync_text_buffer(app, WID_RECODE_FROM, &level);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: format!("mv_formula.vars_pick_level:{idx}"), ..Default::default() }));
    }
    if sub == "vars_move_top" {
        return move_selected_level_in_drawer(app, state, LevelMoveOp::Top).map(Some);
    }
    if sub == "vars_move_up" {
        return move_selected_level_in_drawer(app, state, LevelMoveOp::Up).map(Some);
    }
    if sub == "vars_move_down" {
        return move_selected_level_in_drawer(app, state, LevelMoveOp::Down).map(Some);
    }
    if sub == "vars_move_end" {
        return move_selected_level_in_drawer(app, state, LevelMoveOp::End).map(Some);
    }
    if let Some(raw) = sub.strip_prefix("vars_level_up:")
        && let Ok(idx) = raw.parse::<usize>()
    {
        let (_, ds) = ensure_variables_dialog_synced(app).ok_or_else(|| String::from("No active MultipleVariables sheet."))?;
        let mut levels: Vec<String> = app.grid.variables_dialog.ordered_levels.iter().map(|l| l.level.clone()).collect();
        if idx > 0 && idx < levels.len() {
            state.selected_level_name = Some(levels[idx].clone());
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
            state.selected_level_name = Some(levels[idx].clone());
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
            state.validation_feedback = None;
            return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.insert_col:{col}"), ..Default::default() });
        }
        return Ok(UiEventOutcome { handled: false, action: String::from("mv_formula.insert_col_missing"), ..Default::default() });
    }
    if let Some(name) = payload.strip_prefix("insert_info:") {
        let token = format!("Info(\"{name}\")");
        insert_snippet_into_formula(app, &token);
        state.validation_feedback = None;
        return Ok(UiEventOutcome { handled: true, action: format!("mv_formula.insert_info:{name}"), ..Default::default() });
    }
    if let Some(fn_name) = payload.strip_prefix("insert_fn:") {
        if let Some(spec) = FORMULA_FUNCTIONS.iter().find(|f| f.name == fn_name) {
            state.selected_function_name = Some(spec.name.to_owned());
            insert_snippet_into_formula(app, spec.insert_text);
            state.validation_feedback = None;
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
