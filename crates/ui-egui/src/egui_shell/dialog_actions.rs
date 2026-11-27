//! Desktop `egui_shell` modal button execution and dialog widget click/toggle handlers (`F6`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DiagnosticPlotKind, ErrorCode, GraphKind, MixedModelDfMethod, MultiplicityMethod, MultivariateSpec, OneWayMethod,
    ProjectDocument, ResidualCovariance, SheetId, TransformAnalysisSpec,
};
use curvebook_engine::EngineError;
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::control_io::dispatch_ui_command_as_ui;
use crate::dialogs_canvas::{ActiveDialog, AnalysisCategory, AnalyzeDialogState, AnalyzeDuplicateChoice};
use crate::grid_view::ImportOriginKind;
use crate::interaction::{clear_modal_text_buffers, close_active_modal, sync_edit_params_text_buffers, sync_format_axes_text_buffers};
use crate::modal_predicates::{
    evaluate_dialog_button_for_app, is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1_sphericity_or_reml_control,
    r1b_post_hoc_option_enabled, r1b_sphericity_correction, r2_assume_gaussian_allowed, sync_format_points_fallback_targets,
};

fn bad_input(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, msg)
}

fn disabled_err(reason: impl Into<String>) -> EngineError {
    let msg = reason.into();
    EngineError::with_code(ErrorCode::Conflict, msg.clone())
        .with_remediation("Satisfy the control precondition or select a compatible mode/sheet first.")
        .with_details(json!({ "disabled": true, "disabled_reason": msg }))
}

/// Apply a programmatic click/toggle to a dialog widget by `widget_id` (`F6`).
pub(crate) fn apply_dialog_widget_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    try_toggle_dialog_widget_click(app, wid)
}

/// Execute a modal footer button action (`ok`, `create`, `cancel`, `apply`, `next`, `back`, `preview`).
pub(crate) fn execute_dialog_button_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    let trimmed = action.trim();
    if let Some(res) = try_execute_internal_modal_action(app, trimmed)? {
        return Ok(res);
    }
    if app.canvas_dialogs.active_dialog == ActiveDialog::None {
        return Err(EngineError::with_code(ErrorCode::Conflict, "no modal dialog is currently open"));
    }
    let norm = trimmed.to_ascii_lowercase();
    if norm == "cancel" {
        let prev = format!("{:?}", app.canvas_dialogs.active_dialog);
        super::graph_modals::reset_graph_modal_on_close(&mut app.egui_views.graph_modals);
        app.egui_views.app_modals.export_emf_selected = false;
        app.egui_views.stats_modals.open_session_sheet_id = None;
        app.egui_views.stats_modals.numeric_field_errors.clear();
        super::fit_modal::reset_session_drafts(&mut app.egui_views.fit_modal);
        clear_modal_text_buffers(app);
        close_active_modal(app);
        return Ok(json!({ "action": "cancel", "closed": true, "previous_dialog": prev }));
    }
    sync_format_points_fallback_targets(app);
    let Some(check) = evaluate_dialog_button_for_app(app, &norm) else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("dialog button `{action}` not found on active modal")));
    };
    if !check.enabled {
        return Err(disabled_err(check.disabled_reason.unwrap_or_else(|| format!("dialog button `{action}` is currently disabled"))));
    }
    commit_active_dialog_action(app, &norm, &Value::Null)
}

fn try_execute_internal_modal_action(app: &mut UiApp, action: &str) -> Result<Option<Value>, EngineError> {
    match action {
        "__manage_vars_ok" => commit_manage_vars_ok_action(app).map(Some),
        "__transform_standalone_commit" => commit_transform_standalone_action(app).map(Some),
        "__import_wizard_commit" => commit_import_wizard_action(app).map(Some),
        "__import_project_mode_replace" => commit_import_project_mode_action(app, false).map(Some),
        "__import_project_mode_merge" => commit_import_project_mode_action(app, true).map(Some),
        "__save_edit_params_default" => Ok(Some(commit_save_edit_params_default_action(app))),
        other if other.starts_with("__analyze_category:") => {
            let slug = other.trim_start_matches("__analyze_category:");
            Ok(Some(commit_analyze_category_action(app, slug)))
        }
        _ => Ok(None),
    }
}

fn commit_manage_vars_ok_action(app: &mut UiApp) -> Result<Value, EngineError> {
    let cmds = super::manage_vars_modal::build_manage_vars_ok_commands(app);
    let count = cmds.len();
    if !cmds.is_empty() {
        app.execute_atomic_group(|app| {
            for (cmd_id, params) in cmds {
                let _ = app.execute_command(&cmd_id, params)?;
            }
            Ok::<(), EngineError>(())
        })?;
    }
    app.egui_views.manage_vars_modal.session_undo_depth = None;
    app.egui_views.manage_vars_modal.session_mutation_seq = None;
    app.egui_views.manage_vars_modal.staged_title_unit_drafts.clear();
    app.grid.variables_dialog.recode_mappings.clear();
    app.grid.variables_dialog.open = false;
    Ok(json!({ "action": "ok", "closed": true, "command_count": count }))
}

