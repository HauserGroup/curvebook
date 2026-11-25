//! Desktop `egui_shell` modal button execution and dialog widget click/toggle handlers (`F6`).

use curvebook_doc::{
    AnalysisKind, BaselineOperation, CldOrdering, ColorChannelMapping, ContingencyMethod, EquationDisplayMode, ErrorCode, ErrorPropagationMode,
    FramePreset, GroupedArrangement, MathTransformPreset, MultiplicityMethod, MultivariatePlotMode, MultivariateSpec, OneWayMethod,
    ResidualCovariance, SheetId, SurvivalAnalysisSpec, TableKind, TransformAnalysisSpec,
};
use curvebook_engine::{EngineError, cmd};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::control_io::dispatch_ui_command_as_ui;
use crate::dialogs_canvas::{ActiveDialog, AnalyzeDuplicateChoice, AxisTarget, ExportFigureFormat, equation_display_mode_str};
use crate::interaction::{clear_modal_text_buffers, close_active_modal, sync_edit_params_text_buffers, sync_format_axes_text_buffers};
use crate::modal_predicates::{
    evaluate_dialog_button_for_app, is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1b_post_hoc_option_enabled,
    r1b_sphericity_correction, r2_assume_gaussian_allowed, sync_format_points_fallback_targets,
};
use crate::power_wizard::PowerWizardStage;

fn bad_input(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, msg)
}

fn disabled_err(reason: impl Into<String>) -> EngineError {
    let msg = reason.into();
    EngineError::with_code(ErrorCode::Conflict, msg.clone())
        .with_remediation("Satisfy the control precondition or select a compatible mode/sheet first.")
        .with_details(json!({ "disabled": true, "disabled_reason": msg }))
}

pub(crate) fn is_gui_only_blocked_menu_command(cmd_id: &str) -> bool {
    if matches!(
        cmd_id,
        "table.setCell"
            | "ui.screenshot"
            | "ui.event.dispatch"
            | "ui.shortcut.set"
            | "ui.shortcut.reset"
            | "ui.brush.point"
            | "app.quit"
            | "ui.result.copyMethodsReport"
            | "file.print"
    ) {
        return true;
    }
    cmd::registry().get(cmd_id).is_some_and(|spec| spec.menu_path.is_empty())
}

/// Apply a programmatic click/toggle to a dialog widget by `widget_id` (`F6`).
pub(crate) fn apply_dialog_widget_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    try_toggle_dialog_widget_click(app, wid)
}

/// Execute a modal footer button action (`ok`, `create`, `cancel`, `apply`, `next`, `back`, `preview`).
pub(crate) fn execute_dialog_button_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    if app.canvas_dialogs.active_dialog == ActiveDialog::None {
        return Err(EngineError::with_code(ErrorCode::Conflict, "no modal dialog is currently open"));
    }
    let norm = action.trim().to_ascii_lowercase();
    if norm == "cancel" {
        let prev = format!("{:?}", app.canvas_dialogs.active_dialog);
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

/// Commit the active dialog's primary action (`create`/`next` for Power Wizard, `ok` otherwise).
#[allow(dead_code)]
pub(crate) fn submit_active_dialog_ok(app: &mut UiApp) -> Result<Value, EngineError> {
    let action = match app.canvas_dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => {
            if app.canvas_dialogs.power_wizard.stage == PowerWizardStage::Results {
                "create"
            } else {
                "next"
            }
        }
        _ => "ok",
    };
    execute_dialog_button_action(app, action)
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
                    let intents = app.canvas_dialogs.change_graph_type.commit_intents(app.document());
                    commit_many_intents_opt_close(app, intents, act != "apply")
                }
                ActiveDialog::FormatAxes { .. } => {
                    sync_format_axes_text_buffers(app);
                    let close = act != "apply";
                    let res = commit_single_intent(app, app.canvas_dialogs.format_axes.commit_intent(), close)?;
                    if close {
                        clear_modal_text_buffers(app);
                    }
                    Ok(res)
                }
                ActiveDialog::PairwiseComparisons { .. } => {
                    commit_many_intents_opt_close(app, app.canvas_dialogs.pairwise.commit_intents(), act != "apply")
                }
                ActiveDialog::FormatGraph { .. } => {
                    commit_many_intents_opt_close(app, app.canvas_dialogs.format_graph.commit_intents(), act != "apply")
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
                ActiveDialog::PageLayoutComposer { .. } => commit_many_intents_opt_close(app, app.canvas_dialogs.page_layout.commit_intents(), true),
                ActiveDialog::ExportFigure { sheet_id } => {
                    let mut intent = app.canvas_dialogs.export_figure.commit_intent(&sheet_id);
                    let export_opts = &app.canvas_dialogs.export_figure;
                    if matches!(export_opts.format, ExportFigureFormat::Pdf)
                        && let Some(obj) = intent.params.as_object_mut()
                    {
                        let pdf_cs = if export_opts.color_space == "CMYK-Proof" { "cmyk" } else { "srgb" };
                        obj.insert(String::from("color_space"), json!(pdf_cs));
                    }
                    let opts_snapshot = json!({
                        "transparent_bg": export_opts.transparent_bg,
                        "color_space": export_opts.color_space,
                        "unit": export_opts.unit,
                    });
                    let mut res = commit_single_intent(app, Some(intent), true)?;
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
                    if submit && app.is_gui_only() {
                        return Err(EngineError::with_code(
                            ErrorCode::GuiOnlyViolation,
                            "submitting print jobs directly to the OS queue is blocked under --gui-only",
                        )
                        .with_remediation("Uncheck `Submit to OS print queue` to spool in-memory PDF output safely."));
                    }
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
                | ActiveDialog::Feedback
                | ActiveDialog::ImportWarnings
                | ActiveDialog::ImportProjectMode { .. }
                | ActiveDialog::CsvExportOptions(_)
                | ActiveDialog::ExportAllTablesExistingFiles { .. }
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
    let mut outcomes = Vec::new();
    if let Some(intent) = app.canvas_dialogs.format_points.build_commit_intent() {
        let out = app.execute_command(&intent.command_id, intent.params)?;
        outcomes.push(out.payload);
    }
    let scene = app.canvas_dialogs.format_points.graph_sheet_id.as_ref().and_then(|gid| app.session().render_sheet_scene(gid).ok());
    let row_label_intents = app.canvas_dialogs.format_points.build_row_label_intents_for_scene(app.document(), scene.as_ref());
    for intent in row_label_intents {
        let out = app.execute_command(&intent.command_id, intent.params)?;
        outcomes.push(out.payload);
    }
    let count = app.canvas_dialogs.format_points.selected_points.len();
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

fn commit_analyze_sheet_action(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    let tab_id_opt = app.canvas_dialogs.analyze.extracted_result_tab_id.clone();
    let src_id_opt = app.canvas_dialogs.analyze.source_sheet_id.clone();
    if let (Some(src_id), Some(tab_id)) = (src_id_opt, tab_id_opt)
        && app.document().result_sheet(&src_id).is_some_and(|rs| rs.analyzable_data.is_none())
    {
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
        let sid = SheetId::try_new(extracted_id).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e.to_string()))?;
        app.canvas_dialogs.analyze.source_sheet_id = Some(sid);
        app.canvas_dialogs.analyze.extracted_result_tab_id = None;
    }

    let from_physical_gui = params.is_null();
    let transient_force = std::mem::take(&mut app.canvas_dialogs.analyze.force_create_new_once);
    let force_new = transient_force || params.get("force_create_new").and_then(Value::as_bool).unwrap_or(false);
    if !force_new && let Some(intent) = app.canvas_dialogs.submit_analyze_or_prompt(app.session.document()) {
        let out = app.execute_command(&intent.command_id, intent.params)?;
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
    let out = app.execute_command(&intent.command_id, intent.params)?;
    app.canvas_dialogs.close_dialog();
    if let Some(new_id) = out.payload.get("sheet_id").and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        maybe_open_nonlinear_fit_params(app, new_id, from_physical_gui);
    }
    Ok(json!({ "action": "ok", "mutated": out.mutated, "payload": out.payload }))
}

