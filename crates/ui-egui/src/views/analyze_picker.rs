//! `Analyze...` searchable categorized tree and duplicate-analysis disambiguation view (`R10.3`).

use curvebook_doc::SheetId;
use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AnalysisCategory, AnalyzeDuplicateChoice, VariableRoleFilter};
use crate::interaction::UiEventOutcome;

const SEARCH_WIDGET_ID: &str = "analyze.search";
const ENTRIES_SCROLL_ID: &str = "scroll.analyze.entries";

/// User choice in the re-analysis disambiguation prompt (`DuplicateAnalysisPrompt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DuplicateChoice {
    ModifyExisting,
    #[default]
    CreateNew,
}

/// Transient view state owned by the `Analyze...` picker shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnalyzePickerViewState {
    pub search_query: String,
    #[serde(default)]
    pub collapsed_categories: Vec<String>,
    #[serde(default)]
    pub duplicate_choice: DuplicateChoice,
    #[serde(default)]
    pub duplicate_target_id: Option<String>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `analyze_picker` domain.
///
/// # Errors
/// Returns an error if an explicit duplicate-analysis execution command fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut AnalyzePickerViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "clear_search" {
        app.interaction.text_field_buffers.remove(SEARCH_WIDGET_ID);
        app.canvas_dialogs.analyze.search_query.clear();
        state.search_query.clear();
        app.interaction.scroll_offsets.offsets.insert(String::from(ENTRIES_SCROLL_ID), [0.0, 0.0]);
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.clear_search"), ..Default::default() });
    }
    if let Some(slug) = payload.strip_prefix("toggle_category:") {
        sync_search_text(app, state);
        if let Some(pos) = state.collapsed_categories.iter().position(|s| s == slug) {
            state.collapsed_categories.remove(pos);
        } else if !slug.is_empty() {
            state.collapsed_categories.push(slug.to_owned());
        }
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.toggle_category:{slug}"), ..Default::default() });
    }
    if payload == "expand_all" {
        state.collapsed_categories.clear();
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.expand_all"), ..Default::default() });
    }
    if payload == "collapse_all" {
        state.collapsed_categories = AnalysisCategory::ALL.into_iter().map(|c| String::from(category_slug(c))).collect();
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.collapse_all"), ..Default::default() });
    }
    if payload == "dup_choice:modify_existing" {
        state.duplicate_choice = DuplicateChoice::ModifyExisting;
        if state.duplicate_target_id.is_none()
            && let ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } = &app.canvas_dialogs.active_dialog
        {
            state.duplicate_target_id = existing_result_ids.first().map(|id| id.as_str().to_owned());
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_choice:modify_existing"), ..Default::default() });
    }
    if payload == "dup_choice:create_new" {
        state.duplicate_choice = DuplicateChoice::CreateNew;
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_choice:create_new"), ..Default::default() });
    }
    if let Some(target_id) = payload.strip_prefix("dup_target:") {
        state.duplicate_target_id = Some(target_id.to_owned());
        state.duplicate_choice = DuplicateChoice::ModifyExisting;
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.dup_target:{target_id}"), ..Default::default() });
    }
    if payload == "dup_confirm" {
        return execute_duplicate_confirm(app, state);
    }
    if let Some(slug) = payload.strip_prefix("set_mv_role_filter:") {
        let filter = VariableRoleFilter::from_slug(slug).ok_or_else(|| format!("unknown variable role filter `{slug}`"))?;
        app.canvas_dialogs.analyze.role_filter = filter;
        app.interaction.scroll_offsets.offsets.insert(String::from("scroll.analyze.mv_vars"), [0.0, 0.0]);
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.set_mv_role_filter:{slug}"), ..Default::default() });
    }
    if let Some(col_raw) = payload.strip_prefix("toggle_mv_var:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        app.canvas_dialogs.analyze.toggle_variable(col);
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.toggle_mv_var:{col}"), ..Default::default() });
    }
    if let Some(col_raw) = payload.strip_prefix("toggle_pca_pred:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        app.canvas_dialogs.analyze.toggle_pca_predictor(col);
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.toggle_pca_pred:{col}"), ..Default::default() });
    }
    if let Some(col_raw) = payload.strip_prefix("set_pcr_outcome:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        app.canvas_dialogs.analyze.select_pcr_outcome(col)?;
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.set_pcr_outcome:{col}"), ..Default::default() });
    }
    if payload == "outlier_mode:rout" {
        app.canvas_dialogs.analyze.outlier_method = String::from("rout");
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_mode:rout"), ..Default::default() });
    }
    if payload == "outlier_mode:grubbs" {
        app.canvas_dialogs.analyze.outlier_method = String::from("grubbs");
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_mode:grubbs"), ..Default::default() });
    }
    if let Some(delta_raw) = payload.strip_prefix("outlier_param_step:")
        && let Ok(delta) = delta_raw.parse::<f64>()
    {
        let an = &mut app.canvas_dialogs.analyze;
        if an.outlier_method == "grubbs" {
            an.outlier_alpha = (an.outlier_alpha + delta).clamp(0.001, 0.25);
        } else {
            an.outlier_fdr_q_percent = (an.outlier_fdr_q_percent + delta).clamp(0.1, 20.0);
        }
        return Ok(UiEventOutcome { handled: true, action: format!("analyze_picker.outlier_param_step:{delta_raw}"), ..Default::default() });
    }
    if payload == "outlier_toggle_iterative" {
        let an = &mut app.canvas_dialogs.analyze;
        if an.outlier_method == "grubbs" {
            an.outlier_iterative = !an.outlier_iterative;
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.outlier_toggle_iterative"), ..Default::default() });
    }
    Ok(UiEventOutcome::default())
}