fn commit_transform_standalone_action(app: &mut UiApp) -> Result<Value, EngineError> {
    let Some(cmd_params) = app.egui_views.transform_modals.pending_standalone_transform_params.clone() else {
        return Err(bad_input("no pending transform parameters to execute"));
    };
    let out = app.execute_command("transform.run", cmd_params)?;
    app.egui_views.transform_modals.pending_standalone_transform_params = None;
    app.egui_views.transform_modals.validation_error = None;
    app.egui_views.transform_modals.open_kind = None;
    Ok(json!({ "action": "ok", "closed": true, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_import_project_mode_action(app: &mut UiApp, merge: bool) -> Result<Value, EngineError> {
    let ActiveDialog::ImportProjectMode { command_id, path, .. } = app.canvas_dialogs.active_dialog.clone() else {
        return Err(EngineError::with_code(ErrorCode::Conflict, "import project mode dialog is not open"));
    };
    let (exec_cmd, exec_params, clear_file_path) = match (command_id.as_str(), merge) {
        ("project.open" | "project.instantiateTemplate" | "project.merge", true) => ("project.merge", json!({ "path": path }), false),
        ("project.open", false) => ("project.open", json!({ "path": path }), false),
        ("project.instantiateTemplate", false) => ("project.instantiateTemplate", json!({ "path": path }), true),
        _ => (command_id.as_str(), json!({ "path": path, "merge": merge }), !merge),
    };
    let out = app.execute_command(exec_cmd, exec_params)?;
    if clear_file_path {
        app.session_mut().set_file_path(None, false);
    }
    app.canvas_dialogs.close_dialog();
    let source_label = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or(&path);
    app.record_import_warnings(exec_cmd, source_label, &out);
    Ok(json!({ "action": "ok", "command_id": exec_cmd, "merge": merge, "closed": true, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_save_edit_params_default_action(app: &mut UiApp) -> Value {
    sync_edit_params_text_buffers(app);
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.clone() {
        let key = super::stats_modals::saved_default_kind_key(&spec.kind);
        app.egui_views.stats_modals.saved_defaults.retain(|d| super::stats_modals::saved_default_kind_key(&d.kind) != key);
        app.egui_views.stats_modals.saved_defaults.push(spec);
        app.egui_views.stats_modals.method_saved_as_default = true;
    } else {
        app.egui_views.stats_modals.method_saved_as_default = true;
    }
    json!({ "action": "save_default", "saved": true })
}

fn commit_analyze_category_action(app: &mut UiApp, slug: &str) -> Value {
    let cat = match slug {
        "column" => Some(AnalysisCategory::Column),
        "grouped" => Some(AnalysisCategory::Grouped),
        "xy" => Some(AnalysisCategory::XyAndRegression),
        "contingency" => Some(AnalysisCategory::Contingency),
        "survival" => Some(AnalysisCategory::Survival),
        "multivariate" => Some(AnalysisCategory::Multivariate),
        "transform" => Some(AnalysisCategory::Transform),
        _ => None,
    };
    app.canvas_dialogs.analyze.category_filter = cat;
    app.egui_views.stats_modals.analyze_category_filter = cat;
    json!({ "action": "category_filter", "category": slug })
}

fn commit_import_wizard_action(app: &mut UiApp) -> Result<Value, EngineError> {
    if app.grid.import_wizard.origin_kind == ImportOriginKind::XlsxWorkbook {
        if !app.grid.import_wizard.xlsx_sheets.is_empty() && app.grid.import_wizard.selected_xlsx_sheet_names().is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "select at least one worksheet before importing the .xlsx workbook"));
        }
        let Some(params) = app.grid.import_wizard.build_xlsx_import_params() else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "choose an .xlsx file path or load workbook bytes before importing"));
        };
        let outcome = app.execute_command("table.importXlsx", params)?;
        app.egui_views.import_wizard.pending_commit = None;
        app.grid.import_wizard.open = false;
        return Ok(
            json!({ "action": "ok", "command_id": "table.importXlsx", "closed": true, "mutated": outcome.mutated, "payload": outcome.payload }),
        );
    }
    if matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile)
        && !app.grid.import_wizard.is_live_link
        && app.grid.import_wizard.split_by_column.is_some()
    {
        let Some(params) = app.grid.import_wizard.build_csv_import_params("Imported Data") else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "enter delimited text or a CSV file path before splitting by column"));
        };
        let outcome = app.execute_command("table.importCsv", params)?;
        app.egui_views.import_wizard.pending_commit = None;
        app.grid.import_wizard.open = false;
        return Ok(
            json!({ "action": "ok", "command_id": "table.importCsv", "closed": true, "mutated": outcome.mutated, "payload": outcome.payload }),
        );
    }

    let pending_meta = app.egui_views.import_wizard.pending_commit.clone().unwrap_or_default();
    let result = app.execute_atomic_group(|app| {
        let (target_id, primary_cmd, primary_outcome) = {
            let doc = app.document();
            let existing_target = app
                .grid
                .import_wizard
                .target_sheet_id
                .as_ref()
                .filter(|id| doc.data_sheet(id).is_some())
                .cloned()
                .or_else(|| doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).cloned());
            if let Some(id) = existing_target {
                validate_import_wizard_internal_source(app, &id)?;
                let tool_intent = app.grid.import_wizard.build_command_intent(&id, Some(app.document()));
                let outcome = app.execute_command(&tool_intent.command_id, tool_intent.params)?;
                (id, tool_intent.command_id, outcome)
            } else if doc.data_sheets.is_empty()
                && matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile)
            {
                if app.grid.import_wizard.origin_kind == ImportOriginKind::ExternalDelimitedFile
                    && !app.grid.import_wizard.is_live_link
                    && app.grid.import_wizard.raw_clipboard_text.trim().is_empty()
                    && !app.grid.import_wizard.external_file_path.trim().is_empty()
                {
                    let Some(params) = app.grid.import_wizard.build_csv_import_params("Imported Data") else {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "choose a CSV file path before importing"));
                    };
                    let outcome = app.execute_command("table.importCsv", params)?;
                    let created_id = outcome
                        .payload
                        .get("sheet_id")
                        .and_then(Value::as_str)
                        .and_then(|s| SheetId::try_new(s).ok())
                        .or_else(|| app.document().data_sheets.last().map(|s| s.id.clone()))
                        .ok_or_else(|| EngineError::with_code(ErrorCode::Conflict, "failed to resolve imported data sheet"))?;
                    (created_id, String::from("table.importCsv"), outcome)
                } else {
                    let created =
                        app.execute_command("sheet.createData", json!({ "kind": "Column", "title": "Imported Data", "auto_info": true }))?;
                    let created_id = created
                        .payload
                        .get("sheet_id")
                        .and_then(Value::as_str)
                        .and_then(|s| SheetId::try_new(s).ok())
                        .or_else(|| app.document().data_sheets.last().map(|s| s.id.clone()))
                        .ok_or_else(|| EngineError::with_code(ErrorCode::Conflict, "failed to create target data sheet for import wizard"))?;
                    let tool_intent = app.grid.import_wizard.build_command_intent(&created_id, Some(app.document()));
                    let outcome = app.execute_command(&tool_intent.command_id, tool_intent.params)?;
                    (created_id, tool_intent.command_id, outcome)
                }
            } else {
                return Err(EngineError::with_code(
                    ErrorCode::Conflict,
                    "no target data sheet available for import wizard; activate or select a target data sheet first",
                ));
            }
        };

        let info_target_id = app
            .document()
            .data_sheet(&target_id)
            .and_then(|d| d.info_sheet_id.clone())
            .filter(|iid| app.document().info_sheet(iid).is_some())
            .or_else(|| app.document().info_sheets.first().map(|i| i.id.clone()));

        if let Some(info_id) = info_target_id {
            for (key, val_json) in &pending_meta.extracted_kv_json {
                if let Ok(val) = serde_json::from_str::<Value>(val_json) {
                    let _ = app.execute_command(
                        "info.setConstant",
                        json!({
                            "sheet_id": info_id.as_str(),
                            "name": key,
                            "value": val,
                        }),
                    )?;
                }
            }
            if let Some(note_text) = &pending_meta.info_note_text {
                let _ = app.execute_command(
                    "info.setNote",
                    json!({
                        "sheet_id": info_id.as_str(),
                        "text": note_text,
                        "append": true,
                    }),
                )?;
            }
        }
        Ok::<(String, curvebook_engine::CommandOutcome), EngineError>((primary_cmd, primary_outcome))
    })?;

    app.egui_views.import_wizard.pending_commit = None;
    app.grid.import_wizard.open = false;
    Ok(json!({ "action": "ok", "command_id": result.0, "closed": true, "mutated": result.1.mutated, "payload": result.1.payload }))
}

fn validate_import_wizard_internal_source(app: &UiApp, target_id: &SheetId) -> Result<(), EngineError> {
    if matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::InternalDataSheet | ImportOriginKind::InternalResultSheet) {
        let Some(src_id) = app.grid.import_wizard.source_sheet_id.as_ref() else {
            return Err(EngineError::with_code(ErrorCode::Conflict, "select a source sheet before committing an internal link or import"));
        };
        if src_id == target_id {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("cannot link or import sheet `{}` onto itself; choose a different source or target sheet", target_id.as_str()),
            ));
        }
    }
    Ok(())
}

fn button_not_found(action: &str) -> EngineError {
    EngineError::with_code(ErrorCode::NotFound, format!("dialog button `{action}` not found on active modal"))
}