fn commit_duplicate_prompt(app: &mut UiApp, existing_ids: &[SheetId], params: &Value) -> Result<Value, EngineError> {
    let from_physical_gui = params.is_null();
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
    let out = commit_single_intent(app, Some(intent), true)?;
    if let Some(new_id) = out.get("payload").and_then(|p| p.get("sheet_id")).and_then(Value::as_str) {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
        maybe_open_nonlinear_fit_params(app, new_id, from_physical_gui);
    }
    app.canvas_dialogs.analyze.duplicate_choice = crate::dialogs_canvas::DuplicateChoice::CreateNew;
    app.canvas_dialogs.analyze.duplicate_target_id = None;
    Ok(out)
}

fn commit_edit_params_action(app: &mut UiApp, action: &str) -> Result<Value, EngineError> {
    sync_edit_params_text_buffers(app);
    crate::dialogs_canvas::sync_fit_parameter_inputs(app).map_err(bad_input)?;
    let close = action == "ok";
    let doc = app.document().clone();
    let intents = app.canvas_dialogs.edit_params.commit_intents(Some(&doc));
    let mut iter = intents.into_iter();
    let primary = iter.next().ok_or_else(|| bad_input("missing analysis parameter draft"))?;
    let out = app.execute_command(&primary.command_id, primary.params)?;
    for extra in iter {
        let _ = app.execute_command(&extra.command_id, extra.params)?;
    }
    if close {
        close_active_modal(app);
        clear_modal_text_buffers(app);
    }
    Ok(json!({ "action": action, "mutated": out.mutated, "closed": close, "payload": out.payload }))
}

fn commit_command_palette_action(app: &mut UiApp) -> Result<Value, EngineError> {
    let hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    let selected = hits
        .get(app.canvas_dialogs.command_palette.selected_index)
        .or_else(|| hits.first())
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, "no matching command in palette"))?
        .clone();
    if app.is_gui_only() && is_gui_only_blocked_menu_command(selected.id) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            format!("command palette execution of `{}` is blocked under --gui-only", selected.id),
        )
        .with_remediation("Select a parameterless menu/dialog command or toggle under --gui-only."));
    }
    app.canvas_dialogs.close_dialog();
    if let Some(res) = dispatch_ui_command_as_ui(app, selected.id, &Value::Null)? {
        return Ok(json!({ "action": "ok", "command_id": selected.id, "result": res }));
    }
    let out = app.execute_command(selected.id, Value::Null)?;
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
    let mut payloads = Vec::with_capacity(intents.len());
    for i in intents {
        let out = app.execute_command(&i.command_id, i.params)?;
        payloads.push(out.payload);
    }
    if close {
        app.canvas_dialogs.close_dialog();
    }
    Ok(json!({ "action": if close { "ok" } else { "apply" }, "count": payloads.len(), "closed": close, "payloads": payloads }))
}