fn sync_search_text(app: &mut UiApp, state: &mut AnalyzePickerViewState) {
    if let Some(buf) = app.interaction.text_field_buffers.get(SEARCH_WIDGET_ID) {
        state.search_query = buf.text.clone();
        app.canvas_dialogs.analyze.search_query = buf.text.clone();
    }
}

fn execute_duplicate_confirm(app: &mut UiApp, state: &mut AnalyzePickerViewState) -> Result<UiEventOutcome, String> {
    let existing = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => existing_result_ids.clone(),
        _ => return Err(String::from("DuplicateAnalysisPrompt dialog is not open")),
    };
    let intent = resolve_duplicate_intent(app, state, &existing).ok_or_else(|| String::from("failed to resolve duplicate analysis choice"))?;
    let cmd_id = intent.command_id.clone();
    let outcome = app.execute_command(&cmd_id, intent.params).map_err(|e| e.to_string())?;
    if let Some(new_id) = outcome.payload.get("sheet_id").and_then(serde_json::Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
    }
    app.canvas_dialogs.close_dialog();
    state.duplicate_choice = DuplicateChoice::CreateNew;
    state.duplicate_target_id = None;
    let result_json = serde_json::to_value(&outcome.payload).ok();
    Ok(UiEventOutcome { handled: true, action: String::from("analyze_picker.dup_confirm"), command_id: Some(cmd_id), command_result: result_json })
}

/// Resolve the active [`AnalyzePickerViewState`] choice into an engine [`ToolCommandIntent`].
#[must_use]
pub fn resolve_duplicate_intent(app: &UiApp, state: &AnalyzePickerViewState, existing_result_ids: &[SheetId]) -> Option<ToolCommandIntent> {
    let choice = match state.duplicate_choice {
        DuplicateChoice::ModifyExisting => {
            let target = state
                .duplicate_target_id
                .as_deref()
                .and_then(|tid| existing_result_ids.iter().find(|id| id.as_str() == tid))
                .or_else(|| existing_result_ids.first())?;
            AnalyzeDuplicateChoice::ModifyExisting(target.clone())
        }
        DuplicateChoice::CreateNew => AnalyzeDuplicateChoice::CreateNew,
    };
    app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice)
}

fn category_slug(cat: AnalysisCategory) -> &'static str {
    match cat {
        AnalysisCategory::Transform => "transform",
        AnalysisCategory::Column => "column",
        AnalysisCategory::Grouped => "grouped",
        AnalysisCategory::XyAndRegression => "xy",
        AnalysisCategory::Survival => "survival",
        AnalysisCategory::Contingency => "contingency",
        AnalysisCategory::Multivariate => "multivariate",
    }
}