pub(crate) fn commit_active_dialog_action(app: &mut UiApp, action: &str, params: &Value) -> Result<Value, EngineError> {
    let norm = action.trim().to_ascii_lowercase();
    let act = norm.as_str();
    let dialog_snapshot = app.canvas_dialogs.active_dialog.clone();
    match dialog_snapshot {
        ActiveDialog::PowerAndSampleSize => commit_power_wizard_action(app, act),
        ActiveDialog::EditAnalysisParameters { .. }
        | ActiveDialog::ChangeGraphType { .. }
        | ActiveDialog::FormatAxes { .. }
        | ActiveDialog::PairwiseComparisons { .. }
        | ActiveDialog::FormatGraph { .. }
        | ActiveDialog::FormatEmbeddedTable { .. }
        | ActiveDialog::FormatImage { .. } => {
            if !matches!(act, "ok" | "apply") {
                return Err(button_not_found(action));
            }
            match dialog_snapshot {
                ActiveDialog::EditAnalysisParameters { .. } => commit_edit_params_action(app, act),
                ActiveDialog::ChangeGraphType { .. } => {
                    let close = act != "apply";
                    let intents = app.canvas_dialogs.change_graph_type.commit_intents(app.document());
                    let res = commit_many_intents_opt_close(app, intents, close)?;
                    if close {
                        super::graph_modals::reset_graph_modal_on_close(&mut app.egui_views.graph_modals);
                    }
                    Ok(res)
                }
                ActiveDialog::FormatAxes { .. } => {
                    sync_format_axes_text_buffers(app);
                    let close = act != "apply";
                    let intent = super::graph_modals::format_axes_commit_intent(app).map_err(bad_input)?;
                    let res = commit_single_intent(app, intent, close)?;
                    super::graph_modals::on_format_axes_committed(&mut app.egui_views.graph_modals, close);
                    if close {
                        clear_modal_text_buffers(app);
                    }
                    Ok(res)
                }
                ActiveDialog::PairwiseComparisons { .. } => {
                    let close = act != "apply";
                    let res = commit_many_intents_opt_close(app, app.canvas_dialogs.pairwise.commit_intents(), close)?;
                    if close {
                        super::graph_modals::reset_graph_modal_on_close(&mut app.egui_views.graph_modals);
                    }
                    Ok(res)
                }
                ActiveDialog::FormatGraph { .. } => {
                    let close = act != "apply";
                    let intents = super::graph_modals::format_graph_commit_intents(app).map_err(bad_input)?;
                    let res = commit_many_intents_opt_close(app, intents, close)?;
                    super::graph_modals::on_format_graph_committed(app, close);
                    Ok(res)
                }
                ActiveDialog::FormatEmbeddedTable { .. } => {
                    let doc = app.document().clone();
                    let intent = app.canvas_dialogs.format_embedded_table.commit_intent(&doc);
                    commit_single_intent(app, intent, act != "apply")
                }
                ActiveDialog::FormatImage { .. } => commit_single_intent(app, app.canvas_dialogs.format_image.commit_intent(), act != "apply"),
                _ => Err(button_not_found(action)),
            }
        }
        ActiveDialog::ConvertArchetype { .. } => {
            if !matches!(act, "ok" | "preview") {
                return Err(button_not_found(action));
            }
            if act == "preview" || app.canvas_dialogs.convert_archetype.preview_only {
                let intent = app
                    .canvas_dialogs
                    .convert_archetype
                    .preview_intent()
                    .ok_or_else(|| bad_input("no data sheet bound for archetype conversion preview"))?;
                let out = app.execute_command(&intent.command_id, intent.params)?;
                app.canvas_dialogs.convert_archetype.apply_preview_payload(&out.payload);
                if act == "ok" {
                    app.canvas_dialogs.close_dialog();
                }
                Ok(json!({ "action": act, "mutated": out.mutated, "preview_only": true, "payload": out.payload }))
            } else {
                app.commit_convert_archetype().map(|o| json!({ "mutated": o.mutated, "payload": o.payload })).map_err(bad_input)
            }
        }
        _ => {
            if act != "ok" {
                return Err(button_not_found(action));
            }
            match dialog_snapshot {
                ActiveDialog::FormatSelectedPoints { .. } => commit_format_points_action(app),
                ActiveDialog::AnalyzeSheet { .. } => commit_analyze_sheet_action(app, params),
                ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } => commit_duplicate_prompt(app, &existing_result_ids, params),
                ActiveDialog::ChangeDataAnalyzed { .. } => commit_single_intent(app, app.canvas_dialogs.change_data.commit_intent(), true),
                ActiveDialog::BubblePlotInspector { .. } => commit_single_intent(app, app.canvas_dialogs.bubble_plot.commit_intent(), true),
                ActiveDialog::ApplyStyleFrom { .. } => commit_single_intent(app, app.canvas_dialogs.apply_style.commit_intent(), true),
                ActiveDialog::PageLayoutComposer { .. } => {
                    let res = commit_many_intents_opt_close(app, app.canvas_dialogs.page_layout.commit_intents(), true)?;
                    super::graph_modals::reset_graph_modal_on_close(&mut app.egui_views.graph_modals);
                    Ok(res)
                }
                ActiveDialog::ExportFigure { sheet_id } => {
                    let selected_renderables = super::app_modals::gallery_selected_renderable_ids(app);
                    let batch_scope = app.egui_views.app_modals.export_selected_gallery_scope && !selected_renderables.is_empty();
                    let export_emf = app.egui_views.app_modals.export_emf_selected;
                    let export_opts = &app.canvas_dialogs.export_figure;
                    let opts_snapshot = json!({
                        "transparent_bg": export_opts.transparent_bg,
                        "color_space": export_opts.color_space,
                        "unit": export_opts.unit,
                        "emf": export_emf,
                    });
                    let mut res = if batch_scope {
                        let is_multi_pdf = !export_emf
                            && matches!(app.canvas_dialogs.export_figure.format, crate::dialogs_canvas::ExportFigureFormat::Pdf)
                            && selected_renderables.len() >= 2;
                        if is_multi_pdf {
                            let (_, mut params) = super::app_modals::export_figure_command_and_params(app, false, &selected_renderables[0], false);
                            if let Some(obj) = params.as_object_mut() {
                                obj.remove("sheet_id");
                                let ids: Vec<&str> = selected_renderables.iter().map(SheetId::as_str).collect();
                                obj.insert(String::from("sheet_ids"), json!(ids));
                            }
                            commit_single_intent(app, Some(curvebook_tools::ToolCommandIntent::new("export.pdf", params)), true)?
                        } else {
                            let cmds = selected_renderables
                                .iter()
                                .map(|sid| {
                                    let is_layout = app.document().layout_sheet(sid).is_some();
                                    let (cmd, params) = super::app_modals::export_figure_command_and_params(app, is_layout, sid, export_emf);
                                    curvebook_tools::ToolCommandIntent::new(cmd, params)
                                })
                                .collect();
                            commit_many_intents_opt_close(app, cmds, true)?
                        }
                    } else {
                        let is_layout = app.document().layout_sheet(&sheet_id).is_some();
                        let (cmd, params) = super::app_modals::export_figure_command_and_params(app, is_layout, &sheet_id, export_emf);
                        commit_single_intent(app, Some(curvebook_tools::ToolCommandIntent::new(cmd, params)), true)?
                    };
                    app.egui_views.app_modals.export_emf_selected = false;
                    app.egui_views.app_modals.export_selected_gallery_scope = false;
                    if let Some(obj) = res.as_object_mut() {
                        obj.insert(String::from("export_options"), opts_snapshot);
                    }
                    Ok(res)
                }
                ActiveDialog::ImportReshape => {
                    let payload = crate::import_paste::commit_import_paste_dialog(app).map_err(bad_input)?;
                    app.canvas_dialogs.close_dialog();
                    Ok(json!({ "action": act, "mutated": true, "payload": payload }))
                }
                ActiveDialog::ExcludeWithReason { .. } => {
                    let intent = app.canvas_dialogs.exclude_with_reason.commit_intent().map_err(bad_input)?;
                    commit_single_intent(app, Some(intent), true)
                }
                ActiveDialog::Print { .. } => {
                    let submit = app.canvas_dialogs.print.submit_to_os;
                    let out = app.execute_print_dialog_spool(submit).map_err(bad_input)?;
                    app.canvas_dialogs.close_dialog();
                    Ok(json!({
                        "action": act,
                        "printed": submit,
                        "spooled": true,
                        "submitted_to_os": submit,
                        "page_count": out.page_count,
                        "last_status": app.canvas_dialogs.print.last_status,
                    }))
                }
                ActiveDialog::RandomizeGroups => commit_single_intent(app, app.canvas_dialogs.randomize_groups.commit_intent(), true),
                ActiveDialog::RandomSampleRows { .. } => commit_single_intent(app, app.canvas_dialogs.random_sample_rows.commit_intent(), true),
                ActiveDialog::SimulateDistributions => commit_single_intent(app, app.canvas_dialogs.simulate_distributions.commit_intent(), true),
                ActiveDialog::InsertModelEquation { .. } => {
                    let doc = app.document().clone();
                    commit_single_intent(app, app.canvas_dialogs.insert_model_equation.commit_intent(Some(&doc)), true)
                }
                ActiveDialog::CreateGraph { .. } => commit_single_intent(app, app.canvas_dialogs.create_graph.commit_intent(), true),
                ActiveDialog::EqualizeScaling { .. } => commit_single_intent(app, app.canvas_dialogs.equalize_scaling.commit_intent(), true),
                ActiveDialog::CommandPalette => commit_command_palette_action(app),
                ActiveDialog::ConstantHookPicker { .. } => commit_single_intent(app, app.canvas_dialogs.hook_picker.commit_intent(), true),
                ActiveDialog::Checkpoints => {
                    let name = app.canvas_dialogs.checkpoints.name_draft.trim().to_owned();
                    if name.is_empty() {
                        return Err(disabled_err("Enter a checkpoint name before saving."));
                    }
                    let mut params = Map::new();
                    params.insert(String::from("name"), Value::String(name));
                    let note = app.canvas_dialogs.checkpoints.note_draft.trim().to_owned();
                    if !note.is_empty() {
                        params.insert(String::from("note"), Value::String(note));
                    }
                    let outcome = app.execute_command("project.createCheckpoint", Value::Object(params))?;
                    let doc = app.document().clone();
                    app.canvas_dialogs.checkpoints.name_draft = format!("Checkpoint {}", doc.checkpoints.len() + 1);
                    app.canvas_dialogs.checkpoints.note_draft.clear();
                    Ok(outcome.payload)
                }
                ActiveDialog::ResultViewer { .. }
                | ActiveDialog::About
                | ActiveDialog::ImportWarnings
                | ActiveDialog::ImportProjectMode { .. }
                | ActiveDialog::CsvExportOptions(_)
                | ActiveDialog::ExportAllTablesExistingFiles { .. }
                | ActiveDialog::PzfxExportConfirm { .. }
                | ActiveDialog::None => {
                    if app.canvas_dialogs.active_dialog == ActiveDialog::ImportWarnings {
                        app.import_warnings.panel_open = false;
                    }
                    app.canvas_dialogs.close_dialog();
                    Ok(json!({ "action": act, "closed": true }))
                }
                _ => Err(button_not_found(action)),
            }
        }
    }
}