fn try_toggle_dialog_widget_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::EditAnalysisParameters { .. } => try_toggle_edit_params_click(app, wid),
        ActiveDialog::ImportReshape if wid == "import.subtract_blank_mean" => {
            app.grid.import_reshape.subtract_blank_mean = !app.grid.import_reshape.subtract_blank_mean;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::ConvertArchetype { .. } if wid == "convert.allow_lossy" || wid == "convert.preview_only" => {
            if wid == "convert.allow_lossy" {
                app.canvas_dialogs.convert_archetype.allow_lossy = !app.canvas_dialogs.convert_archetype.allow_lossy;
            } else {
                app.canvas_dialogs.convert_archetype.preview_only = !app.canvas_dialogs.convert_archetype.preview_only;
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::FormatGraph { .. } => try_toggle_format_graph_click(app, wid),
        ActiveDialog::InsertModelEquation { .. } => try_toggle_insert_equation_click(app, wid),
        ActiveDialog::PairwiseComparisons { .. }
            if matches!(
                wid,
                "pairwise.hide_non_significant"
                    | "pairwise.auto_add"
                    | "pairwise.cld_enabled"
                    | "pairwise.cld_uppercase"
                    | "pairwise.cld_ordering"
                    | "pairwise.display_mode"
            ) =>
        {
            if wid == "pairwise.auto_add" {
                let doc = app.document().clone();
                let added = app.canvas_dialogs.pairwise.auto_add_from_analysis(&doc);
                return Ok(Some(json!({ "clicked": wid, "added": added, "updated": true })));
            }
            let pw = &mut app.canvas_dialogs.pairwise;
            match wid {
                "pairwise.hide_non_significant" => {
                    pw.hide_non_significant = !pw.hide_non_significant;
                    pw.brackets_dirty = true;
                }
                "pairwise.cld_enabled" => pw.toggle_cld_enabled(),
                "pairwise.cld_uppercase" => pw.cld_uppercase = !pw.cld_uppercase,
                "pairwise.cld_ordering" => {
                    pw.cld_ordering = match pw.cld_ordering {
                        CldOrdering::DescendingMean => CldOrdering::AscendingMean,
                        CldOrdering::AscendingMean => CldOrdering::DescendingMean,
                    };
                }
                "pairwise.display_mode" => pw.cycle_display_mode(),
                _ => {}
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::BubblePlotInspector { .. } => try_toggle_bubble_plot_click(app, wid),
        ActiveDialog::ApplyStyleFrom { .. }
            if matches!(wid, "apply_style.clone_palette" | "apply_style.clone_frame_and_axes" | "apply_style.clone_error_bars_and_legend") =>
        {
            match wid {
                "apply_style.clone_palette" => app.canvas_dialogs.apply_style.clone_palette = !app.canvas_dialogs.apply_style.clone_palette,
                "apply_style.clone_frame_and_axes" => {
                    app.canvas_dialogs.apply_style.clone_frame_and_axes = !app.canvas_dialogs.apply_style.clone_frame_and_axes;
                }
                _ => {
                    app.canvas_dialogs.apply_style.clone_error_bars_and_legend = !app.canvas_dialogs.apply_style.clone_error_bars_and_legend;
                }
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::PageLayoutComposer { .. } if wid == "layout.align_plot_origins" => {
            app.canvas_dialogs.page_layout.align_plot_origins = !app.canvas_dialogs.page_layout.align_plot_origins;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::FormatAxes { .. } if matches!(wid, "axes.enable_y2" | "axes.target_axis") => {
            if wid == "axes.enable_y2" {
                app.canvas_dialogs.format_axes.enable_y2 = !app.canvas_dialogs.format_axes.enable_y2;
            } else {
                sync_format_axes_text_buffers(app);
                app.interaction.text_field_buffers.remove("axes.title");
                let next_target = match app.canvas_dialogs.format_axes.target_axis {
                    AxisTarget::X => "y",
                    AxisTarget::Y => "y2",
                    AxisTarget::Y2 => "x",
                };
                let doc = app.document().clone();
                let mut map = Map::new();
                map.insert(String::from("axes.target_axis"), json!(next_target));
                crate::dialogs_canvas::dialog_patch::patch_format_axes(&mut app.canvas_dialogs.format_axes, &map, Some(&doc), &app.locale_preset)
                    .map_err(disabled_err)?;
            }
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::RandomSampleRows { .. } if wid == "sample.proportional_allocation" => {
            app.canvas_dialogs.random_sample_rows.proportional_allocation = !app.canvas_dialogs.random_sample_rows.proportional_allocation;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::SimulateDistributions if wid == "simulate.use_multiple_variables" => {
            app.canvas_dialogs.simulate_distributions.use_multiple_variables = !app.canvas_dialogs.simulate_distributions.use_multiple_variables;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::Print { .. } if wid == "print.crop_marks" => {
            app.canvas_dialogs.print.job.crop_marks = !app.canvas_dialogs.print.job.crop_marks;
            Ok(Some(json!({ "clicked": wid, "updated": true })))
        }
        ActiveDialog::AnalyzeSheet { .. } => try_toggle_analyze_click(app, wid),
        _ => Ok(None),
    }
}

fn try_toggle_bubble_plot_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    let bp = &app.canvas_dialogs.bubble_plot;
    let n_cols = bp.available_columns.len().max(1);
    let mut patch = Map::new();
    match wid {
        "bubble.show_convex_hulls" => {
            app.canvas_dialogs.bubble_plot.show_convex_hulls = !app.canvas_dialogs.bubble_plot.show_convex_hulls;
            return Ok(Some(json!({ "clicked": wid, "updated": true })));
        }
        "bubble.show_confidence_ellipses" => {
            app.canvas_dialogs.bubble_plot.show_confidence_ellipses = !app.canvas_dialogs.bubble_plot.show_confidence_ellipses;
            return Ok(Some(json!({ "clicked": wid, "updated": true })));
        }
        "bubble.plot_mode" => {
            let next = match bp.plot_mode {
                MultivariatePlotMode::BubbleScatter => "categorical_summary",
                MultivariatePlotMode::CategoricalSummary { .. } => "bubble_scatter",
            };
            patch.insert(String::from("plot_mode"), json!(next));
        }
        "bubble.x_column" => {
            patch.insert(String::from("x_column"), json!((bp.x_column + 1) % n_cols));
        }
        "bubble.y_column" => {
            patch.insert(String::from("y_column"), json!((bp.y_column + 1) % n_cols));
        }
        "bubble.color_mode" => match &bp.color_channel {
            ColorChannelMapping::Constant(_) => {
                patch.insert(String::from("color_mode"), json!("categorical"));
                patch.insert(String::from("color_column"), json!(bp.label_column.unwrap_or(0)));
            }
            ColorChannelMapping::Categorical { column, .. } => {
                patch.insert(String::from("color_mode"), json!("continuous"));
                patch.insert(String::from("color_column"), json!(*column));
            }
            ColorChannelMapping::Continuous { .. } => {
                patch.insert(String::from("color_mode"), json!("constant"));
            }
        },
        "bubble.color_column" => match &bp.color_channel {
            ColorChannelMapping::Constant(_) => {
                patch.insert(String::from("color_mode"), json!("categorical"));
                patch.insert(String::from("color_column"), json!(0));
            }
            ColorChannelMapping::Categorical { column, .. } => {
                patch.insert(String::from("color_mode"), json!("categorical"));
                patch.insert(String::from("color_column"), json!((column + 1) % n_cols));
            }
            ColorChannelMapping::Continuous { column, .. } => {
                patch.insert(String::from("color_mode"), json!("continuous"));
                patch.insert(String::from("color_column"), json!((column + 1) % n_cols));
            }
        },
        _ => return Ok(None),
    }
    crate::dialogs_canvas::dialog_patch::patch_bubble_plot(&mut app.canvas_dialogs.bubble_plot, &patch).map_err(disabled_err)?;
    Ok(Some(json!({ "clicked": wid, "updated": true })))
}

fn next_math_transform_str(cur: Option<MathTransformPreset>) -> &'static str {
    match cur {
        None => "log10",
        Some(MathTransformPreset::Log10) => "ln",
        Some(MathTransformPreset::Ln) => "log2",
        Some(MathTransformPreset::Log2) => "exp10",
        Some(MathTransformPreset::Exp10) => "sqrt",
        _ => "none",
    }
}

fn next_table_kind_str(cur: TableKind) -> &'static str {
    match cur {
        TableKind::Column => "grouped",
        TableKind::Grouped => "xy",
        TableKind::Xy => "multiple_variables",
        TableKind::MultipleVariables => "contingency",
        TableKind::Contingency => "survival",
        TableKind::Survival => "parts_of_whole",
        TableKind::PartsOfWhole => "nested",
        TableKind::Nested => "column",
    }
}

fn next_baseline_op_str(cur: BaselineOperation) -> &'static str {
    match cur {
        BaselineOperation::Difference => "ratio",
        BaselineOperation::Ratio => "percent_change",
        BaselineOperation::PercentChange => "fractional_change",
        BaselineOperation::FractionalChange => "difference",
    }
}

fn try_toggle_edit_params_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    let mut patch = Map::new();
    let ep = &app.canvas_dialogs.edit_params;
    let cont_spec = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Contingency(c)) => c.clone(),
        _ => curvebook_doc::ContingencySpec::default(),
    };
    let gg_checked = ep.draft.as_ref().is_none_or(|d| {
        !matches!(
            d.kind,
            AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
        )
    });
    match wid {
        "edit.assume_gaussian" => {
            if ep.assume_gaussian {
                let gate = r2_assume_gaussian_allowed(app.document(), ep.draft.as_ref().map(|d| &d.source_sheet_id));
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason
                            .unwrap_or_else(|| String::from("Nonparametric rank tests are disabled for pre-summarized Column tables.")),
                    ));
                }
            }
            patch.insert(String::from("assume_gaussian"), json!(!ep.assume_gaussian));
        }
        "edit.paired_or_matched" => {
            if !ep.paired_or_matched {
                let gate = r1_paired_or_matched_allowed(app.document(), ep.draft.as_ref().map(|d| &d.source_sheet_id));
                if !gate.enabled {
                    return Err(disabled_err(
                        gate.disabled_reason.unwrap_or_else(|| String::from("Matched / Repeated Measures requires equal N >= 2")),
                    ));
                }
            }
            patch.insert(String::from("paired_or_matched"), json!(!ep.paired_or_matched));
        }
        "edit.two_group.paired_sign_test" => {
            let paired_gate = r1_paired_or_matched_allowed(app.document(), ep.draft.as_ref().map(|d| &d.source_sheet_id));
            if !paired_gate.enabled {
                return Err(disabled_err(
                    paired_gate.disabled_reason.unwrap_or_else(|| String::from("Paired Sign test requires equal paired replicates >= 2")),
                ));
            }
            let gauss_gate = r2_assume_gaussian_allowed(app.document(), ep.draft.as_ref().map(|d| &d.source_sheet_id));
            if !gauss_gate.enabled {
                return Err(disabled_err(
                    gauss_gate.disabled_reason.unwrap_or_else(|| String::from("Nonparametric tests are disabled for pre-summarized Column tables.")),
                ));
            }
            patch.insert(String::from("two_group_method"), json!("paired_sign_test"));
        }
        "edit.sphericity.geisser_greenhouse" | "edit.rm.assume_sphericity" => {
            let reml_active = is_reml_mixed_effects_active(ep, app.document());
            let gate = r1b_sphericity_correction(ep.paired_or_matched, reml_active);
            if !gate.enabled {
                return Err(disabled_err(gate.disabled_reason.unwrap_or_else(|| String::from("Sphericity control is disabled."))));
            }
            patch.insert(String::from("edit.sphericity.geisser_greenhouse"), json!(!gg_checked));
        }
        "edit.rm.mixed_effects_reml" => {
            let cur = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::OneWayAnova(a)) if a.mixed_effects_reml
            ) || matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::FactorialAnova(f)) if f.mixed_effects_reml
            );
            patch.insert(String::from("mixed_effects_reml"), json!(!cur));
        }
        "edit.rm.df_kenward_roger" => {
            patch.insert(String::from("df_method"), json!("kenward_roger"));
        }
        "edit.rm.df_satterthwaite" => {
            patch.insert(String::from("df_method"), json!("satterthwaite"));
        }
        "edit.post_hoc.tukey"
        | "edit.post_hoc.dunnett"
        | "edit.post_hoc.dunn"
        | "edit.post_hoc.bonferroni"
        | "edit.post_hoc.sidak"
        | "edit.post_hoc.holm_sidak"
        | "edit.post_hoc.fisher_lsd"
        | "edit.post_hoc.games_howell"
        | "edit.post_hoc.bh"
        | "edit.post_hoc.bky"
        | "edit.post_hoc.by"
        | "edit.post_hoc.none" => {
            let (raw_method, method_enum) = match wid {
                "edit.post_hoc.tukey" => ("tukey", MultiplicityMethod::TukeyHsd),
                "edit.post_hoc.dunnett" => ("dunnett", MultiplicityMethod::Dunnett { control_column: ep.control_column }),
                "edit.post_hoc.dunn" => ("dunn", MultiplicityMethod::DunnRankSum),
                "edit.post_hoc.bonferroni" => ("bonferroni", MultiplicityMethod::Bonferroni),
                "edit.post_hoc.sidak" => ("sidak", MultiplicityMethod::Sidak),
                "edit.post_hoc.holm_sidak" => ("holm_sidak", MultiplicityMethod::HolmSidak),
                "edit.post_hoc.fisher_lsd" => ("fisher_lsd", MultiplicityMethod::FisherLsd),
                "edit.post_hoc.games_howell" => ("games_howell", MultiplicityMethod::GamesHowell),
                "edit.post_hoc.bh" => ("bh", MultiplicityMethod::BenjaminiHochbergFdr),
                "edit.post_hoc.bky" => ("bky", MultiplicityMethod::TwoStageBkyFdr),
                "edit.post_hoc.by" => ("by", MultiplicityMethod::BenjaminiYekutieliFdr),
                _ => ("none", MultiplicityMethod::None),
            };
            let reml_active = is_reml_mixed_effects_active(ep, app.document());
            let gate = r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, method_enum);
            if !gate.enabled {
                return Err(disabled_err(gate.disabled_reason.unwrap_or_else(|| String::from("Post-hoc method is disabled."))));
            }
            patch.insert(String::from("multiplicity_method"), json!(raw_method));
        }
        "edit.two_group.welch" => {
            patch.insert(String::from("welch_correction"), json!(!ep.welch_correction));
        }
        "edit.two_group.graph_differences" => {
            app.canvas_dialogs.edit_params.graph_differences = !app.canvas_dialogs.edit_params.graph_differences;
            return Ok(Some(json!({ "clicked": wid, "updated": true })));
        }
        "edit.two_group.graph_ranks" => {
            app.canvas_dialogs.edit_params.graph_ranks = !app.canvas_dialogs.edit_params.graph_ranks;
            return Ok(Some(json!({ "clicked": wid, "updated": true })));
        }
        "edit.two_group.hodges_lehmann" => {
            app.canvas_dialogs.edit_params.hodges_lehmann_ci = !app.canvas_dialogs.edit_params.hodges_lehmann_ci;
            return Ok(Some(json!({ "clicked": wid, "updated": true })));
        }
        "edit.contingency.odds_ratio" => {
            patch.insert(String::from("compute_odds_and_risk_ratios"), json!(!ep.contingency_odds_ratio_checked));
        }
        "edit.contingency.attributable_risk" => {
            patch.insert(String::from("attributable_risk"), json!(!ep.contingency_attributable_risk_checked));
        }
        "edit.contingency.sensitivity_specificity" => {
            patch.insert(String::from("sensitivity_specificity"), json!(!cont_spec.compute_sensitivity_specificity));
        }
        "edit.contingency.fisher" => {
            patch.insert(String::from("fisher"), json!(cont_spec.method != ContingencyMethod::FisherExact));
        }
        "edit.contingency.mcnemar" => {
            patch.insert(String::from("mcnemar"), json!(cont_spec.method != ContingencyMethod::McNemarPaired));
        }
        "edit.contingency.cochran_armitage" => {
            patch.insert(String::from("cochran_armitage"), json!(true));
        }
        "edit.contingency.fleiss_kappa" => {
            patch.insert(String::from("contingency_method"), json!("fleiss_kappa"));
        }
        "edit.contingency.exact_binomial" => {
            patch.insert(String::from("contingency_method"), json!("exact_binomial"));
        }
        "edit.contingency.barnard_exact" => {
            patch.insert(String::from("contingency_method"), json!("barnard_exact"));
        }
        "edit.contingency.likelihood_g2" => {
            patch.insert(String::from("contingency_method"), json!("likelihood_ratio_g2"));
        }
        "edit.factorial.three_way" => {
            let cur = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::FactorialAnova(f)) if f.three_way);
            patch.insert(String::from("three_way"), json!(!cur));
        }
        "edit.factorial.residual_covariance" => {
            let cur_unstructured = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::FactorialAnova(f)) if f.residual_covariance == ResidualCovariance::Unstructured
            );
            let next = if cur_unstructured { "compound_symmetry" } else { "unstructured" };
            patch.insert(String::from("residual_covariance"), json!(next));
        }
        "edit.pca.standardize" => {
            let cur = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, .. })));
            patch.insert(String::from("standardize"), json!(!cur));
        }
        "edit.correlation.spearman_rank" => {
            let cur = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true, .. }))
            );
            patch.insert(String::from("spearman_rank"), json!(!cur));
        }
        "edit.glm.compute_vif" => {
            let cur = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::GeneralizedLinearModel(g)) if g.compute_vif);
            patch.insert(String::from("compute_vif"), json!(!cur));
        }
        "edit.km.trend_test" => {
            let cur = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { trend_test: true, .. }))
            );
            patch.insert(String::from("trend_test"), json!(!cur));
        }
        "edit.sample.proportional_allocation" => {
            let cur = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { proportional_allocation: true, .. }))
            );
            patch.insert(String::from("proportional_allocation"), json!(!cur));
        }
        "edit.correlation.heatmap" => {
            patch.insert(String::from("correlation_heatmap"), json!(!ep.correlation_heatmap));
        }
        "edit.cluster.k_means_auto" => {
            let cur_auto =
                matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: None, .. })));
            patch.insert(String::from("k_means_auto"), json!(!cur_auto));
        }
        "edit.transform.x_transform" => {
            let cur = match ep.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, .. })) => *x_transform,
                _ => None,
            };
            patch.insert(String::from("x_transform"), json!(next_math_transform_str(cur)));
        }
        "edit.transform.y_transform" => {
            let cur = match ep.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. })) => *y_transform,
                _ => None,
            };
            patch.insert(String::from("y_transform"), json!(next_math_transform_str(cur)));
        }
        "edit.transform.target_archetype" => {
            let cur = match ep.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, .. })) => *target_archetype,
                _ => TableKind::Column,
            };
            patch.insert(String::from("target_archetype"), json!(next_table_kind_str(cur)));
        }
        "edit.transform.baseline_operation" => {
            let cur = match ep.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation, .. })) => *operation,
                _ => BaselineOperation::Difference,
            };
            patch.insert(String::from("baseline_operation"), json!(next_baseline_op_str(cur)));
        }
        "edit.transform.error_propagation" => {
            let cur_endpoints = matches!(
                ep.draft.as_ref().map(|d| &d.kind),
                Some(AnalysisKind::Transform(TransformAnalysisSpec::Math { error_propagation: ErrorPropagationMode::ReplicateEndpoints, .. }))
            );
            let next = if cur_endpoints { "delta" } else { "endpoints" };
            patch.insert(String::from("error_propagation"), json!(next));
        }
        _ => {
            if let Some(col_raw) = wid.strip_prefix("edit.pca.pred.")
                && let Ok(col) = col_raw.parse::<usize>()
            {
                app.canvas_dialogs.edit_params.toggle_pca_predictor(col);
                return Ok(Some(json!({ "clicked": wid, "updated": true })));
            }
            if let Some(col_raw) = wid.strip_prefix("edit.pcr.outcome.")
                && let Ok(col) = col_raw.parse::<usize>()
            {
                app.canvas_dialogs.edit_params.select_pcr_outcome(col).map_err(disabled_err)?;
                return Ok(Some(json!({ "clicked": wid, "updated": true })));
            }
            if let Some(col_raw) = wid.strip_prefix("edit.two_group.group.")
                && let Ok(col) = col_raw.parse::<usize>()
            {
                patch.insert(String::from("mv_group_column"), json!(col));
            } else if let Some(col_raw) = wid.strip_prefix("edit.two_group.resp.")
                && let Ok(col) = col_raw.parse::<usize>()
            {
                patch.insert(String::from("mv_response_column"), json!(col));
            } else if let Some(col_raw) = wid.strip_prefix("edit.fac.resp.")
                && let Ok(col) = col_raw.parse::<usize>()
            {
                patch.insert(String::from("response_column"), json!(col));
            } else if let Some(col_raw) = wid.strip_prefix("edit.fac.col.")
                && col_raw.parse::<usize>().is_ok()
            {
                patch.insert(wid.to_owned(), Value::Null);
            } else {
                return Ok(None);
            }
        }
    }
    crate::dialogs_canvas::dialog_patch::patch_edit_params(&mut app.canvas_dialogs.edit_params, &patch, &app.locale_preset).map_err(disabled_err)?;
    Ok(Some(json!({ "clicked": wid, "updated": true })))
}