fn commit_power_wizard_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    match action {
        "next" => {
            let next = app.canvas_dialogs.power_wizard.advance().map_err(disabled_err)?;
            Ok(json!({ "action": "next", "stage": next.label(), "stage_index": next.index() }))
        }
        "back" => {
            let prev = app.canvas_dialogs.power_wizard.go_back().map_err(disabled_err)?;
            Ok(json!({ "action": "back", "stage": prev.label(), "stage_index": prev.index() }))
        }
        "create" | "ok" => crate::control_io::host_routing::commit_power_wizard_create(app, action),
        _ => Err(button_not_found(action)),
    }
}

fn commit_format_points_action(app: &mut UiApp) -> Result<Value, EngineError> {
    sync_format_points_fallback_targets(app);
    let count = app.canvas_dialogs.format_points.selected_points.len();
    let outcomes = app.execute_atomic_group(|app| {
        let mut out_payloads = Vec::new();
        if let Some(intent) = app.canvas_dialogs.format_points.build_commit_intent() {
            let out = app.execute_command(&intent.command_id, intent.params)?;
            out_payloads.push(out.payload);
        }
        let scene = app.canvas_dialogs.format_points.graph_sheet_id.as_ref().and_then(|gid| app.session().render_sheet_scene(gid).ok());
        let row_label_intents = app.canvas_dialogs.format_points.build_row_label_intents_for_scene(app.document(), scene.as_ref());
        for intent in row_label_intents {
            let out = app.execute_command(&intent.command_id, intent.params)?;
            out_payloads.push(out.payload);
        }
        Ok::<Vec<Value>, EngineError>(out_payloads)
    })?;
    app.canvas_dialogs.close_dialog();
    Ok(json!({ "action": "ok", "formatted_count": count, "outcomes": outcomes }))
}

fn maybe_open_nonlinear_fit_params(app: &mut UiApp, sheet_id_str: &str, from_physical_gui: bool) {
    if !from_physical_gui || app.canvas_dialogs.suppress_auto_open_fit_params {
        return;
    }
    let Ok(rs_id) = SheetId::try_new(sheet_id_str) else {
        return;
    };
    let doc = app.session.document().clone();
    if doc.result_sheet(&rs_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::NonlinearRegression(_))) {
        let _ = app.canvas_dialogs.open_edit_parameters(&doc, &rs_id);
    }
}

fn resolve_ext01_equation(doc: &ProjectDocument, analyze: &AnalyzeDialogState, src_id: &SheetId, mv_x_columns: &[usize]) -> Option<String> {
    let eq_trimmed = analyze.equation_id.trim();
    if (!eq_trimmed.is_empty() && eq_trimmed != "log_agonist_vs_response_4pl") || analyze.custom_dsl.is_some() {
        return None;
    }
    let xs = super::fit_modal::collect_sheet_active_x_values(doc, src_id, None, mv_x_columns);
    Some(curvebook_fit::suggested_default_dose_response_equation(&xs).to_owned())
}

fn apply_ext01_default_nonlinear_equation(doc: &ProjectDocument, analyze: &AnalyzeDialogState, intent: &mut curvebook_tools::ToolCommandIntent) {
    if intent.command_id != "analysis.run" {
        return;
    }
    let Some(src_str) = intent.params.get("source_sheet_id").and_then(Value::as_str) else {
        return;
    };
    let Ok(src_id) = SheetId::try_new(src_str) else {
        return;
    };
    let Some(suggested) = resolve_ext01_equation(doc, analyze, &src_id, &analyze.mv_x_columns) else {
        return;
    };
    if let Some(nl_obj) =
        intent.params.get_mut("kind").and_then(Value::as_object_mut).and_then(|k| k.get_mut("NonlinearRegression")).and_then(Value::as_object_mut)
    {
        let eq = nl_obj.get("equation_id").and_then(Value::as_str).unwrap_or("");
        if eq.is_empty() || eq == "log_agonist_vs_response_4pl" {
            nl_obj.insert(String::from("equation_id"), Value::String(suggested));
        }
    } else if intent.params.get("kind").and_then(Value::as_str) == Some("nonlinear_regression")
        && let Some(obj) = intent.params.as_object_mut()
    {
        let eq = obj.get("equation_id").and_then(Value::as_str).unwrap_or("");
        if eq.is_empty() || eq == "log_agonist_vs_response_4pl" {
            obj.insert(String::from("equation_id"), Value::String(suggested));
        }
    }
}

fn stage_pending_nonlinear_create(app: &mut UiApp, pending_extract: Option<(SheetId, String)>) -> Result<Value, EngineError> {
    let doc = app.document().clone();
    let src_id = app.canvas_dialogs.analyze.source_sheet_id.clone().ok_or_else(|| bad_input("Analyze selection incomplete"))?;
    let mut kind = app.canvas_dialogs.analyze.build_analysis_kind().ok_or_else(|| bad_input("Analyze selection incomplete"))?;
    if let AnalysisKind::NonlinearRegression(nl) = &mut kind
        && let Some(suggested) = resolve_ext01_equation(&doc, &app.canvas_dialogs.analyze, &src_id, &nl.mv_x_columns)
    {
        nl.equation_id = suggested;
    }
    let seed_id = SheetId::try_new("pending-nonlinear-fit").map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
    let mut spec = AnalysisSpec::new(seed_id, src_id, kind).map_err(|e| bad_input(e.to_string()))?;
    spec.selected_data_sets.clone_from(&app.canvas_dialogs.analyze.selected_data_sets);

    super::fit_modal::reset_session_drafts(&mut app.egui_views.fit_modal);
    clear_modal_text_buffers(app);
    crate::dialogs_canvas::clear_fit_parameter_inputs(app);
    app.egui_views.stats_modals.open_session_sheet_id = None;
    app.egui_views.stats_modals.numeric_field_errors.clear();
    app.canvas_dialogs.analyze.extracted_result_tab_id = None;
    app.canvas_dialogs.analyze.duplicate_choice = crate::dialogs_canvas::DuplicateChoice::CreateNew;
    app.canvas_dialogs.analyze.duplicate_target_id = None;

    let draft_id = app.canvas_dialogs.open_edit_parameters_for_create(&doc, spec, pending_extract);
    Ok(json!({
        "action": "ok",
        "mutated": false,
        "deferred_create": true,
        "dialog": "edit_analysis_parameters",
        "draft_sheet_id": draft_id.as_str(),
    }))
}

fn should_defer_nonlinear_create(app: &UiApp, from_physical_gui: bool, kind: Option<&AnalysisKind>) -> bool {
    from_physical_gui && !app.canvas_dialogs.suppress_auto_open_fit_params && matches!(kind, Some(AnalysisKind::NonlinearRegression(_)))
}

fn execute_result_tab_extract_for_fit(app: &mut UiApp, src_id: &SheetId, tab_id: &str) -> Result<SheetId, EngineError> {
    let ext = app.execute_command(
        "sheet.extractResultTab",
        json!({
            "result_sheet_id": src_id.as_str(),
            "tab_id": tab_id,
            "archetype": "auto",
            "auto_graph": false,
        }),
    )?;
    let extracted_id = ext
        .payload
        .get("sheet_id")
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "extractResultTab did not return `sheet_id`"))?;
    SheetId::try_new(extracted_id).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))
}

fn execute_analyze_intent_atomically(
    app: &mut UiApp,
    mut intent: curvebook_tools::ToolCommandIntent,
    pending_extract: Option<(SheetId, String)>,
    spawn_corr_heatmap: bool,
) -> Result<curvebook_engine::CommandOutcome, EngineError> {
    app.execute_atomic_group(|app| {
        if let Some((src_id, tab_id)) = pending_extract
            && app.document().result_sheet(&src_id).is_some_and(|rs| rs.analyzable_data.is_none())
        {
            let sid = execute_result_tab_extract_for_fit(app, &src_id, &tab_id)?;
            if let Some(obj) = intent.params.as_object_mut() {
                obj.insert(String::from("source_sheet_id"), Value::String(sid.as_str().to_owned()));
            }
        }
        apply_ext01_default_nonlinear_equation(app.document(), &app.canvas_dialogs.analyze, &mut intent);
        let out = app.execute_command(&intent.command_id, intent.params)?;
        if spawn_corr_heatmap
            && let Some(res_id_str) = out.payload.get("sheet_id").and_then(Value::as_str)
            && let Ok(res_id) = SheetId::try_new(res_id_str)
            && app
                .document()
                .result_sheet(&res_id)
                .is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })))
            && !app
                .document()
                .graph_sheets
                .iter()
                .any(|g| g.spec.primary_source_sheet_id == res_id && g.spec.kind == GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))
        {
            let _ = app.execute_command(
                "graph.create",
                json!({
                    "data_sheet_id": res_id.as_str(),
                    "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap),
                    "initialized": true,
                }),
            )?;
        }
        Ok(out)
    })
}

fn commit_analyze_sheet_action(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let pending_extract = app
        .canvas_dialogs
        .analyze
        .source_sheet_id
        .clone()
        .zip(app.canvas_dialogs.analyze.extracted_result_tab_id.clone())
        .filter(|(src_id, _)| app.document().result_sheet(src_id).is_some_and(|rs| rs.analyzable_data.is_none()));
    let spawn_corr_heatmap = app.canvas_dialogs.analyze.correlation_all_pairs && app.canvas_dialogs.analyze.correlation_heatmap;

    let from_physical_gui = params.is_null();
    let transient_force = std::mem::take(&mut app.canvas_dialogs.analyze.force_create_new_once);
    let force_new = transient_force || params.get("force_create_new").and_then(Value::as_bool).unwrap_or(false);
    let prev_dialog = app.canvas_dialogs.active_dialog.clone();
    let planned_kind = app.canvas_dialogs.analyze.build_analysis_kind();
    let defer_nonlinear = should_defer_nonlinear_create(app, from_physical_gui, planned_kind.as_ref());

    if !force_new && defer_nonlinear && !app.canvas_dialogs.analyze.matching_existing_analyses(app.session.document()).is_empty() {
        let _ = app.canvas_dialogs.submit_analyze_or_prompt(app.session.document());
        if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
            return Ok(json!({ "action": "ok", "dialog": "duplicate_analysis_prompt", "pending_confirmation": true }));
        }
    }
    if defer_nonlinear && planned_kind.is_some() {
        return stage_pending_nonlinear_create(app, pending_extract);
    }

    if !force_new && let Some(intent) = app.canvas_dialogs.submit_analyze_or_prompt(app.session.document()) {
        let out = match execute_analyze_intent_atomically(app, intent, pending_extract, spawn_corr_heatmap) {
            Ok(o) => o,
            Err(err) => {
                app.canvas_dialogs.active_dialog = prev_dialog;
                return Err(err);
            }
        };
        app.canvas_dialogs.analyze.extracted_result_tab_id = None;
        if let Some(new_id) = out.payload.get("sheet_id").and_then(Value::as_str) {
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
            maybe_open_nonlinear_fit_params(app, new_id, from_physical_gui);
        }
        return Ok(json!({ "action": "ok", "mutated": out.mutated, "payload": out.payload }));
    }
    if !force_new && matches!(app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        return Ok(json!({ "action": "ok", "dialog": "duplicate_analysis_prompt", "pending_confirmation": true }));
    }
    let intent = app.canvas_dialogs.analyze.build_create_new_intent().ok_or_else(|| bad_input("Analyze selection incomplete"))?;
    let out = execute_analyze_intent_atomically(app, intent, pending_extract, spawn_corr_heatmap)?;
    app.canvas_dialogs.analyze.extracted_result_tab_id = None;
    app.canvas_dialogs.close_dialog();
    if let Some(new_id) = out.payload.get("sheet_id").and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        maybe_open_nonlinear_fit_params(app, new_id, from_physical_gui);
    }
    Ok(json!({ "action": "ok", "mutated": out.mutated, "payload": out.payload }))
}

fn commit_duplicate_prompt(app: &mut UiApp, existing_ids: &[SheetId], params: &Value) -> Result<Value, EngineError> {
    let from_physical_gui = params.is_null();
    let pending_extract = app
        .canvas_dialogs
        .analyze
        .source_sheet_id
        .clone()
        .zip(app.canvas_dialogs.analyze.extracted_result_tab_id.clone())
        .filter(|(src_id, _)| app.document().result_sheet(src_id).is_some_and(|rs| rs.analyzable_data.is_none()));
    let spawn_corr_heatmap = app.canvas_dialogs.analyze.correlation_all_pairs && app.canvas_dialogs.analyze.correlation_heatmap;
    let raw_choice = match params.get("choice") {
        Some(v) => Some(v),
        None => params.get("fields").and_then(Value::as_object).and_then(|f| f.get("choice")),
    };
    let intent = match raw_choice {
        None | Some(Value::Null) => app
            .canvas_dialogs
            .analyze
            .resolve_selected_duplicate_intent(existing_ids)
            .ok_or_else(|| bad_input("failed to resolve duplicate analysis choice"))?,
        Some(other) => {
            let choice_str =
                other.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad_input("duplicate prompt `choice` must be a string"))?;
            let choice = if choice_str.eq_ignore_ascii_case("modify_existing")
                && let Some(first) = existing_ids.first()
            {
                AnalyzeDuplicateChoice::ModifyExisting(first.clone())
            } else if choice_str.eq_ignore_ascii_case("create_new") || choice_str.eq_ignore_ascii_case("modify_existing") {
                AnalyzeDuplicateChoice::CreateNew
            } else {
                return Err(bad_input(format!("unsupported duplicate analysis choice `{choice_str}`; expected create_new or modify_existing")));
            };
            app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice).ok_or_else(|| bad_input("failed to resolve duplicate analysis choice"))?
        }
    };
    let is_create_new = intent.command_id == "analysis.run";
    if is_create_new {
        let planned_kind = app.canvas_dialogs.analyze.build_analysis_kind();
        if should_defer_nonlinear_create(app, from_physical_gui, planned_kind.as_ref()) {
            return stage_pending_nonlinear_create(app, pending_extract);
        }
    }
    let cmd_id = intent.command_id.clone();
    let outcome = execute_analyze_intent_atomically(app, intent, is_create_new.then_some(pending_extract).flatten(), spawn_corr_heatmap)?;
    app.canvas_dialogs.analyze.extracted_result_tab_id = None;
    app.canvas_dialogs.close_dialog();
    if let Some(new_id) = outcome.payload.get("sheet_id").and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        maybe_open_nonlinear_fit_params(app, new_id, from_physical_gui);
    }
    app.canvas_dialogs.analyze.duplicate_choice = crate::dialogs_canvas::DuplicateChoice::CreateNew;
    app.canvas_dialogs.analyze.duplicate_target_id = None;
    Ok(json!({ "action": "ok", "command_id": cmd_id, "mutated": outcome.mutated, "payload": outcome.payload }))
}