fn try_toggle_format_graph_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    match wid {
        "format_graph.grouped_arrangement" => {
            app.canvas_dialogs.format_graph.grouped_arrangement = match app.canvas_dialogs.format_graph.grouped_arrangement {
                GroupedArrangement::Interleaved => GroupedArrangement::Stacked,
                GroupedArrangement::Stacked => GroupedArrangement::Superimposed,
                GroupedArrangement::Superimposed => GroupedArrangement::Separated,
                GroupedArrangement::Separated => GroupedArrangement::Interleaved,
            };
        }
        "format_graph.frame_preset" => {
            app.canvas_dialogs.format_graph.frame_preset = match app.canvas_dialogs.format_graph.frame_preset {
                FramePreset::OpenL => FramePreset::OffsetRangeTufte,
                FramePreset::OffsetRangeTufte => FramePreset::PlainBoxed,
                FramePreset::PlainBoxed => FramePreset::DualYSecondaryRight,
                FramePreset::DualYSecondaryRight => FramePreset::OriginCrossing,
                FramePreset::OriginCrossing => FramePreset::OpenL,
            };
        }
        "format_graph.show_legend" => app.canvas_dialogs.format_graph.show_legend = !app.canvas_dialogs.format_graph.show_legend,
        "format_graph.legend_bordered" => app.canvas_dialogs.format_graph.legend_bordered = !app.canvas_dialogs.format_graph.legend_bordered,
        "format_graph.show_raw_data" => app.canvas_dialogs.format_graph.show_raw_data = !app.canvas_dialogs.format_graph.show_raw_data,
        "format_graph.show_fitted_curve" => app.canvas_dialogs.format_graph.show_fitted_curve = !app.canvas_dialogs.format_graph.show_fitted_curve,
        "format_graph.apply_globally" | "format_graph.apply_all_data_sets" => {
            app.canvas_dialogs.format_graph.apply_all_data_sets = !app.canvas_dialogs.format_graph.apply_all_data_sets;
        }
        "format_graph.half_violin_with_points" => {
            app.canvas_dialogs.format_graph.half_violin_with_points = !app.canvas_dialogs.format_graph.half_violin_with_points;
        }
        "format_graph.error_bar_enabled" => app.canvas_dialogs.format_graph.error_bar_enabled = !app.canvas_dialogs.format_graph.error_bar_enabled,
        "format_graph.toggle_model_equation_mode" => {
            let Some(cur) = app.canvas_dialogs.format_graph.model_equation_mode else {
                return Err(disabled_err("Graph has no ModelEquation annotation."));
            };
            let next = if cur == EquationDisplayMode::Substituted { EquationDisplayMode::Symbolic } else { EquationDisplayMode::Substituted };
            app.canvas_dialogs.format_graph.set_model_equation_mode(next);
            return Ok(Some(json!({ "clicked": wid, "mode": equation_display_mode_str(next), "updated": true })));
        }
        "format_graph.selected_level" => app.canvas_dialogs.format_graph.cycle_selected_mv_level(),
        "format_graph.move_to_top" | "format_graph.move_up" | "format_graph.move_down" | "format_graph.move_to_end" => {
            let op = wid.strip_prefix("format_graph.").unwrap_or(wid);
            let _ = app.canvas_dialogs.format_graph.reorder_mv_level(op).map_err(disabled_err)?;
            let fg = &app.canvas_dialogs.format_graph;
            let len = fg.mv_levels.len();
            let new_idx = crate::dialogs_canvas::resolve_selected_level_idx(&fg.mv_levels, fg.mv_selected_level.as_deref());
            if len > 1 {
                match (app.focus_widget_id.as_deref(), new_idx) {
                    (Some("format_graph.move_to_top"), Some(0)) => app.focus_widget_id = Some(String::from("format_graph.move_to_end")),
                    (Some("format_graph.move_up"), Some(0)) => app.focus_widget_id = Some(String::from("format_graph.move_down")),
                    (Some("format_graph.move_down"), Some(i)) if i + 1 >= len => {
                        app.focus_widget_id = Some(String::from("format_graph.move_up"));
                    }
                    (Some("format_graph.move_to_end"), Some(i)) if i + 1 >= len => {
                        app.focus_widget_id = Some(String::from("format_graph.move_to_top"));
                    }
                    _ => {}
                }
            }
        }
        _ => return Ok(None),
    }
    Ok(Some(json!({ "clicked": wid, "updated": true })))
}