fn commit_edit_params_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    if let Some(err) = app.egui_views.stats_modals.numeric_field_errors.values().next() {
        return Err(bad_input(err.clone()));
    }
    if let Some(err) = app.canvas_dialogs.edit_params.unsupported_contingency_ci_error() {
        return Err(bad_input(err));
    }
    sync_edit_params_text_buffers(app);
    crate::dialogs_canvas::sync_fit_parameter_inputs(app).map_err(bad_input)?;
    let close = action == "ok";
    let was_pending_create = app.canvas_dialogs.edit_params.pending_create;
    let pending_extract = app.canvas_dialogs.edit_params.pending_extract.clone();
    let doc = app.document().clone();
    let mut intents = app.canvas_dialogs.edit_params.commit_intents(Some(&doc));
    if !was_pending_create
        && app.egui_views.transform_modals.create_new_graph
        && let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog
        && app
            .canvas_dialogs
            .edit_params
            .draft
            .as_ref()
            .is_some_and(|d| matches!(d.kind, AnalysisKind::Transform(TransformAnalysisSpec::Normalize { .. })))
        && !doc.graph_sheets.iter().any(|g| g.spec.primary_source_sheet_id == *result_sheet_id)
    {
        intents.push(curvebook_tools::ToolCommandIntent::new(
            "graph.create",
            json!({
                "data_sheet_id": result_sheet_id.as_str(),
                "initialized": true,
            }),
        ));
    }
    if intents.is_empty() {
        return Err(bad_input("missing analysis parameter draft"));
    }
    let primary_out = app.execute_atomic_group(|app| {
        let mut iter = intents.into_iter();
        let mut primary = iter.next().ok_or_else(|| bad_input("missing analysis parameter draft"))?;
        if was_pending_create
            && let Some((src_id, tab_id)) = pending_extract
            && app.document().result_sheet(&src_id).is_some_and(|rs| rs.analyzable_data.is_none())
        {
            let sid = execute_result_tab_extract_for_fit(app, &src_id, &tab_id)?;
            if let Some(obj) = primary.params.as_object_mut() {
                obj.insert(String::from("source_sheet_id"), Value::String(sid.as_str().to_owned()));
            }
        }
        let out = app.execute_command(&primary.command_id, primary.params)?;
        for extra in iter {
            let _ = app.execute_command(&extra.command_id, extra.params)?;
        }
        Ok::<curvebook_engine::CommandOutcome, EngineError>(out)
    })?;
    if close {
        super::fit_modal::reset_session_drafts(&mut app.egui_views.fit_modal);
        close_active_modal(app);
        clear_modal_text_buffers(app);
        app.egui_views.stats_modals.open_session_sheet_id = None;
        app.egui_views.stats_modals.numeric_field_errors.clear();
    }
    if was_pending_create && let Some(new_id) = primary_out.payload.get("sheet_id").and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        if let Ok(sid) = SheetId::try_new(new_id) {
            let doc_after = app.document().clone();
            app.canvas_dialogs.on_sheet_selected_with_auto_modal(&doc_after, &sid, false);
        }
    }
    Ok(json!({ "action": action, "mutated": primary_out.mutated, "closed": close, "payload": primary_out.payload }))
}

fn commit_command_palette_action(app: &mut UiApp) -> Result<Value, EngineError> {
    let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    let selected = hits
        .get(app.canvas_dialogs.command_palette.selected_index)
        .or_else(|| hits.first())
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "no matching command in palette"))?
        .clone();
    if let Some(res) = dispatch_ui_command_as_ui(app, selected.id, &Value::Null)? {
        if app.canvas_dialogs.active_dialog == ActiveDialog::CommandPalette {
            app.canvas_dialogs.close_dialog();
        }
        return Ok(json!({ "action": "ok", "command_id": selected.id, "result": res }));
    }
    let out = app.execute_command(selected.id, Value::Null)?;
    if app.canvas_dialogs.active_dialog == ActiveDialog::CommandPalette {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": "ok", "command_id": selected.id, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_single_intent(app: &mut UiApp, intent: Option<curvebook_tools::ToolCommandIntent>, close: bool) -> Result<Value, EngineError> {
    let Some(i) = intent else {
        return Err(bad_input("active dialog has no valid commit parameters"));
    };
    let out = app.execute_command(&i.command_id, i.params)?;
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": "ok", "command_id": i.command_id, "mutated": out.mutated, "payload": out.payload }))
}

fn commit_many_intents_opt_close(app: &mut UiApp, intents: Vec<curvebook_tools::ToolCommandIntent>, close: bool) -> Result<Value, EngineError> {
    let payloads = app.execute_atomic_group(|app| {
        let mut out_payloads = Vec::with_capacity(intents.len());
        for i in intents {
            let out = app.execute_command(&i.command_id, i.params)?;
            out_payloads.push(out.payload);
        }
        Ok::<Vec<Value>, EngineError>(out_payloads)
    })?;
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": if close { "ok" } else { "apply" }, "count": payloads.len(), "closed": close, "payloads": payloads }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogToggleGroup {
    ImportReshape,
    ConvertArchetype,
    ApplyStyle,
    EditParams,
}

impl DialogToggleGroup {
    const fn from_dialog(dialog: &ActiveDialog) -> Option<Self> {
        match dialog {
            ActiveDialog::ImportReshape => Some(Self::ImportReshape),
            ActiveDialog::ConvertArchetype { .. } => Some(Self::ConvertArchetype),
            ActiveDialog::ApplyStyleFrom { .. } => Some(Self::ApplyStyle),
            ActiveDialog::EditAnalysisParameters { .. } => Some(Self::EditParams),
            _ => None,
        }
    }
}

struct DialogBoolToggle {
    group: DialogToggleGroup,
    widget_id: &'static str,
    field: fn(&mut UiApp) -> &mut bool,
}

const DIALOG_BOOL_TOGGLES: &[DialogBoolToggle] = &[
    DialogBoolToggle {
        group: DialogToggleGroup::ImportReshape,
        widget_id: "import.subtract_blank_mean",
        field: |a| &mut a.grid.import_reshape.subtract_blank_mean,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::ConvertArchetype,
        widget_id: "convert.allow_lossy",
        field: |a| &mut a.canvas_dialogs.convert_archetype.allow_lossy,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::ConvertArchetype,
        widget_id: "convert.preview_only",
        field: |a| &mut a.canvas_dialogs.convert_archetype.preview_only,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::ApplyStyle,
        widget_id: "apply_style.clone_palette",
        field: |a| &mut a.canvas_dialogs.apply_style.clone_palette,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::ApplyStyle,
        widget_id: "apply_style.clone_frame_and_axes",
        field: |a| &mut a.canvas_dialogs.apply_style.clone_frame_and_axes,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::ApplyStyle,
        widget_id: "apply_style.clone_error_bars_and_legend",
        field: |a| &mut a.canvas_dialogs.apply_style.clone_error_bars_and_legend,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::EditParams,
        widget_id: "edit.two_group.graph_differences",
        field: |a| &mut a.canvas_dialogs.edit_params.graph_differences,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::EditParams,
        widget_id: "edit.two_group.graph_ranks",
        field: |a| &mut a.canvas_dialogs.edit_params.graph_ranks,
    },
    DialogBoolToggle {
        group: DialogToggleGroup::EditParams,
        widget_id: "edit.two_group.hodges_lehmann",
        field: |a| &mut a.canvas_dialogs.edit_params.hodges_lehmann_ci,
    },
];

fn try_toggle_dialog_widget_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    if let Some(group) = DialogToggleGroup::from_dialog(&app.canvas_dialogs.active_dialog)
        && let Some(toggle) = DIALOG_BOOL_TOGGLES.iter().find(|t| t.group == group && t.widget_id == wid)
    {
        let flag = (toggle.field)(app);
        *flag = !*flag;
        return Ok(Some(json!({ "clicked": wid, "updated": true })));
    }
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::AnalyzeSheet { .. } => {
            let az = &mut app.canvas_dialogs.analyze;
            match wid {
                "analyze.correlation_all_pairs" => az.set_correlation_all_pairs(!az.correlation_all_pairs),
                "analyze.correlation.heatmap" => az.set_correlation_heatmap(!az.correlation_heatmap).map_err(disabled_err)?,
                "analyze.factorial.residual_covariance" => {
                    az.factorial_residual_covariance = if az.factorial_residual_covariance == ResidualCovariance::Unstructured {
                        ResidualCovariance::CompoundSymmetry
                    } else {
                        ResidualCovariance::Unstructured
                    };
                }
                _ => return Ok(None),
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::EditAnalysisParameters { .. } => try_toggle_edit_params_click(app, wid),
        _ => Ok(None),
    }
}

fn try_toggle_edit_params_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    let ep = &app.canvas_dialogs.edit_params;
    match wid {
        "edit.rm.mixed_effects_reml" | "edit.rm.df_kenward_roger" | "edit.rm.df_satterthwaite" => {
            let is_nested = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::NestedComparison(_)));
            if wid == "edit.rm.mixed_effects_reml" || !is_nested {
                let gate = r1_sphericity_or_reml_control(ep.paired_or_matched);
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason.unwrap_or_else(|| String::from("Mixed-effects REML controls require matched/repeated measures.")),
                    ));
                }
            }
            match wid {
                "edit.rm.mixed_effects_reml" => {
                    let cur = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::OneWayAnova(a)) if a.mixed_effects_reml)
                        || matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::FactorialAnova(f)) if f.mixed_effects_reml);
                    app.canvas_dialogs.edit_params.set_mixed_effects_reml(!cur);
                }
                "edit.rm.df_kenward_roger" => app.canvas_dialogs.edit_params.set_df_method(MixedModelDfMethod::KenwardRoger),
                "edit.rm.df_satterthwaite" => app.canvas_dialogs.edit_params.set_df_method(MixedModelDfMethod::Satterthwaite),
                _ => {}
            }
        }
        "edit.two_group.welch" => {
            let next = !ep.welch_correction;
            app.canvas_dialogs.edit_params.set_welch_correction(next).map_err(disabled_err)?;
        }
        "edit.contingency.attributable_risk" => {
            let next = !ep.contingency_attributable_risk_checked;
            app.canvas_dialogs.edit_params.set_contingency_attributable_risk(next);
        }
        "edit.cluster.k_means_auto" => {
            app.canvas_dialogs.edit_params.toggle_k_means_auto();
        }
        "edit.post_hoc.tukey" | "edit.post_hoc.dunnett" | "edit.post_hoc.dunn" => {
            let method = match wid {
                "edit.post_hoc.tukey" => MultiplicityMethod::TukeyHsd,
                "edit.post_hoc.dunnett" => MultiplicityMethod::Dunnett { control_column: ep.control_column },
                _ => MultiplicityMethod::DunnRankSum,
            };
            let gate = r1b_post_hoc_option_enabled(ep.assume_gaussian, is_reml_mixed_effects_active(ep, app.document()), method);
            if !gate.enabled {
                return Err(disabled_err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method is disabled."))));
            }
            app.canvas_dialogs.edit_params.set_multiplicity_method(method).map_err(disabled_err)?;
        }
        "edit.assume_gaussian" => {
            if ep.assume_gaussian {
                let gate = r2_assume_gaussian_allowed(app.document(), ep.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref()));
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason
                            .unwrap_or_else(|| String::from("Nonparametric rank tests are disabled for pre-summarized Column tables.")),
                    ));
                }
            }
            let next = !ep.assume_gaussian;
            app.canvas_dialogs.edit_params.set_assume_gaussian(next);
        }
        "edit.paired_or_matched" => {
            if !ep.paired_or_matched {
                let gate = r1_paired_or_matched_allowed(app.document(), ep.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref()));
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason.unwrap_or_else(|| String::from("Matched / Repeated Measures requires equal N >= 2")),
                    ));
                }
            }
            let next = !ep.paired_or_matched;
            app.canvas_dialogs.edit_params.set_paired_or_matched(next);
        }
        "edit.sphericity.geisser_greenhouse" => {
            let gate = r1b_sphericity_correction(ep.paired_or_matched, is_reml_mixed_effects_active(ep, app.document()));
            if !gate.enabled {
                return Err(disabled_err(gate.disabled_reason.unwrap_or_else(|| String::from("Sphericity control is disabled."))));
            }
            let gg_checked = ep.draft.as_ref().is_none_or(|d| {
                !matches!(
                    d.kind,
                    AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
                )
            });
            app.canvas_dialogs.edit_params.set_geisser_greenhouse(!gg_checked).map_err(disabled_err)?;
        }
        _ => return Ok(None),
    }
    Ok(Some(json!({ "clicked": wid, "updated": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{UiTreeSnapshot, UiWidgetNode, inspect_ui_tree};
    use crate::interaction::{UiInputEvent, ViewportMetrics, dispatch_ui_event};

    fn inspect_dialog(app: &UiApp) -> UiTreeSnapshot {
        inspect_ui_tree(app, Some("dialog"), false)
    }

    fn dialog_node<'a>(snap: &'a UiTreeSnapshot, wid: &str) -> Option<&'a UiWidgetNode> {
        snap.dialog.as_ref().and_then(|d| d.tabs.iter().chain(d.controls.iter()).chain(d.buttons.iter()).find(|n| n.widget_id == wid))
    }

    fn make_9col_mv_app() -> Result<(UiApp, SheetId, SheetId), String> {
        let mut app = UiApp::new();
        let created = app
            .execute_command(
                "sheet.createData",
                json!({ "sheet_id": "mv-9", "kind": "multiple_variables", "cols": 9, "rows": 8, "auto_family": false }),
            )
            .map_err(|e| e.to_string())?;
        let ds_id = SheetId::from(created.payload["sheet_id"].as_str().ok_or("missing mv sheet_id")?);
        app.execute_command("table.setVariableRole", json!({ "sheet_id": ds_id.as_str(), "col": 0, "role": "label" })).map_err(|e| e.to_string())?;
        app.execute_command("table.setVariableRole", json!({ "sheet_id": ds_id.as_str(), "col": 1, "role": "categorical" }))
            .map_err(|e| e.to_string())?;
        let mut edits = Vec::new();
        for r in 0..8usize {
            let grp = if r < 4 { "Ctrl" } else { "Treat" };
            edits.push(json!({ "row": r, "col": 0, "value": format!("S{r}") }));
            edits.push(json!({ "row": r, "col": 1, "value": grp }));
            for c in 2..9usize {
                edits.push(json!({ "row": r, "col": c, "value": ((r + 1) * (c + 1)) as f64 + if r >= 4 { 10.0 } else { 0.0 } }));
            }
        }
        app.execute_command("table.setCells", json!({ "sheet_id": ds_id.as_str(), "edits": edits })).map_err(|e| e.to_string())?;
        let g = app
            .execute_command("graph.create", json!({ "data_sheet_id": ds_id.as_str(), "kind": "multiple_variables" }))
            .map_err(|e| e.to_string())?;
        let gs_id = SheetId::from(g.payload["sheet_id"].as_str().ok_or("missing graph sheet_id")?);
        Ok((app, ds_id, gs_id))
    }

    #[test]
    fn w6_mv_pcr_cluster_two_group_factorial_and_fit() -> Result<(), String> {
        let (mut app, ds_id, _gs_id) = make_9col_mv_app()?;

        // 1. Analyze picker PCR inline controls & R10 outcome gating.
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_analyze_dialog(&doc, &ds_id));
        app.canvas_dialogs.analyze.select_catalog_entry("stats.pcr");
        let snap_an = inspect_dialog(&app);
        assert!(dialog_node(&snap_an, "analyze.pca.pred.0").is_some());
        assert!(dialog_node(&snap_an, "analyze.pcr.outcome.0").is_some());
        app.canvas_dialogs.analyze.toggle_pca_predictor(2);
        let snap_an2 = inspect_dialog(&app);
        assert_eq!(dialog_node(&snap_an2, "analyze.pcr.outcome.2").map(|n| n.enabled), Some(false));
        assert_eq!(dialog_node(&snap_an2, "analyze.pcr.outcome.3").map(|n| n.enabled), Some(true));
        app.canvas_dialogs.close_dialog();

        // 2. Two-group & Factorial ANOVA MV selectors (>6 cols).
        let tg = app
            .execute_command(
                "analysis.run",
                json!({ "source_sheet_id": ds_id.as_str(), "kind": "unpaired_t", "mv_group_column": 1, "mv_response_column": 8 }),
            )
            .map_err(|e| e.to_string())?;
        let tg_id = SheetId::from(tg.payload["sheet_id"].as_str().ok_or("missing tg sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &tg_id));
        let snap_tg = inspect_dialog(&app);
        assert!(dialog_node(&snap_tg, "edit.dataset.8").is_some(), "expected >6 dataset buttons rendered");
        assert!(dialog_node(&snap_tg, "edit.two_group.welch").is_some());
        assert!(dialog_node(&snap_tg, "edit.two_group.group.1").is_some());
        assert!(dialog_node(&snap_tg, "edit.two_group.resp.8").is_some());
        app.canvas_dialogs.close_dialog();

        let fac = app
            .execute_command(
                "analysis.run",
                json!({ "source_sheet_id": ds_id.as_str(), "kind": "two_way_anova", "multifactor_columns": [1], "response_column": 8 }),
            )
            .map_err(|e| e.to_string())?;
        let fac_id = SheetId::from(fac.payload["sheet_id"].as_str().ok_or("missing fac sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &fac_id));
        let snap_fac = inspect_dialog(&app);
        assert!(dialog_node(&snap_fac, "edit.fac.resp.8").is_some());
        assert!(dialog_node(&snap_fac, "edit.fac.col.1").is_some());
        app.canvas_dialogs.close_dialog();

        // 3. Clustering controls & Fit MV selectors on fit.tab.model.
        let cl =
            app.execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "clustering" })).map_err(|e| e.to_string())?;
        let cl_id = SheetId::from(cl.payload["sheet_id"].as_str().ok_or("missing clust sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &cl_id));
        let snap_cl = inspect_dialog(&app);
        for id in [
            "edit.cluster.k_means_auto",
            "edit.cluster.k_sweep_max.inc",
            "edit.cluster.dist.cosine",
            "edit.cluster.link.single",
            "edit.cluster.link.wpgma",
            "edit.cluster.link.centroid",
            "edit.cluster.link.median",
        ] {
            assert!(dialog_node(&snap_cl, id).is_some(), "missing cluster control {id}");
        }
        try_toggle_edit_params_click(&mut app, "edit.cluster.k_means_auto").map_err(|e| e.to_string())?;
        assert!(matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: Some(3), .. }))
        ));
        try_toggle_edit_params_click(&mut app, "edit.cluster.k_means_auto").map_err(|e| e.to_string())?;
        assert!(matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: None, .. }))
        ));
        app.canvas_dialogs.close_dialog();

        let fit = app
            .execute_command(
                "analysis.run",
                json!({
                    "source_sheet_id": ds_id.as_str(),
                    "kind": "nonlinear_regression",
                    "equation_id": "straight_line",
                    "mv_x_columns": [2],
                    "mv_y_column": 3,
                    "mv_group_columns": [1],
                }),
            )
            .map_err(|e| e.to_string())?;
        let fit_id = SheetId::from(fit.payload["sheet_id"].as_str().ok_or("missing fit sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &fit_id));
        let snap_fit = inspect_dialog(&app);
        assert!(dialog_node(&snap_fit, "fit.tab.model").is_some());
        assert!(dialog_node(&snap_fit, "fit.mv.x.2").is_some());
        assert!(dialog_node(&snap_fit, "fit.mv.y.3").is_some());
        assert!(dialog_node(&snap_fit, "fit.mv.group.1").is_some());
        Ok(())
    }

    #[test]
    fn w6_live_dialog_widget_clicks_table() -> Result<(), String> {
        let (mut app, ds_id, gs_id) = make_9col_mv_app()?;

        app.canvas_dialogs.open_import_reshape();
        let before = app.grid.import_reshape.subtract_blank_mean;
        assert!(apply_dialog_widget_click(&mut app, "import.subtract_blank_mean").map_err(|e| e.to_string())?.is_some());
        assert_eq!(app.grid.import_reshape.subtract_blank_mean, !before);

        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_convert_archetype(&doc, &ds_id));
        assert!(apply_dialog_widget_click(&mut app, "convert.allow_lossy").map_err(|e| e.to_string())?.is_some());
        assert!(apply_dialog_widget_click(&mut app, "convert.preview_only").map_err(|e| e.to_string())?.is_some());

        assert!(app.canvas_dialogs.open_apply_style_from(&doc, &gs_id));
        let before_pal = app.canvas_dialogs.apply_style.clone_palette;
        assert!(apply_dialog_widget_click(&mut app, "apply_style.clone_palette").map_err(|e| e.to_string())?.is_some());
        assert_eq!(app.canvas_dialogs.apply_style.clone_palette, !before_pal);

        assert!(apply_dialog_widget_click(&mut app, "dead.legacy.widget_id").map_err(|e| e.to_string())?.is_none());
        app.canvas_dialogs.close_dialog();
        Ok(())
    }

    #[test]
    fn w6_transform_axes_text_sync_and_heatmap_spawn() -> Result<(), String> {
        use crate::dialogs_canvas::AxisTarget;
        use curvebook_doc::{TableKind, TransformAnalysisSpec};

        let (mut app, ds_id, gs_id) = make_9col_mv_app()?;
        let vp = ViewportMetrics::default();

        // 1. Transform SelectAndTransform text sync & commit.
        let sel = app
            .execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "select_and_transform" }))
            .map_err(|e| e.to_string())?;
        let sel_id = SheetId::from(sel.payload["sheet_id"].as_str().ok_or("missing select sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &sel_id));
        if let Some(d) = &mut app.canvas_dialogs.edit_params.draft {
            d.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                filter_formula: None,
                target_archetype: TableKind::Grouped,
                derived_columns: Vec::new(),
            }));
        }
        let col2_title = match app.document().data_sheet(&ds_id).map(|ds| &ds.archetype) {
            Some(curvebook_doc::TableArchetype::MultipleVariables(mv)) => mv.variables[2].title.clone(),
            _ => String::from("Var 3"),
        };
        let filter_expr = format!("[{col2_title}] >= 10");
        app.focus_widget_id = Some(String::from("edit.transform.filter_formula"));
        let ev_filter = dispatch_ui_event(&mut app, UiInputEvent::TextInput { text: filter_expr.clone() }, vp)?;
        assert!(ev_filter.handled, "expected edit.transform.filter_formula TextInput handled, got {ev_filter:?}");
        commit_edit_params_action(&mut app, "ok").map_err(|e| e.to_string())?;
        let rs_sel = app.document().result_sheet(&sel_id).ok_or("missing sel rs")?;
        assert!(matches!(
            &rs_sel.analysis.kind,
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                target_archetype: TableKind::Grouped,
                filter_formula: Some(f),
                ..
            }) if f == &filter_expr
        ));

        // 2. FormatAxes text sync & correlation heatmap spawn on commit.
        let gs_ref = app.document().graph_sheet(&gs_id).ok_or("missing gs")?.clone();
        app.canvas_dialogs.format_axes.sync_from_graph(&gs_ref, AxisTarget::Y2);
        app.canvas_dialogs.format_axes.enable_y2 = true;
        app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs_id.clone() };
        app.focus_widget_id = Some(String::from("axes.title"));
        assert!(dispatch_ui_event(&mut app, UiInputEvent::TextInput { text: String::from("Signal (AU)") }, vp)?.handled);
        app.focus_widget_id = Some(String::from("axes.y_thresholds"));
        assert!(dispatch_ui_event(&mut app, UiInputEvent::TextInput { text: String::from("50") }, vp)?.handled);
        commit_active_dialog_action(&mut app, "ok", &Value::Null).map_err(|e| e.to_string())?;
        let gs = app.document().graph_sheet(&gs_id).ok_or("missing gs")?;
        assert_eq!(gs.spec.y2_axis.as_ref().map(|a| a.title.as_str()), Some("Signal (AU)"));
        assert!(gs.spec.y_axis.thresholds.iter().any(|t| (t.value - 50.0).abs() < 1e-6));

        let corr = app
            .execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "correlation_matrix" }))
            .map_err(|e| e.to_string())?;
        let corr_id = SheetId::from(corr.payload["sheet_id"].as_str().ok_or("missing corr sheet")?);
        let graphs_before = app.document().graph_sheets.len();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &corr_id));
        app.canvas_dialogs.edit_params.correlation_heatmap = true;
        commit_edit_params_action(&mut app, "ok").map_err(|e| e.to_string())?;
        assert_eq!(app.document().graph_sheets.len(), graphs_before + 1);
        assert!(app.document().graph_sheets.iter().any(|g| {
            g.spec.primary_source_sheet_id == corr_id
                && g.spec.kind == curvebook_doc::GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::Heatmap)
        }));
        Ok(())
    }
}