fn try_toggle_insert_equation_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    match wid {
        "equation.toggle_mode" | "insert_equation.toggle_mode" => {
            let next = app.canvas_dialogs.insert_model_equation.toggle_mode();
            return Ok(Some(json!({ "clicked": wid, "mode": equation_display_mode_str(next), "updated": true })));
        }
        "equation.bordered" => {
            app.canvas_dialogs.insert_model_equation.bordered = !app.canvas_dialogs.insert_model_equation.bordered;
        }
        "equation.update_existing" => {
            app.canvas_dialogs.insert_model_equation.update_existing = !app.canvas_dialogs.insert_model_equation.update_existing;
        }
        _ => return Ok(None),
    }
    Ok(Some(json!({ "clicked": wid, "updated": true })))
}

fn try_toggle_analyze_click(app: &mut UiApp, wid: &str) -> Result<Option<Value>, EngineError> {
    let mut patch = Map::new();
    let an = &app.canvas_dialogs.analyze;
    match wid {
        "analyze.correlation_all_pairs" => patch.insert(String::from("correlation_all_pairs"), json!(!an.correlation_all_pairs)),
        "analyze.correlation.heatmap" => patch.insert(String::from("correlation_heatmap"), json!(!an.correlation_heatmap)),
        "analyze.factorial.three_way" => patch.insert(String::from("three_way"), json!(!an.factorial_three_way)),
        "analyze.factorial.residual_covariance" => {
            let next = if an.factorial_residual_covariance == ResidualCovariance::Unstructured { "compound_symmetry" } else { "unstructured" };
            patch.insert(String::from("residual_covariance"), json!(next))
        }
        "analyze.pca.standardize" => patch.insert(String::from("standardize"), json!(!an.pca_standardize)),
        "analyze.correlation.spearman_rank" => patch.insert(String::from("spearman_rank"), json!(!an.corr_spearman_rank)),
        "analyze.glm.compute_vif" => patch.insert(String::from("compute_vif"), json!(!an.glm_compute_vif)),
        "analyze.sample.proportional_allocation" => patch.insert(String::from("proportional_allocation"), json!(!an.sample_proportional)),
        _ => return Ok(None),
    };
    crate::dialogs_canvas::dialog_patch::patch_analyze(&mut app.canvas_dialogs.analyze, &patch, &app.locale_preset).map_err(disabled_err)?;
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
    fn w6_transform_text_sync_and_cycling() -> Result<(), String> {
        let (mut app, ds_id, _gs_id) = make_9col_mv_app()?;
        let vp = ViewportMetrics::default();

        let math = app
            .execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "math", "y_transform": "log10" }))
            .map_err(|e| e.to_string())?;
        let math_id = SheetId::from(math.payload["sheet_id"].as_str().ok_or("missing math sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &math_id));
        try_toggle_edit_params_click(&mut app, "edit.transform.x_transform").map_err(|e| e.to_string())?;
        try_toggle_edit_params_click(&mut app, "edit.transform.y_transform").map_err(|e| e.to_string())?;
        assert!(matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: Some(MathTransformPreset::Log10),
                y_transform: Some(MathTransformPreset::Ln),
                ..
            }))
        ));
        app.canvas_dialogs.close_dialog();

        let sel = app
            .execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "select_and_transform" }))
            .map_err(|e| e.to_string())?;
        let sel_id = SheetId::from(sel.payload["sheet_id"].as_str().ok_or("missing select sheet")?);
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &sel_id));
        if let Some(d) = &mut app.canvas_dialogs.edit_params.draft {
            d.update_kind(AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                filter_formula: None,
                target_archetype: TableKind::Column,
                derived_columns: Vec::new(),
            }));
        }
        try_toggle_edit_params_click(&mut app, "edit.transform.target_archetype").map_err(|e| e.to_string())?;
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
        assert!(
            matches!(
                &rs_sel.analysis.kind,
                AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
                    target_archetype: TableKind::Grouped,
                    filter_formula: Some(f),
                    ..
                }) if f == &filter_expr
            ),
            "unexpected select_and_transform kind after commit: {:?}",
            rs_sel.analysis.kind
        );
        Ok(())
    }

    #[test]
    fn w6_axes_bubble_and_heatmap_spawn() -> Result<(), String> {
        let (mut app, ds_id, gs_id) = make_9col_mv_app()?;
        let vp = ViewportMetrics::default();

        let gs_ref = app.document().graph_sheet(&gs_id).ok_or("missing gs")?.clone();
        app.canvas_dialogs.format_axes.sync_from_graph(&gs_ref, AxisTarget::Y);
        app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs_id.clone() };
        try_toggle_dialog_widget_click(&mut app, "axes.target_axis").map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.format_axes.target_axis, AxisTarget::Y2);
        try_toggle_dialog_widget_click(&mut app, "axes.enable_y2").map_err(|e| e.to_string())?;
        assert!(app.canvas_dialogs.format_axes.enable_y2);
        app.focus_widget_id = Some(String::from("axes.title"));
        let ev_title = dispatch_ui_event(&mut app, UiInputEvent::TextInput { text: String::from("Signal (AU)") }, vp)?;
        assert!(ev_title.handled, "expected axes.title TextInput handled, got {ev_title:?}");
        app.focus_widget_id = Some(String::from("axes.y_thresholds"));
        let ev_thresh = dispatch_ui_event(&mut app, UiInputEvent::TextInput { text: String::from("50") }, vp)?;
        assert!(ev_thresh.handled, "expected axes.y_thresholds TextInput handled, got {ev_thresh:?}");
        commit_active_dialog_action(&mut app, "ok", &Value::Null).map_err(|e| e.to_string())?;
        let gs = app.document().graph_sheet(&gs_id).ok_or("missing gs")?;
        assert_eq!(gs.spec.y2_axis.as_ref().map(|a| a.title.as_str()), Some("Signal (AU)"));
        assert!(gs.spec.y_axis.thresholds.iter().any(|t| (t.value - 50.0).abs() < 1e-6));

        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_bubble_plot(&doc, &gs_id));
        try_toggle_dialog_widget_click(&mut app, "bubble.plot_mode").map_err(|e| e.to_string())?;
        assert!(matches!(app.canvas_dialogs.bubble_plot.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        try_toggle_dialog_widget_click(&mut app, "bubble.color_mode").map_err(|e| e.to_string())?;
        try_toggle_dialog_widget_click(&mut app, "bubble.color_column").map_err(|e| e.to_string())?;
        assert!(matches!(app.canvas_dialogs.bubble_plot.color_channel, ColorChannelMapping::Categorical { column: 1, .. }));
        app.canvas_dialogs.close_dialog();

        let corr = app
            .execute_command("analysis.run", json!({ "source_sheet_id": ds_id.as_str(), "kind": "correlation_matrix" }))
            .map_err(|e| e.to_string())?;
        let corr_id = SheetId::from(corr.payload["sheet_id"].as_str().ok_or("missing corr sheet")?);
        let graphs_before = app.document().graph_sheets.len();
        let doc = app.document().clone();
        assert!(app.canvas_dialogs.open_edit_parameters(&doc, &corr_id));
        try_toggle_edit_params_click(&mut app, "edit.correlation.heatmap").map_err(|e| e.to_string())?;
        commit_edit_params_action(&mut app, "ok").map_err(|e| e.to_string())?;
        assert_eq!(app.document().graph_sheets.len(), graphs_before + 1);
        assert!(app.document().graph_sheets.iter().any(|g| {
            g.spec.primary_source_sheet_id == corr_id
                && g.spec.kind == curvebook_doc::GraphKind::Diagnostic(curvebook_doc::DiagnosticPlotKind::Heatmap)
        }));
        Ok(())
    }
}
