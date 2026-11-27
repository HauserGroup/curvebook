//! Private intent execution helpers dispatched on settled [`super::PendingUiIntents`] batches.

use curvebook_doc::{CellCoord, ErrorCode, SheetId};
use curvebook_engine::EngineError;

use super::super::layout::LayoutMetrics;
use super::super::{import_wizard, power_studio};
use super::{UiIntent, UiIntentOutcome};
use crate::UiApp;
use crate::control_io::dispatch_ui_command_as_ui;
use crate::dialogs_canvas::{ActiveDialog, ActiveInspector, AnalyzeDuplicateChoice};
use crate::grid_view::ImportOriginKind;

pub(super) fn apply_single_ui_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::EngineCommand { command_id, mut params } => {
            if command_id == "project.open" && app.session.is_dirty() && params.get("force").and_then(serde_json::Value::as_bool) != Some(true) {
                return Err(EngineError::with_code(ErrorCode::Conflict, "unsaved changes exist; save or discard before opening another project"));
            }
            if matches!(command_id.as_str(), "sheet.duplicate" | "sheet.duplicateFamily" | "sheet.delete" | "sheet.freeze" | "sheet.unfreeze")
                && params.get("sheet_id").is_none()
                && let Some(active_id) = app.session.document().active_sheet_id.as_ref()
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("sheet_id"), serde_json::Value::String(active_id.as_str().to_owned()));
            }
            let prev_active_id = app.session.document().active_sheet_id.clone();
            let select_mode_is_family = command_id == "sheet.select" && params.get("mode").and_then(serde_json::Value::as_str) == Some("family");
            let outcome = app.execute_session_ui_cmd(&command_id, params)?;
            if matches!(command_id.as_str(), "edit.undo" | "edit.redo" | "sheet.select") {
                crate::interaction::clear_inspector_axis_buffers(app);
            }
            if command_id == "sheet.select"
                && let Some(active_id) = app.session.document().active_sheet_id.clone()
            {
                let doc = app.session.document().clone();
                if app.split.enabled {
                    if select_mode_is_family {
                        app.split.sync_pin_invariants(&doc);
                    } else {
                        let last_sec = app.egui_views.center_stage.focused_split_pane == crate::egui_shell::center_stage::SplitPaneSlot::Secondary;
                        app.split.on_navigator_select(&doc, prev_active_id.as_ref(), &active_id, last_sec);
                    }
                } else {
                    app.split.sync_pin_invariants(&doc);
                }
                app.canvas_dialogs.on_sheet_selected_with_auto_modal(&doc, &active_id, false);
            } else if command_id == "sheet.delete" {
                let doc = app.session.document().clone();
                app.split.sync_pin_invariants(&doc);
            }
            Ok(UiIntentOutcome::Engine { command_id, outcome })
        }
        UiIntent::EngineCommandGroup { commands } => {
            let last = app.execute_atomic_group(|app| {
                let mut last: Option<(String, curvebook_engine::CommandOutcome)> = None;
                for (command_id, params) in commands {
                    let outcome = app.execute_session_ui_cmd(&command_id, params)?;
                    last = Some((command_id, outcome));
                }
                Ok::<_, EngineError>(last)
            })?;
            Ok(match last {
                Some((command_id, outcome)) => UiIntentOutcome::Engine { command_id, outcome },
                None => UiIntentOutcome::ShellUpdated,
            })
        }
        UiIntent::UiCommand { command_id, params } => {
            if command_id == "edit.cut" {
                return apply_wave4_grid_wizard_intent(app, UiIntent::GridCutSelection);
            }
            if command_id == "edit.copy" {
                return apply_wave4_grid_wizard_intent(app, UiIntent::WorkspaceCopy { dpi: 144 });
            }
            if command_id == "edit.paste" {
                let doc = app.session.document();
                let active_id = doc.active_sheet_id.clone();
                if active_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some()) {
                    return apply_wave4_grid_wizard_intent(app, UiIntent::GridPasteFromClipboard { start_coord: None });
                }
                if let Some(target_sheet_id) = crate::interaction::resolve_figure_sheet_for_shortcut(app) {
                    return apply_wave4_grid_wizard_intent(app, UiIntent::CanvasPaste { target_sheet_id, anchor_pt: None });
                }
            }
            let result = dispatch_ui_command_as_ui(app, &command_id, &params)?;
            Ok(UiIntentOutcome::Ui { command_id, result })
        }
        UiIntent::SetTheme(mode) => {
            let prev_scale = app.theme.ui_scale;
            app.set_theme_mode(mode);
            app.theme.ui_scale = prev_scale;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetLocale(locale) => {
            let _ = app.set_locale(&locale);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetUiScale(scale) => {
            let clamped = if scale.is_finite() { scale.clamp(0.75, 3.0) } else { 1.0 };
            app.theme.ui_scale = clamped;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectGridCell { sheet_id, coord } => {
            let target_sheet = sheet_id.or_else(|| app.document().active_sheet_id.clone());
            if let Some(sid) = target_sheet {
                app.grid.select_single_for_sheet(&sid, coord);
            } else {
                app.grid.select_single_sync(coord);
            }
            app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectXCell { row, subcol } => {
            app.grid.select_x_cell_sync(row, subcol);
            app.focus_widget_id = Some(format!("grid.xcell.{row}.{subcol}"));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ExtendGridSelection { sheet_id, coord } => {
            let mut grid = std::mem::take(&mut app.grid);
            let target_id = sheet_id.or_else(|| app.document().active_sheet_id.clone());
            let brushed_sheet = if let Some(target_id) = target_id
                && let Some(ds) = app.document().data_sheet(&target_id)
            {
                grid.extend_shift_range(ds, coord);
                Some(target_id)
            } else {
                None
            };
            app.grid = grid;
            if let Some(active_id) = brushed_sheet {
                app.sync_brush_from_grid_selection(&active_id);
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ToggleGridCell { sheet_id, coord } => {
            let mut grid = std::mem::take(&mut app.grid);
            let target_id = sheet_id.or_else(|| app.document().active_sheet_id.clone());
            let brushed_sheet = if let Some(target_id) = target_id
                && let Some(ds) = app.document().data_sheet(&target_id)
            {
                grid.toggle_cmd_selection(ds, coord);
                Some(target_id)
            } else {
                None
            };
            app.grid = grid;
            if let Some(active_id) = brushed_sheet {
                app.sync_brush_from_grid_selection(&active_id);
            }
            app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::GridNavigate { dir, extend, jump } => {
            let doc = app.session.document();
            if let Some(ds) = doc.active_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)) {
                app.grid.navigate(ds, dir, extend, jump);
                let cur = app.grid.active_cell_for_sheet(&ds.id).unwrap_or_default();
                if extend {
                    app.grid.extend_shift_range(ds, cur);
                } else {
                    app.grid.select_single_for_sheet(&ds.id, cur);
                }
                app.grid.refresh_selection_stats(doc);
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetRowTitlesCollapsed { collapsed } => {
            app.grid.row_titles_collapsed_override = Some(collapsed);
            app.egui_views.column_grid.row_titles_collapsed = collapsed;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::DialogWidgetClick { widget_id } => {
            let result = super::super::dialog_actions::apply_dialog_widget_click(app, &widget_id)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.widget:{widget_id}"), result })
        }
        UiIntent::DialogButtonAction { action } => {
            let result = super::super::dialog_actions::execute_dialog_button_action(app, &action)?;
            if !power_studio::is_power_studio_dialog_active(app) {
                power_studio::reset_results_transient_state(&mut app.egui_views.power_studio);
            }
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.button:{action}"), result: Some(result) })
        }
        UiIntent::SetTextFieldBuffer { widget_id, text } => {
            app.interaction.text_field_buffers.insert(widget_id.clone(), crate::interaction::TextFieldEditState::new(text, false));
            Ok(UiIntentOutcome::Ui { command_id: format!("textbox.sync:{widget_id}"), result: None })
        }
        UiIntent::OpenProjectFile => Ok(UiIntentOutcome::ShellUpdated),
        UiIntent::CopyResultTable { sheet_id, force_full } => {
            let outcome = crate::dialogs_canvas::copy_result_table_to_clipboard(app, sheet_id.as_str(), force_full);
            Ok(UiIntentOutcome::Ui { command_id: outcome.command_id.unwrap_or(outcome.action), result: outcome.command_result })
        }
        UiIntent::SetFloatingNotePosition { sheet_id, note_id, pos_pt } => {
            if !pos_pt[0].is_finite() || !pos_pt[1].is_finite() {
                return Err(EngineError::with_code(ErrorCode::Conflict, "floating note position must be finite"));
            }
            let clamped = [pos_pt[0].clamp(0.0, 2400.0) as f32, pos_pt[1].clamp(0.0, 2400.0) as f32];
            app.egui_views.center_stage.note_positions_pt.insert((sheet_id, note_id.clone()), clamped);
            app.egui_views.center_stage.focused_floating_note_id = Some(note_id);
            app.nav.active_pane = crate::workspace_nav::ActiveWorkspacePane::FloatingNotes;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ToggleFloatingNoteCollapsed { sheet_id, note_id } => {
            let doc_min = crate::workspace_nav::sheet_floating_notes(app.document(), &sheet_id)
                .iter()
                .find(|n| n.id.as_str() == note_id)
                .is_some_and(|n| n.minimized);
            let cur = app.nav.floating_notes_collapsed.get(&note_id).copied().unwrap_or(doc_min);
            let next = !cur;
            app.nav.floating_notes_collapsed.insert(note_id.clone(), next);
            app.egui_views.center_stage.note_collapsed.insert((sheet_id, note_id.clone()), next);
            app.egui_views.center_stage.focused_floating_note_id = Some(note_id);
            app.nav.active_pane = crate::workspace_nav::ActiveWorkspacePane::FloatingNotes;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ClearHookPickerSearch => {
            app.canvas_dialogs.hook_picker.search_query.clear();
            app.interaction.text_field_buffers.remove("hook.search_input");
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::AnalyzePicker(action) => super::stat_params::apply_analyze_picker_intent(app, action),
        UiIntent::StatParams(action) => super::stat_params::apply_stat_params_intent(app, action),
        other => apply_shell_workspace_intent(app, other),
    }
}

fn apply_shell_workspace_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::SelectResultTab { index, sheet_id } => {
            let target_id = sheet_id.or_else(|| app.session.document().active_sheet_id.clone());
            if let Some(id) = target_id {
                let max_idx = app.session.document().result_sheet(&id).map_or(0, |rs| rs.tabs.len().saturating_sub(1));
                let clamped = index.min(max_idx);
                app.egui_views.results_view.per_sheet_active_tab.insert(id.clone(), clamped);
                if app.session.document().active_sheet_id.as_ref() == Some(&id) {
                    app.canvas_dialogs.result_view.active_tab_index = clamped;
                }
            }
        }
        UiIntent::SetResultReportOptions { format, scope } => {
            app.canvas_dialogs.result_view.report_format = format;
            app.canvas_dialogs.result_view.report_scope = scope;
        }
        UiIntent::SetActiveInspector { inspector } => {
            app.canvas_dialogs.active_inspector = inspector;
            if inspector != ActiveInspector::None {
                app.nav.inspector_open = true;
                app.egui_views.trailing_collapsed_rail = false;
            }
        }
        UiIntent::SetFocusPane { pane } => {
            app.nav.active_pane = pane;
        }
        UiIntent::SetCanvasZoom { zoom } => {
            let clamped = if zoom.is_finite() { zoom.clamp(0.1, 8.0) } else { 1.0 };
            app.canvas_dialogs.canvas.zoom = clamped;
            app.canvas_dialogs.pan_zoom.zoom = clamped;
            app.egui_views.canvas_view.fit_zoom_active = false;
            app.egui_views.center_stage.secondary_canvas_view.fit_zoom_active = false;
        }
        UiIntent::CanvasPinchZoom { scale_delta, anchor_pt } => {
            app.canvas_dialogs.pan_zoom.apply_pinch_zoom(scale_delta, anchor_pt);
            app.canvas_dialogs.canvas.zoom = app.canvas_dialogs.pan_zoom.zoom;
            app.canvas_dialogs.canvas.pan_pt =
                curvebook_chart::PointPt::new(app.canvas_dialogs.pan_zoom.pan_pt[0], app.canvas_dialogs.pan_zoom.pan_pt[1]);
            app.egui_views.canvas_view.fit_zoom_active = false;
            app.egui_views.center_stage.secondary_canvas_view.fit_zoom_active = false;
        }
        UiIntent::CanvasPanDelta { delta_x, delta_y } => {
            app.canvas_dialogs.pan_zoom.apply_pan_delta([delta_x, delta_y], 0.016);
            app.canvas_dialogs.canvas.zoom = app.canvas_dialogs.pan_zoom.zoom;
            app.canvas_dialogs.canvas.pan_pt =
                curvebook_chart::PointPt::new(app.canvas_dialogs.pan_zoom.pan_pt[0], app.canvas_dialogs.pan_zoom.pan_pt[1]);
        }
        UiIntent::SetFormulaBarInput { text } => {
            let doc = app.document().clone();
            app.grid.formula_bar.set_input(text, None, &doc);
            app.focus_widget_id = Some(String::from("formula_bar.input"));
        }
        UiIntent::CommitFormulaBar => {
            let res = crate::grid_view::commit_formula_bar(app)?;
            return Ok(UiIntentOutcome::Ui { command_id: String::from("table.setCalculatedVariable"), result: Some(res) });
        }
        UiIntent::CancelFormulaBar => {
            crate::grid_view::cancel_formula_bar(app);
        }
        UiIntent::AcceptFormulaBarCompletion => {
            let doc = app.document().clone();
            if app.grid.formula_bar.accept_completion(&doc) {
                app.focus_widget_id = Some(String::from("formula_bar.input"));
            }
        }
        UiIntent::ToggleNavigatorSection { section } => {
            if let Some(pos) = app.nav.collapsed_sections.iter().position(|s| *s == section) {
                app.nav.collapsed_sections.remove(pos);
            } else {
                app.nav.collapsed_sections.push(section);
            }
        }
        UiIntent::SetNavigatorSearch { query } => {
            app.nav.search_query = query;
        }
        UiIntent::SetNavigatorNesting { mode } => {
            app.nav.nesting_mode = mode;
        }
        UiIntent::SetSplitRatio { ratio } => {
            app.split.set_ratio(ratio);
        }
        UiIntent::ToggleInspectorPin => {
            let doc = app.session.document().clone();
            if app.split.enabled {
                if app.split.inspector_pinned {
                    app.split.inspector_pinned = false;
                    app.split.inspector_target_sheet_id = doc.active_sheet_id.clone();
                } else {
                    let target = app.split.effective_inspector_target(&doc).cloned();
                    app.split.inspector_target_sheet_id = target;
                    app.split.inspector_pinned = app.split.inspector_target_sheet_id.is_some();
                }
            } else {
                app.split.inspector_pinned = false;
                app.split.inspector_target_sheet_id = doc.active_sheet_id.clone();
            }
        }
        UiIntent::SwitchInspectorTarget { sheet_id } => {
            let doc = app.session.document().clone();
            if doc.sheet_section(&sheet_id).is_some() {
                app.split.inspector_target_sheet_id = Some(sheet_id);
                if app.split.enabled {
                    app.split.sync_pin_invariants(&doc);
                }
            }
        }
        UiIntent::SetPreviewFilter { filter } => {
            app.preview_filter = filter;
        }
        UiIntent::SetPaletteOpen { open } => {
            if open {
                app.canvas_dialogs.open_command_palette();
                app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
                app.egui_views.command_palette.just_opened = true;
            } else if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette) {
                app.canvas_dialogs.active_dialog = ActiveDialog::None;
            }
        }
        UiIntent::OpenCheckpointsDialog => {
            let doc = app.session.document().clone();
            app.canvas_dialogs.open_checkpoints(&doc);
        }
        UiIntent::SetLauncherOpen { open } => {
            app.egui_views.launcher_open = open;
            app.nav.welcome.open = open;
        }
        UiIntent::ApplyWorkspacePreset { preset } => {
            let cfg = preset.default_dock_config();
            app.nav.navigator_open = cfg.navigator_open;
            app.family_open = cfg.family_open;
            app.nav.inspector_open = cfg.inspector_open;
            app.egui_views.workspace_preset = preset;
            app.egui_views.leading_collapsed_rail = cfg.leading_collapsed_rail;
            app.egui_views.trailing_collapsed_rail = cfg.trailing_collapsed_rail;
            app.egui_views.leading_width_pt = Some(cfg.leading_width_pt);
            app.egui_views.trailing_width_pt = Some(cfg.trailing_width_pt);
        }
        UiIntent::SyncLeadingWidth { width_pt } => {
            let clamped = width_pt.clamp(LayoutMetrics::CANONICAL.navigator_min_w, LayoutMetrics::CANONICAL.navigator_max_w);
            app.egui_views.leading_width_pt = Some(clamped);
        }
        UiIntent::SyncTrailingWidth { width_pt } => {
            let clamped = width_pt.clamp(LayoutMetrics::CANONICAL.inspector_min_w, LayoutMetrics::CANONICAL.inspector_max_w);
            app.egui_views.trailing_width_pt = Some(clamped);
        }
        UiIntent::ToggleLeadingCollapsedRail => {
            app.egui_views.leading_collapsed_rail = !app.egui_views.leading_collapsed_rail;
            if !app.egui_views.leading_collapsed_rail {
                app.nav.navigator_open = true;
            }
        }
        UiIntent::ToggleTrailingCollapsedRail => {
            app.egui_views.trailing_collapsed_rail = !app.egui_views.trailing_collapsed_rail;
            if !app.egui_views.trailing_collapsed_rail {
                app.nav.inspector_open = true;
            }
        }
        UiIntent::HistoryBack => return step_history_intent(app, false),
        UiIntent::HistoryForward => return step_history_intent(app, true),
        UiIntent::DismissDiagnosticBanner { sheet_id } => {
            if !app.egui_views.center_stage.dismissed_banner_sheets.contains(&sheet_id) {
                app.egui_views.center_stage.dismissed_banner_sheets.push(sheet_id);
            }
        }
        UiIntent::DismissFigureExportWarning => {
            app.canvas_dialogs.figure_export_warning = None;
        }
        UiIntent::DismissStatusBanner => {
            app.canvas_dialogs.status_banner = None;
        }
        UiIntent::OpenRetargetDialog { result_sheet_id, target_data_sheet_id } => {
            let doc = app.session.document();
            if let Some(rs) = doc.result_sheet(&result_sheet_id) {
                app.canvas_dialogs.change_data.open_for_result(rs, doc);
                if let Some(target) = target_data_sheet_id {
                    app.canvas_dialogs.change_data.selected_target_id = Some(target);
                }
                app.canvas_dialogs.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id };
            }
        }
        UiIntent::BrushPoint { sheet_id, coord } => {
            app.brush_from_grid_cell(&sheet_id, coord);
        }
        UiIntent::AddBrushedPoint { sheet_id, coord } => {
            let source = curvebook_chart::SourceCellId { sheet_id, row: coord.row, col: coord.col, subcol: coord.subcol };
            if !app.brushed_points.contains(&source) {
                app.brushed_points.push(source.clone());
            }
            if app.brushed_cell.is_none() {
                app.brushed_cell = Some(source);
            }
        }
        UiIntent::ClearBrush => {
            app.clear_brush();
        }
        other => return apply_wave4_intent(app, other),
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn apply_wave4_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::CommitDialogAction { action } => {
            let result = super::super::dialog_actions::execute_dialog_button_action(app, &action)?;
            Ok(UiIntentOutcome::Ui { command_id: format!("dialog.button:{action}"), result: Some(result) })
        }
        UiIntent::CloseAllModals => {
            app.import_warnings.panel_open = false;
            app.canvas_dialogs.close_dialog();
            app.grid.variables_dialog.open = false;
            app.grid.import_wizard.open = false;
            app.egui_views.transform_modals.open_kind = None;
            app.egui_views.app_modals.close_all();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetRetargetSelectedDatasets { cols } => {
            app.canvas_dialogs.change_data.selected_data_sets = cols;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::UpdateImportReshapeState(state) => {
            app.grid.import_reshape = *state;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetImportReshapePlateValuesText { text, ensure_default_roles } => {
            let parsed = app
                .grid
                .import_reshape
                .parse_plate_values_text(&text, &app.locale_preset)
                .map_err(|msg| EngineError::with_code(ErrorCode::BadInput, msg))?;
            app.grid.import_reshape.plate_values = parsed;
            if ensure_default_roles {
                let is_384 = app.grid.import_reshape.mode == crate::grid_view::ImportReshapeMode::Plate384;
                let expected = if is_384 { 384 } else { 96 };
                if app.grid.import_reshape.plate_roles.len() != expected {
                    app.grid.import_reshape.plate_roles = import_wizard::default_plate_roles(is_384, app.grid.import_reshape.subtract_blank_mean);
                }
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetConvertArchetypeTarget { target_kind } => {
            let mut working = app.canvas_dialogs.convert_archetype.clone();
            let prev_target = working.target_kind;
            working.set_target_kind(target_kind);
            if (working.target_kind != prev_target || !working.preview_ready)
                && let Some(intent) = working.preview_intent()
            {
                let out = app.execute_session_ui_cmd(&intent.command_id, intent.params)?;
                working.apply_preview_payload(&out.payload);
            }
            app.canvas_dialogs.convert_archetype = working;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetExcludeWithReason { reason_tag, note } => {
            if let Some(tag) = reason_tag {
                app.canvas_dialogs.exclude_with_reason.reason_tag = tag;
            }
            if let Some(n) = note {
                app.canvas_dialogs.exclude_with_reason.note = n;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::UpdateFormatSelectedPointsState(state) => {
            app.canvas_dialogs.format_points = *state;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::UpdateInsertModelEquationState(state) => {
            app.canvas_dialogs.insert_model_equation = *state;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SwitchFormatAxesTarget { target } => {
            let doc = app.document().clone();
            app.canvas_dialogs.format_axes.switch_target_axis_preserving_thresholds(target, Some(&doc));
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetCommandPaletteQuery { query } => {
            app.canvas_dialogs.command_palette.query.clone_from(&query);
            #[cfg(not(target_arch = "wasm32"))]
            {
                app.egui_views.command_palette.query = query;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetHookPickerSearch { query } => {
            app.canvas_dialogs.hook_picker.search_query = query;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SetCheckpointsDraft { name, note } => {
            if let Some(n) = name {
                app.canvas_dialogs.checkpoints.name_draft = n;
            }
            if let Some(nt) = note {
                app.canvas_dialogs.checkpoints.note_draft = nt;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::OpenManageVariables { .. }
        | UiIntent::SelectManageVarColumn { .. }
        | UiIntent::StageManageVarRecode { .. }
        | UiIntent::ClearManageVarRecodes
        | UiIntent::ApplyManageVarRecodes { .. }
        | UiIntent::ApplyManageVarOrder { .. }
        | UiIntent::CloseManageVariables
        | UiIntent::CancelManageVariables { .. }
        | UiIntent::OpenImportWizard { .. }
        | UiIntent::UpdateImportWizard(_)
        | UiIntent::PasteClipboardIntoImportReshape
        | UiIntent::CommitImportWizard { .. }
        | UiIntent::CloseImportWizard
        | UiIntent::SavePlateMapFile { .. }
        | UiIntent::LoadPlateMapFile { .. }
        | UiIntent::LoadPlateLayoutFromSheet { .. }
        | UiIntent::ApplyPlateGridsText { .. }
        | UiIntent::OpenTransformModal { .. }
        | UiIntent::CloseTransformModal
        | UiIntent::GridCopySelection
        | UiIntent::GridCutSelection
        | UiIntent::GridPasteFromClipboard { .. }
        | UiIntent::PasteXCellText { .. }
        | UiIntent::WorkspaceCopy { .. }
        | UiIntent::WorkspaceCopyForSlides { .. }
        | UiIntent::CanvasPaste { .. }
        | UiIntent::PasteImageFromClipboard { .. } => apply_wave4_grid_wizard_intent(app, intent),
        UiIntent::SetAnalyzeQuery { .. }
        | UiIntent::SelectAnalyzeEntry { .. }
        | UiIntent::ToggleAnalyzeDataset { .. }
        | UiIntent::SetAnalyzeSelectedDatasets { .. }
        | UiIntent::SetEditParamsDraft { .. }
        | UiIntent::SetEditParamsActiveTab { .. }
        | UiIntent::SetEditParamsPairedOrMatched { .. }
        | UiIntent::SetEditParamsConfidence { .. }
        | UiIntent::SaveEditParamsAsDefault
        | UiIntent::SelectRetargetSource { .. }
        | UiIntent::ResolveDuplicateAnalysis { .. }
        | UiIntent::SelectHookConstant { .. }
        | UiIntent::UnlinkHookConstant
        | UiIntent::UpdatePowerWizardState(_)
        | UiIntent::CommitPowerWizard { .. }
        | UiIntent::UpdateChangeGraphTypeState(_)
        | UiIntent::UpdateFormatGraphState(_)
        | UiIntent::UpdateFormatAxesState(_)
        | UiIntent::UpdateBubblePlotState(_)
        | UiIntent::UpdatePairwiseState(_)
        | UiIntent::UpdatePageLayoutState(_)
        | UiIntent::UpdateExportFigureState(_)
        | UiIntent::UpdatePrintDialogState(_)
        | UiIntent::UpdateCreateGraphState(_)
        | UiIntent::UpdateEqualizeScalingState(_)
        | UiIntent::UpdateFormatEmbeddedTableState(_)
        | UiIntent::UpdateFormatImageState(_)
        | UiIntent::UpdateApplyStyleState(_)
        | UiIntent::UpdateRandomizeGroupsState(_)
        | UiIntent::UpdateRandomSampleRowsState(_)
        | UiIntent::UpdateSimulateDistributionsState(_)
        | UiIntent::SyncResultSelection { .. }
        | UiIntent::SetGallerySelection { .. }
        | UiIntent::SetGalleryViewOptions { .. }
        | UiIntent::OpenExportFigure { .. }
        | UiIntent::OpenPrintForSheets { .. }
        | UiIntent::OpenCsvExportForSheets { .. }
        | UiIntent::PlaceSelectedOntoNewLayout { .. } => apply_wave4_modal_state_intent(app, intent),
        UiIntent::OpenPreferences { .. }
        | UiIntent::ClosePreferences { .. }
        | UiIntent::SetShortcutBinding { .. }
        | UiIntent::ResetShortcutBinding { .. }
        | UiIntent::UpdatePreferencesDraft(_)
        | UiIntent::ResolveUnsavedChanges { .. }
        | UiIntent::OpenFindReplace { .. }
        | UiIntent::CloseFindReplace
        | UiIntent::ExecuteFindReplace { .. }
        | UiIntent::CopyImportWarnings
        | UiIntent::UpdateCsvExportOptions(_) => apply_wave4_app_modal_intent(app, intent),
        UiIntent::OpenPaletteWithQuery { .. }
        | UiIntent::OpenAnalyzeWithCatalog { .. }
        | UiIntent::OpenFitWithEquation { .. }
        | UiIntent::JumpToDataColumn { .. }
        | UiIntent::OpenSheetFind
        | UiIntent::CloseSheetFind
        | UiIntent::RecordSearchRecent(_) => apply_search_and_find_intent(app, intent),
        _ => Ok(UiIntentOutcome::ShellUpdated),
    }
}

fn resolve_active_or_first_data_sheet_id(doc: &curvebook_doc::ProjectDocument) -> Option<SheetId> {
    if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) {
        return Some(active.clone());
    }
    if let Some(active) = doc.active_sheet_id.as_ref() {
        if let Some(graph) = doc.graph_sheet(active)
            && doc.data_sheet(&graph.spec.primary_source_sheet_id).is_some()
        {
            return Some(graph.spec.primary_source_sheet_id.clone());
        }
        if let Some(res) = doc.result_sheet(active)
            && let Some(src) = res.analysis.source_sheet_id.as_ref()
            && doc.data_sheet(src).is_some()
        {
            return Some(src.clone());
        }
    }
    doc.data_sheets.first().map(|s| s.id.clone())
}

fn apply_search_and_find_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::OpenPaletteWithQuery { query } => {
            app.canvas_dialogs.open_command_palette();
            app.canvas_dialogs.command_palette.sync_shortcuts(&app.shortcuts);
            app.egui_views.command_palette.query = query;
            app.egui_views.command_palette.preserve_query_on_open = true;
            app.egui_views.command_palette.just_opened = true;
        }
        UiIntent::OpenAnalyzeWithCatalog { catalog_id } => {
            let doc = app.session.document().clone();
            if let Some(source_id) = resolve_active_or_first_data_sheet_id(&doc)
                && app.canvas_dialogs.open_analyze_dialog(&doc, &source_id)
            {
                app.canvas_dialogs.analyze.select_catalog_entry(&catalog_id);
            }
        }
        UiIntent::OpenFitWithEquation { equation_id } => {
            let doc = app.session.document().clone();
            if let Some(source_id) = resolve_active_or_first_data_sheet_id(&doc)
                && app.canvas_dialogs.open_analyze_dialog(&doc, &source_id)
            {
                app.canvas_dialogs.analyze.select_catalog_entry("fit.nonlinear");
                app.canvas_dialogs.analyze.equation_id = equation_id;
            }
        }
        UiIntent::JumpToDataColumn { sheet_id, col } => {
            if app.session.document().active_sheet_id.as_ref() != Some(&sheet_id) {
                let _ = app.execute_session_ui_cmd("sheet.select", serde_json::json!({ "sheet_id": sheet_id.as_str() }))?;
            }
            app.egui_views.launcher_open = false;
            app.nav.welcome.open = false;
            app.nav.gallery.open = false;
            app.egui_views.column_grid.scroll_to_column_title(col);
            let coord = curvebook_tools::GridCellCoord::new(0, col, 0);
            app.grid.select_single_for_sheet(&sheet_id, coord);
            app.focus_widget_id = Some(format!("grid.cell.0.{col}.0"));
        }
        UiIntent::OpenSheetFind => {
            let doc = app.session.document();
            let target = doc.active_sheet_id.as_ref().filter(|sid| doc.data_sheet(sid).is_some() || doc.result_sheet(sid).is_some()).cloned();
            if target.is_some() {
                app.egui_views.sheet_find.open_for_sheet(target);
            }
        }
        UiIntent::CloseSheetFind => {
            app.egui_views.sheet_find.close();
            app.egui_views.column_grid.active_find_matches = None;
            app.egui_views.results_view.active_find_matches = None;
        }
        UiIntent::RecordSearchRecent(item) => {
            app.egui_views.search_recents.record(&item);
        }
        _ => {}
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn apply_wave4_grid_wizard_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::OpenManageVariables { sheet_id, column } => {
            app.egui_views.manage_vars_modal.target_sheet_id = Some(sheet_id.clone());
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                let col = column.unwrap_or(0).min(ds.archetype.column_count().saturating_sub(1));
                app.grid.variables_dialog.open_for_column(&ds, col);
            } else {
                app.grid.variables_dialog.open = true;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SelectManageVarColumn { sheet_id, column } => {
            app.egui_views.manage_vars_modal.target_sheet_id = Some(sheet_id.clone());
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                let col = column.min(ds.archetype.column_count().saturating_sub(1));
                app.grid.variables_dialog.open_for_column(&ds, col);
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::StageManageVarRecode { from, to } => {
            app.grid.variables_dialog.add_recode_mapping(from, to);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ClearManageVarRecodes => {
            app.grid.variables_dialog.recode_mappings.clear();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ApplyManageVarRecodes { sheet_id } => {
            if let Some(tool_intent) = app.grid.variables_dialog.build_recode_command(&sheet_id) {
                let outcome = app.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
                app.grid.variables_dialog.recode_mappings.clear();
                if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                    app.grid.variables_dialog.sync_from_sheet(&ds);
                }
                return Ok(UiIntentOutcome::Engine { command_id: tool_intent.command_id, outcome });
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::ApplyManageVarOrder { sheet_id, column, order, reference_level } => {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                app.grid.variables_dialog.set_level_order(order.clone(), Some(&ds));
            }
            let params = serde_json::json!({
                "sheet_id": sheet_id.as_str(),
                "col": column,
                "order_spec": order,
                "reference_level": reference_level,
            });
            let outcome = app.execute_session_ui_cmd("table.setCategoricalOrder", params)?;
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                app.grid.variables_dialog.sync_from_sheet(&ds);
            }
            Ok(UiIntentOutcome::Engine { command_id: String::from("table.setCategoricalOrder"), outcome })
        }
        UiIntent::CloseManageVariables => {
            app.grid.variables_dialog.open = false;
            app.egui_views.manage_vars_modal.target_sheet_id = None;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CancelManageVariables { revert_steps } => {
            for _ in 0..revert_steps {
                let _ = app.execute_command("edit.undo", serde_json::json!({}));
            }
            app.grid.variables_dialog.recode_mappings.clear();
            app.grid.variables_dialog.open = false;
            app.egui_views.manage_vars_modal.target_sheet_id = None;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::OpenImportWizard { origin, is_live_link, target_sheet_id } => {
            app.grid.import_wizard.open = true;
            app.grid.import_wizard.origin_kind = origin;
            app.grid.import_wizard.is_live_link = is_live_link;
            app.grid.import_wizard.target_sheet_id = target_sheet_id
                .or_else(|| app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned());
            app.grid.import_wizard.path_policy = app.session.path_policy().clone();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::UpdateImportWizard(wizard) => {
            app.grid.import_wizard = *wizard;
            app.grid.import_wizard.path_policy = app.session.path_policy().clone();
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::PasteClipboardIntoImportReshape => {
            let clip = app
                .read_clipboard_text()
                .ok()
                .filter(|s| s.available)
                .and_then(|s| s.text)
                .filter(|t| !t.is_empty())
                .or_else(|| app.interaction.last_clipboard_text.clone())
                .unwrap_or_default();
            if clip.is_empty() {
                return Ok(UiIntentOutcome::ShellUpdated);
            }
            if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ImportReshape) || app.grid.import_reshape.open {
                if app.grid.import_reshape.mode == crate::grid_view::ImportReshapeMode::TidyLongPivot {
                    app.grid.import_reshape.tidy_csv = clip;
                    app.egui_views.import_wizard.reshape_tidy_csv_draft = app.grid.import_reshape.tidy_csv.clone();
                } else {
                    let parsed = app
                        .grid
                        .import_reshape
                        .parse_plate_values_text(&clip, &app.locale_preset)
                        .map_err(|msg| EngineError::with_code(ErrorCode::BadInput, msg))?;
                    app.grid.import_reshape.plate_values = parsed;
                    let expected = if app.grid.import_reshape.mode == crate::grid_view::ImportReshapeMode::Plate384 { 384 } else { 96 };
                    if app.grid.import_reshape.plate_roles.len() != expected {
                        app.grid.import_reshape.plate_roles =
                            import_wizard::default_plate_roles(expected == 384, app.grid.import_reshape.subtract_blank_mean);
                    }
                    app.egui_views.import_wizard.plate_values_draft = app
                        .grid
                        .import_reshape
                        .plate_values
                        .iter()
                        .map(|v| v.map_or_else(|| String::from("NA"), |n| format!("{n}")))
                        .collect::<Vec<_>>()
                        .join(", ");
                }
            } else if app.grid.import_wizard.open {
                app.grid.import_wizard.raw_clipboard_text = clip.clone();
                app.egui_views.import_wizard.raw_source_draft = clip;
            }
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CommitImportWizard { close_after } => {
            if app.grid.import_wizard.origin_kind == ImportOriginKind::XlsxWorkbook {
                if !app.grid.import_wizard.xlsx_sheets.is_empty() && app.grid.import_wizard.selected_xlsx_sheet_names().is_empty() {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "select at least one worksheet before importing the .xlsx workbook"));
                }
                let Some(params) = app.grid.import_wizard.build_xlsx_import_params() else {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "choose an .xlsx file path or load workbook bytes before importing"));
                };
                let outcome = app.execute_session_ui_cmd("table.importXlsx", params)?;
                if close_after {
                    app.grid.import_wizard.open = false;
                }
                return Ok(UiIntentOutcome::Engine { command_id: String::from("table.importXlsx"), outcome });
            }
            if matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile)
                && !app.grid.import_wizard.is_live_link
                && app.grid.import_wizard.split_by_column.is_some()
            {
                let Some(params) = app.grid.import_wizard.build_csv_import_params("Imported Data") else {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "enter delimited text or a CSV file path before splitting by column"));
                };
                let outcome = app.execute_session_ui_cmd("table.importCsv", params)?;
                if close_after {
                    app.grid.import_wizard.open = false;
                }
                return Ok(UiIntentOutcome::Engine { command_id: String::from("table.importCsv"), outcome });
            }
            let doc = app.session.document();
            let existing_target = app
                .grid
                .import_wizard
                .target_sheet_id
                .as_ref()
                .filter(|id| doc.data_sheet(id).is_some())
                .cloned()
                .or_else(|| doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()).cloned());
            let should_create_target = existing_target.is_none()
                && doc.data_sheets.is_empty()
                && matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::Clipboard | ImportOriginKind::ExternalDelimitedFile);
            if existing_target.is_none() && !should_create_target {
                return Err(EngineError::with_code(
                    ErrorCode::Conflict,
                    "no target data sheet available for import wizard; activate or select a target data sheet first",
                ));
            }
            if should_create_target
                && app.grid.import_wizard.origin_kind == ImportOriginKind::ExternalDelimitedFile
                && !app.grid.import_wizard.is_live_link
                && app.grid.import_wizard.raw_clipboard_text.trim().is_empty()
                && !app.grid.import_wizard.external_file_path.trim().is_empty()
            {
                let Some(params) = app.grid.import_wizard.build_csv_import_params("Imported Data") else {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "choose a CSV file path before importing"));
                };
                let outcome = app.execute_session_ui_cmd("table.importCsv", params)?;
                if close_after {
                    app.grid.import_wizard.open = false;
                }
                return Ok(UiIntentOutcome::Engine { command_id: String::from("table.importCsv"), outcome });
            }
            if let Some(ref target_id) = existing_target
                && matches!(app.grid.import_wizard.origin_kind, ImportOriginKind::InternalDataSheet | ImportOriginKind::InternalResultSheet)
            {
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
            let (command_id, outcome) = app.execute_atomic_group(|a| {
                let target_id = if let Some(id) = existing_target {
                    id
                } else {
                    let created = a.execute_session_ui_cmd("sheet.createData", serde_json::json!({ "kind": "Column", "title": "Imported Data" }))?;
                    created
                        .payload
                        .get("sheet_id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|s| SheetId::try_new(s).ok())
                        .or_else(|| a.session.document().data_sheets.last().map(|s| s.id.clone()))
                        .ok_or_else(|| EngineError::with_code(ErrorCode::Conflict, "failed to create target data sheet for import wizard"))?
                };
                let tool_intent = a.grid.import_wizard.build_command_intent(&target_id, Some(a.session.document()));
                let outcome = a.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
                Ok::<_, EngineError>((tool_intent.command_id, outcome))
            })?;
            if close_after {
                app.grid.import_wizard.open = false;
            }
            Ok(UiIntentOutcome::Engine { command_id, outcome })
        }
        UiIntent::CloseImportWizard => {
            app.grid.import_wizard.open = false;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::SavePlateMapFile { path } => execute_save_plate_map_file(app, &path),
        UiIntent::LoadPlateMapFile { path } => execute_load_plate_map_file(app, &path),
        UiIntent::LoadPlateLayoutFromSheet { layout_sheet_id, x_grid_sheet_id, blank_labels, positive_control_labels } => {
            execute_load_plate_layout_from_sheet(app, &layout_sheet_id, x_grid_sheet_id.as_ref(), blank_labels, positive_control_labels)
        }
        UiIntent::ApplyPlateGridsText { label_grid_text, x_grid_text, blank_labels, positive_control_labels } => {
            execute_apply_plate_grids_text(app, &label_grid_text, &x_grid_text, blank_labels, positive_control_labels)
        }
        UiIntent::OpenTransformModal { sheet_id, kind } => {
            app.egui_views.transform_modals.open_kind = Some(kind);
            app.egui_views.transform_modals.target_sheet_id = Some(sheet_id);
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::CloseTransformModal => {
            app.egui_views.transform_modals.open_kind = None;
            Ok(UiIntentOutcome::ShellUpdated)
        }
        UiIntent::GridCopySelection => execute_grid_clipboard_copy(app, false),
        UiIntent::GridCutSelection => execute_grid_clipboard_copy(app, true),
        UiIntent::GridPasteFromClipboard { start_coord } => execute_grid_clipboard_paste(app, start_coord),
        UiIntent::PasteXCellText { sheet_id, x_row, x_subcol, text } => {
            let res = crate::grid_view::paste_text_into_active_x_cell(app, &sheet_id, x_row, x_subcol, &text)?;
            Ok(UiIntentOutcome::Ui { command_id: String::from("table.paste"), result: Some(res) })
        }
        UiIntent::WorkspaceCopy { dpi } => execute_workspace_copy(app, dpi),
        UiIntent::WorkspaceCopyForSlides { dpi } => execute_workspace_copy_for_slides(app, dpi),
        UiIntent::CanvasPaste { target_sheet_id, anchor_pt } => execute_canvas_paste(app, &target_sheet_id, anchor_pt),
        UiIntent::PasteImageFromClipboard { target_sheet_id, anchor_pt } => execute_paste_image_from_clipboard(app, &target_sheet_id, anchor_pt),
        _ => Ok(UiIntentOutcome::ShellUpdated),
    }
}

fn build_canvas_paste_bounds(anchor_pt: Option<[f64; 2]>, w: f64, h: f64) -> Option<Vec<f64>> {
    anchor_pt.map(|[x, y]| vec![x.max(4.0), y.max(4.0), w, h])
}

fn execute_paste_image_from_clipboard(
    app: &mut UiApp,
    target_sheet_id: &SheetId,
    anchor_pt: Option<[f64; 2]>,
) -> Result<UiIntentOutcome, EngineError> {
    let img_bytes = app
        .read_clipboard_image()
        .map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?
        .or_else(|| app.interaction.last_clipboard_image_png.clone());
    let Some(bytes) = img_bytes else {
        return Err(EngineError::with_code(ErrorCode::Conflict, "No PNG, JPEG, or TIFF image found on clipboard"));
    };
    let mut params = serde_json::Map::new();
    params.insert(String::from("target_sheet_id"), serde_json::json!(target_sheet_id.as_str()));
    params.insert(String::from("bytes_base64"), serde_json::json!(curvebook_platform::base64::encode(&bytes)));
    if let Some(bounds) = build_canvas_paste_bounds(anchor_pt, 160.0, 120.0) {
        params.insert(String::from("bounds_pt"), serde_json::json!(bounds));
    }
    let outcome = app.execute_command("edit.pasteImage", serde_json::Value::Object(params))?;
    app.interaction.last_clipboard_image_png = Some(bytes);
    app.sync_views();
    Ok(UiIntentOutcome::Engine { command_id: String::from("edit.pasteImage"), outcome })
}

fn execute_canvas_paste(app: &mut UiApp, target_sheet_id: &SheetId, anchor_pt: Option<[f64; 2]>) -> Result<UiIntentOutcome, EngineError> {
    if let Ok(Some(bytes)) = app.read_clipboard_image() {
        app.interaction.last_clipboard_image_png = Some(bytes);
        return execute_paste_image_from_clipboard(app, target_sheet_id, anchor_pt);
    }
    if let Some(copied) = app.interaction.last_copied_result_table.clone() {
        let mut params = serde_json::Map::new();
        params.insert(String::from("source_sheet_id"), serde_json::json!(copied.source_sheet_id.as_str()));
        params.insert(String::from("target_sheet_id"), serde_json::json!(target_sheet_id.as_str()));
        if let Some(tab_id) = copied.tab_id {
            params.insert(String::from("tab_id"), serde_json::json!(tab_id));
        }
        if let Some(rows) = copied.selected_rows {
            params.insert(String::from("selected_rows"), serde_json::json!(rows));
        }
        if let Some(cols) = copied.selected_cols {
            params.insert(String::from("selected_cols"), serde_json::json!(cols));
        }
        if let Some(bounds) = build_canvas_paste_bounds(anchor_pt, 180.0, 90.0) {
            params.insert(String::from("bounds_pt"), serde_json::json!(bounds));
        }
        let outcome = app.execute_command("edit.pasteResultTable", serde_json::Value::Object(params))?;
        app.sync_views();
        return Ok(UiIntentOutcome::Engine { command_id: String::from("edit.pasteResultTable"), outcome });
    }
    if app.interaction.last_clipboard_image_png.is_some() {
        return execute_paste_image_from_clipboard(app, target_sheet_id, anchor_pt);
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_workspace_copy(app: &mut UiApp, dpi: u16) -> Result<UiIntentOutcome, EngineError> {
    if let Some(rs_id) = crate::interaction::resolve_result_sheet_for_shortcut(app, false) {
        let outcome = crate::dialogs_canvas::copy_result_table_to_clipboard(app, rs_id.as_str(), false);
        if outcome.handled {
            return Ok(UiIntentOutcome::Ui {
                command_id: outcome.command_id.unwrap_or_else(|| String::from("results.copy_selection")),
                result: outcome.command_result,
            });
        }
    }
    if !app.egui_views.launcher_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && let Some(fig_id) = crate::interaction::resolve_figure_sheet_for_shortcut(app)
    {
        let outcome =
            crate::interaction::copy_figure_sheet_bundle_dpi(app, &fig_id, dpi).map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?;
        return Ok(UiIntentOutcome::Ui { command_id: outcome.command_id.unwrap_or(outcome.action), result: outcome.command_result });
    }
    if app.session.document().active_sheet_id.as_ref().is_some_and(|id| app.session.document().data_sheet(id).is_some()) {
        return execute_grid_clipboard_copy(app, false);
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_workspace_copy_for_slides(app: &mut UiApp, dpi: u16) -> Result<UiIntentOutcome, EngineError> {
    if !app.egui_views.launcher_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && let Some(fig_id) = crate::interaction::resolve_figure_sheet_for_shortcut(app)
            .or_else(|| super::super::command_palette::resolve_renderable_sheet_id_for_menu(app.document()))
    {
        let outcome = crate::interaction::copy_figure_sheet_slide_bundle_dpi(app, &fig_id, dpi)
            .map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?;
        return Ok(UiIntentOutcome::Ui { command_id: outcome.command_id.unwrap_or(outcome.action), result: outcome.command_result });
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_grid_clipboard_copy(app: &mut UiApp, cut: bool) -> Result<UiIntentOutcome, EngineError> {
    let (plan, clear_reqs, frozen) = {
        let doc = app.session.document();
        let Some(ds_id) = doc.active_sheet_id.as_ref().filter(|id| doc.data_sheet(id).is_some()) else {
            return Ok(UiIntentOutcome::ShellUpdated);
        };
        let Some(ds) = doc.data_sheet(ds_id) else {
            return Ok(UiIntentOutcome::ShellUpdated);
        };
        let plan = app
            .grid
            .prepare_clipboard_selection(ds, app.locale_preset.decimal_separator)
            .map_err(|msg| EngineError::with_code(ErrorCode::Conflict, msg))?;
        let clear_reqs = if cut && !ds.frozen { plan.clear_cells_requests(ds) } else { Vec::new() };
        (plan, clear_reqs, ds.frozen)
    };
    app.interaction.last_clipboard_text = Some(plan.tsv.clone());
    let _ = app.write_clipboard_text(&plan.tsv);
    if cut && !frozen && !clear_reqs.is_empty() {
        let last_outcome = app.execute_atomic_group(|a| {
            let mut out = None;
            for params in clear_reqs {
                out = Some(a.execute_session_ui_cmd("table.clearCells", params)?);
            }
            Ok::<_, EngineError>(out)
        })?;
        if let Some(outcome) = last_outcome {
            return Ok(UiIntentOutcome::Engine { command_id: String::from("table.clearCells"), outcome });
        }
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_grid_clipboard_paste(app: &mut UiApp, start_coord: Option<curvebook_tools::GridCellCoord>) -> Result<UiIntentOutcome, EngineError> {
    let Some(ds_id) = app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let clip = app
        .read_clipboard_text()
        .ok()
        .filter(|s| s.available)
        .and_then(|s| s.text)
        .filter(|t| !t.is_empty())
        .or_else(|| app.interaction.last_clipboard_text.clone())
        .unwrap_or_default();
    if clip.is_empty() {
        return Ok(UiIntentOutcome::ShellUpdated);
    }
    let anchor = start_coord.or_else(|| app.grid.active_cell_for_sheet(&ds_id)).unwrap_or(curvebook_tools::GridCellCoord::new(0, 0, 0));
    let clean_clip = clip.strip_prefix('\u{FEFF}').unwrap_or(&clip).replace('\0', "");
    let dec = app.locale_preset.decimal_separator;
    let mut paste_params = serde_json::json!({
        "sheet_id": ds_id.as_str(),
        "text": clean_clip,
        "start_row": anchor.row,
        "start_col": anchor.col,
        "start_subcol": anchor.subcol,
        "decimal_separator": if dec == ',' { "," } else { "auto" },
        "propagate_title_header": false,
    });
    if dec == ','
        && !clean_clip.contains(';')
        && (clean_clip.contains('\t') || clean_clip.lines().all(|line| line.matches(',').count() <= 1))
        && let Some(obj) = paste_params.as_object_mut()
    {
        obj.insert(String::from("delimiter"), serde_json::json!("\t"));
    }
    let outcome = app.execute_session_ui_cmd("table.paste", paste_params)?;
    Ok(UiIntentOutcome::Engine { command_id: String::from("table.paste"), outcome })
}

fn apply_wave4_modal_state_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::SetAnalyzeQuery { query } => app.canvas_dialogs.analyze.search_query = query,
        UiIntent::SelectAnalyzeEntry { entry_id, clear_duplicate_prompt } => {
            app.canvas_dialogs.analyze.selected_entry_id = Some(entry_id);
            if clear_duplicate_prompt && let ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } = app.canvas_dialogs.active_dialog.clone()
            {
                app.canvas_dialogs.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id };
            }
        }
        UiIntent::ToggleAnalyzeDataset { col_index } => {
            let sel = &mut app.canvas_dialogs.analyze.selected_data_sets;
            if let Some(pos) = sel.iter().position(|&c| c == col_index) {
                sel.remove(pos);
            } else {
                sel.push(col_index);
            }
        }
        UiIntent::SetAnalyzeSelectedDatasets { cols } => {
            app.canvas_dialogs.analyze.selected_data_sets = cols;
        }
        UiIntent::SetEditParamsDraft { spec } => {
            app.canvas_dialogs.edit_params.draft = Some(*spec);
            app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
        }
        UiIntent::SetEditParamsActiveTab { tab } => {
            app.canvas_dialogs.edit_params.active_tab = tab;
        }
        UiIntent::SetEditParamsPairedOrMatched { paired } => {
            app.canvas_dialogs.edit_params.paired_or_matched = paired;
        }
        UiIntent::SetEditParamsConfidence { level } => {
            if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
                spec.confidence_level = level;
            }
        }
        UiIntent::SaveEditParamsAsDefault => {
            if let Some(spec) = app.canvas_dialogs.edit_params.draft.clone() {
                match &spec.kind {
                    curvebook_doc::AnalysisKind::OneWayAnova(_) => {
                        app.egui_views.anova_modal.saved_default_spec = Some(spec);
                        app.egui_views.anova_modal.method_saved_as_default = true;
                    }
                    curvebook_doc::AnalysisKind::NonlinearRegression(_) => {
                        app.egui_views.fit_modal.saved_default_spec = Some(spec);
                        app.egui_views.fit_modal.method_saved_as_default = true;
                    }
                    _ => {
                        let disc = std::mem::discriminant(&spec.kind);
                        app.egui_views.stats_modals.saved_defaults.retain(|d| std::mem::discriminant(&d.kind) != disc);
                        app.egui_views.stats_modals.saved_defaults.push(spec);
                        app.egui_views.stats_modals.method_saved_as_default = true;
                    }
                }
            } else {
                app.egui_views.anova_modal.method_saved_as_default = true;
                app.egui_views.stats_modals.method_saved_as_default = true;
                app.egui_views.fit_modal.method_saved_as_default = true;
            }
        }
        UiIntent::SelectRetargetSource { target_data_sheet_id } => {
            app.canvas_dialogs.change_data.selected_target_id = Some(target_data_sheet_id);
        }
        UiIntent::ResolveDuplicateAnalysis { modify_existing_id } => {
            let choice = match modify_existing_id {
                Some(id) => AnalyzeDuplicateChoice::ModifyExisting(id),
                None => AnalyzeDuplicateChoice::CreateNew,
            };
            if let Some(tool_intent) = app.canvas_dialogs.analyze.resolve_duplicate_choice(&choice) {
                let outcome = app.execute_session_ui_cmd(&tool_intent.command_id, tool_intent.params)?;
                app.canvas_dialogs.close_dialog();
                return Ok(UiIntentOutcome::Engine { command_id: tool_intent.command_id, outcome });
            }
        }
        UiIntent::SelectHookConstant { index } => {
            app.canvas_dialogs.hook_picker.selected_index = index;
        }
        UiIntent::UnlinkHookConstant => {
            app.canvas_dialogs.hook_picker.selected_index = None;
        }
        UiIntent::UpdatePowerWizardState(state) => {
            app.canvas_dialogs.power_wizard = *state;
            let _ = app.canvas_dialogs.power_wizard.recalculate();
            sync_edit_params_from_power_wizard(app);
        }
        UiIntent::CommitPowerWizard { action } => {
            let result = super::super::dialog_actions::execute_dialog_button_action(app, &action)?;
            if !power_studio::is_power_studio_dialog_active(app) {
                power_studio::reset_results_transient_state(&mut app.egui_views.power_studio);
            }
            return Ok(UiIntentOutcome::Ui { command_id: format!("power_wizard:{action}"), result: Some(result) });
        }
        UiIntent::UpdateChangeGraphTypeState(state) => app.canvas_dialogs.change_graph_type = *state,
        UiIntent::UpdateFormatGraphState(state) => app.canvas_dialogs.format_graph = *state,
        UiIntent::UpdateFormatAxesState(state) => app.canvas_dialogs.format_axes = *state,
        UiIntent::UpdateBubblePlotState(state) => app.canvas_dialogs.bubble_plot = *state,
        UiIntent::UpdatePairwiseState(state) => app.canvas_dialogs.pairwise = *state,
        UiIntent::UpdatePageLayoutState(state) => app.canvas_dialogs.page_layout = *state,
        UiIntent::UpdateExportFigureState(state) => app.canvas_dialogs.export_figure = *state,
        UiIntent::UpdatePrintDialogState(state) => app.canvas_dialogs.print = *state,
        UiIntent::UpdateCreateGraphState(state) => app.canvas_dialogs.create_graph = *state,
        UiIntent::UpdateEqualizeScalingState(state) => app.canvas_dialogs.equalize_scaling = *state,
        UiIntent::UpdateFormatEmbeddedTableState(state) => app.canvas_dialogs.format_embedded_table = *state,
        UiIntent::UpdateFormatImageState(state) => app.canvas_dialogs.format_image = *state,
        UiIntent::UpdateApplyStyleState(state) => app.canvas_dialogs.apply_style = *state,
        UiIntent::UpdateRandomizeGroupsState(state) => app.canvas_dialogs.randomize_groups = *state,
        UiIntent::UpdateRandomSampleRowsState(state) => app.canvas_dialogs.random_sample_rows = *state,
        UiIntent::UpdateSimulateDistributionsState(state) => app.canvas_dialogs.simulate_distributions = *state,
        UiIntent::SyncResultSelection { sheet_id, tab_index, anchor, focus } => {
            app.canvas_dialogs.result_view.bound_sheet_id = Some(sheet_id.as_str().to_owned());
            app.canvas_dialogs.result_view.bound_tab_idx = Some(tab_index);
            app.canvas_dialogs.result_view.selected_anchor = Some(anchor);
            app.canvas_dialogs.result_view.selected_focus = Some(focus);
        }
        UiIntent::SetGallerySelection { sheet_ids } => {
            app.nav.gallery.selected_ids = sheet_ids.into_iter().collect();
            let doc = app.session.document().clone();
            app.nav.sync_gallery(&doc);
        }
        UiIntent::SetGalleryViewOptions { search_query, sort_order, current_family_only } => {
            let doc = app.session.document().clone();
            if let Some(ref q) = search_query {
                app.egui_views.center_stage.gallery_search = q.clone();
                app.egui_views.center_stage.last_synced_gallery_search = q.clone();
            }
            app.nav.set_gallery_view_options(&doc, search_query, sort_order, current_family_only);
        }
        UiIntent::OpenExportFigure { sheet_id, target_selected } => {
            app.canvas_dialogs.open_export_figure(&sheet_id);
            app.egui_views.app_modals.export_selected_gallery_scope = target_selected;
        }
        UiIntent::OpenPrintForSheets { sheet_ids } => {
            let doc = app.session.document().clone();
            let valid: Vec<SheetId> = sheet_ids.into_iter().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).collect();
            if let Some(first) = valid.first() {
                app.open_print_dialog_for(Some(first));
                app.canvas_dialogs.print.job.sheet_ids = valid.clone();
                if valid.len() > 1 {
                    app.canvas_dialogs.print.job.title = Some(format!("{} sheets", valid.len()));
                }
            }
        }
        UiIntent::OpenCsvExportForSheets { sheet_ids } => {
            let doc = app.session.document().clone();
            let valid: Vec<SheetId> = sheet_ids.into_iter().filter(|id| doc.data_sheet(id).is_some()).collect();
            if let Some(first) = valid.first().cloned() {
                app.canvas_dialogs.csv_export.reset_for_locale(app.locale_preset.decimal_separator);
                let target = if valid.len() == 1 {
                    crate::dialogs_canvas::CsvExportTargetKind::DataTable { sheet_id: first }
                } else {
                    crate::dialogs_canvas::CsvExportTargetKind::DataTables { sheet_ids: valid }
                };
                app.canvas_dialogs.active_dialog = ActiveDialog::CsvExportOptions(target);
            }
        }
        UiIntent::PlaceSelectedOntoNewLayout { graph_ids } => {
            return apply_place_selected_onto_new_layout(app, &graph_ids);
        }
        _ => {}
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn batch_layout_grid_dims(count: usize) -> (u8, u8) {
    match count {
        0 | 1 => (1, 1),
        2 => (1, 2),
        3 | 4 => (2, 2),
        5 | 6 => (2, 3),
        7..=9 => (3, 3),
        _ => {
            let cols = (count as f64).sqrt().ceil().clamp(1.0, 8.0) as u8;
            let rows = count.div_ceil(usize::from(cols)).clamp(1, 8) as u8;
            (rows, cols)
        }
    }
}

fn apply_place_selected_onto_new_layout(app: &mut UiApp, graph_ids: &[SheetId]) -> Result<UiIntentOutcome, EngineError> {
    let valid: Vec<SheetId> = {
        let doc = app.session.document();
        graph_ids.iter().filter(|id| doc.graph_sheet(id).is_some()).cloned().collect()
    };
    if valid.is_empty() {
        return Ok(UiIntentOutcome::ShellUpdated);
    }
    let (rows, cols) = batch_layout_grid_dims(valid.len());
    let create_out = app.execute_session_ui_cmd(
        "layout.create",
        serde_json::json!({
            "grid_template": [rows, cols],
            "title": format!("Layout ({})", valid.len()),
        }),
    )?;
    let Some(created_raw) = create_out.payload.get("sheet_id").and_then(|v| v.as_str()) else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let layout_id = SheetId::new(created_raw);
    let mut composer = crate::dialogs_canvas::PageLayoutComposerState { rows, cols, ..Default::default() };
    composer.layout_sheet_id = Some(layout_id.clone());
    composer.arrange_grid(&valid, 14.0, 10.0);
    let _ = app.execute_session_ui_cmd(
        "layout.setSpec",
        serde_json::json!({
            "sheet_id": layout_id.as_str(),
            "spec": composer.to_layout_spec(),
        }),
    )?;
    let _ = app.execute_session_ui_cmd("sheet.select", serde_json::json!({ "sheet_id": layout_id.as_str() }))?;
    app.nav.gallery.open = false;
    app.sync_views();
    Ok(UiIntentOutcome::Ui {
        command_id: String::from("ui.gallery.placeOnNewLayout"),
        result: Some(serde_json::json!({ "sheet_id": layout_id.as_str(), "panel_count": valid.len() })),
    })
}

fn sync_edit_params_from_power_wizard(app: &mut UiApp) {
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return;
    }
    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
        && let curvebook_doc::AnalysisKind::PowerAnalysis(ref existing) = spec.kind
    {
        let info_id = existing.info_sheet_id.clone();
        spec.update_kind(curvebook_doc::AnalysisKind::PowerAnalysis(app.canvas_dialogs.power_wizard.to_doc_spec(info_id)));
    }
}

fn apply_wave4_app_modal_intent(app: &mut UiApp, intent: UiIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        UiIntent::OpenPreferences { category } => {
            app.egui_views.app_modals.open_preferences(&app.shortcuts, category);
            app.egui_views.app_modals.preferences.capture_live_snapshot(
                &app.locale,
                app.theme.mode,
                app.theme.ui_scale,
                app.nav.nesting_mode,
                app.preview_filter,
            );
        }
        UiIntent::ClosePreferences { save } => {
            if save {
                let scale_val = app.egui_views.app_modals.preferences.ui_scale_spinner.last_committed_value;
                if let Some(pct) = scale_val {
                    let scale = (pct / 100.0).clamp(0.75, 3.0) as f32;
                    app.theme.ui_scale = scale;
                }
                if let Some(staged) = app.egui_views.app_modals.preferences.staged_shortcuts.take() {
                    app.shortcuts = staged;
                }
                app.egui_views.app_modals.preferences.initial_snapshot = None;
            } else {
                if let Some(snap) = app.egui_views.app_modals.preferences.initial_snapshot.take() {
                    app.set_theme_mode(snap.theme_mode);
                    let _ = app.set_ui_scale(snap.ui_scale);
                    let _ = app.set_locale(&snap.locale);
                    app.nav.nesting_mode = snap.nesting_mode;
                    app.preview_filter = snap.preview_filter;
                }
                app.egui_views.app_modals.preferences.reset_scale_spinner();
            }
            app.egui_views.app_modals.preferences.discard_shortcut_draft();
            app.egui_views.app_modals.preferences_open = false;
        }
        UiIntent::SetShortcutBinding { command_id, combo } => {
            let staged = app.egui_views.app_modals.preferences.staged_shortcuts.get_or_insert_with(|| app.shortcuts.clone());
            match staged.set_shortcut(&command_id, &combo) {
                Ok(()) => {
                    app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
                }
                Err(err) => {
                    app.egui_views.app_modals.preferences.shortcut_conflict_error = Some(err.clone());
                    return Err(EngineError::with_code(ErrorCode::Conflict, err));
                }
            }
        }
        UiIntent::ResetShortcutBinding { command_id } => {
            let staged = app.egui_views.app_modals.preferences.staged_shortcuts.get_or_insert_with(|| app.shortcuts.clone());
            match command_id {
                Some(id) => {
                    if let Err(err) = staged.reset_shortcut(&id) {
                        app.egui_views.app_modals.preferences.shortcut_conflict_error = Some(err.clone());
                        return Err(EngineError::with_code(ErrorCode::Conflict, err));
                    }
                    app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
                }
                None => {
                    staged.reset_all();
                    app.egui_views.app_modals.preferences.shortcut_conflict_error = None;
                }
            }
        }
        UiIntent::UpdatePreferencesDraft(mut draft) => {
            if draft.staged_shortcuts.is_none() {
                draft.staged_shortcuts = app.egui_views.app_modals.preferences.staged_shortcuts.take();
            }
            app.egui_views.app_modals.preferences = *draft;
        }
        UiIntent::ResolveUnsavedChanges { action } => {
            app.egui_views.app_modals.pending_unsaved_action = Some(action);
            app.egui_views.app_modals.unsaved_changes_open = false;
        }
        UiIntent::OpenFindReplace { initial_query } => {
            if let Some(q) = initial_query {
                app.egui_views.app_modals.find_replace.find_query = q;
            }
            app.egui_views.app_modals.find_replace_open = true;
        }
        UiIntent::CloseFindReplace => {
            app.egui_views.app_modals.find_replace_open = false;
            app.egui_views.app_modals.find_replace.match_count = 0;
            app.egui_views.app_modals.find_replace.status_message = None;
        }
        UiIntent::ExecuteFindReplace { replace_all, find_query, replace_with, match_case, match_entire_cell } => {
            return apply_find_replace_on_active_sheet(app, replace_all, &find_query, &replace_with, match_case, match_entire_cell);
        }
        UiIntent::CopyImportWarnings => {
            if let Err(err) = app.copy_import_warnings_to_clipboard() {
                app.canvas_dialogs.status_banner = Some(format!("Could not copy import warnings: {err}"));
            }
        }
        UiIntent::UpdateCsvExportOptions(opts) => {
            app.canvas_dialogs.csv_export = *opts;
        }
        _ => {}
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FindCellTarget {
    XCell { row: usize, subcol: usize },
    DataCell { coord: CellCoord },
}

struct FindCellMatch {
    pos_key: (usize, bool, usize, usize),
    target: FindCellTarget,
    next_value: Option<String>,
}

fn replace_matched_cell_text(raw: &str, find_query: &str, query_cmp: &str, replace_with: &str, match_case: bool, match_entire_cell: bool) -> String {
    if match_entire_cell {
        replace_with.to_owned()
    } else if match_case {
        raw.replace(find_query, replace_with)
    } else {
        let mut out = String::new();
        let lower_raw = raw.to_ascii_lowercase();
        let mut cursor = 0usize;
        while let Some(rel) = lower_raw.get(cursor..).and_then(|slice| slice.find(query_cmp)) {
            let start = cursor + rel;
            let end = start + query_cmp.len();
            if let Some(prefix) = raw.get(cursor..start) {
                out.push_str(prefix);
            }
            out.push_str(replace_with);
            cursor = end;
        }
        if let Some(tail) = raw.get(cursor..) {
            out.push_str(tail);
        }
        out
    }
}

fn cell_raw_find_text(cell: &curvebook_doc::Cell) -> Option<String> {
    match &cell.value {
        curvebook_doc::CellValue::Empty => None,
        curvebook_doc::CellValue::Number(v) => Some(v.to_string()),
        curvebook_doc::CellValue::Text(t) => Some(t.clone()),
    }
}

fn apply_find_replace_on_active_sheet(
    app: &mut UiApp,
    replace_all: bool,
    find_query: &str,
    replace_with: &str,
    match_case: bool,
    match_entire_cell: bool,
) -> Result<UiIntentOutcome, EngineError> {
    if find_query.is_empty() {
        return Ok(UiIntentOutcome::ShellUpdated);
    }
    let Some(ds_id) = app.session.document().active_sheet_id.as_ref().filter(|id| app.session.document().data_sheet(id).is_some()).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };
    let Some(ds) = app.session.document().data_sheet(&ds_id).cloned() else {
        return Ok(UiIntentOutcome::ShellUpdated);
    };

    let query_cmp = if match_case { find_query.to_owned() } else { find_query.to_ascii_lowercase() };
    let rows = ds.archetype.row_count();
    let cols = ds.archetype.column_count();
    let x_is_generated_seq = matches!(
        &ds.archetype,
        curvebook_doc::TableArchetype::Xy(xy) if matches!(xy.x_mode, curvebook_doc::XColumnMode::GeneratedSequence { .. })
    );

    let mut matches: Vec<FindCellMatch> = Vec::new();
    for r in 0..rows {
        if !x_is_generated_seq {
            for xs in 0usize.. {
                let Some(cell) = ds.archetype.x_or_time_cell(r, xs) else {
                    break;
                };
                if let Some(raw) = cell_raw_find_text(cell) {
                    let hay = if match_case { raw.clone() } else { raw.to_ascii_lowercase() };
                    let matched = if match_entire_cell { hay == query_cmp } else { hay.contains(&query_cmp) };
                    if matched {
                        let next_value =
                            replace_all.then(|| replace_matched_cell_text(&raw, find_query, &query_cmp, replace_with, match_case, match_entire_cell));
                        matches.push(FindCellMatch { pos_key: (r, false, 0, xs), target: FindCellTarget::XCell { row: r, subcol: xs }, next_value });
                    }
                }
            }
        }
        for c in 0..cols {
            let subcols = crate::grid_view::data_set_subcolumn_count(&ds.archetype, c).max(1);
            for s in 0..subcols {
                let coord = CellCoord::new(r, c, s);
                let Some(cell) = ds.archetype.data_cell(coord) else {
                    continue;
                };
                let Some(raw) = cell_raw_find_text(cell) else {
                    continue;
                };
                let hay = if match_case { raw.clone() } else { raw.to_ascii_lowercase() };
                let matched = if match_entire_cell { hay == query_cmp } else { hay.contains(&query_cmp) };
                if matched {
                    let next_value =
                        replace_all.then(|| replace_matched_cell_text(&raw, find_query, &query_cmp, replace_with, match_case, match_entire_cell));
                    matches.push(FindCellMatch { pos_key: (r, true, c, s), target: FindCellTarget::DataCell { coord }, next_value });
                }
            }
        }
    }

    app.egui_views.app_modals.find_replace.match_count = matches.len();
    let cur_pos_key = if let Some((xr, xs)) = app.grid.active_x_cell_for_sheet(&ds_id) {
        Some((xr, false, 0usize, xs))
    } else {
        app.grid.active_cell_for_sheet(&ds_id).map(|ac| (ac.row, true, ac.col, ac.subcol))
    };

    let selected_match = if replace_all {
        matches.first()
    } else if let Some(cur) = cur_pos_key {
        matches.iter().find(|m| m.pos_key > cur).or_else(|| matches.first())
    } else {
        matches.first()
    };

    if let Some(chosen) = selected_match {
        match chosen.target {
            FindCellTarget::XCell { row, subcol } => {
                app.grid.select_x_cell_for_sheet(&ds_id, row, subcol);
                app.focus_widget_id = Some(format!("grid.xcell.{row}.{subcol}"));
            }
            FindCellTarget::DataCell { coord } => {
                app.grid.select_single_for_sheet(&ds_id, curvebook_tools::GridCellCoord::new(coord.row, coord.col, coord.subcol));
                app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
            }
        }
    }

    if replace_all {
        let mut x_edits = Vec::new();
        let mut data_edits = Vec::new();
        for m in matches {
            let Some(val) = m.next_value else {
                continue;
            };
            match m.target {
                FindCellTarget::XCell { row, subcol } => {
                    x_edits.push(serde_json::json!({ "row": row, "subcol": subcol, "value": val }));
                }
                FindCellTarget::DataCell { coord } => {
                    data_edits.push(serde_json::json!({ "row": coord.row, "col": coord.col, "subcol": coord.subcol, "value": val }));
                }
            }
        }
        let last_outcome = app.execute_atomic_group(|a| {
            let mut last: Option<(&'static str, curvebook_engine::CommandOutcome)> = None;
            if !x_edits.is_empty() {
                let outcome = a.execute_session_ui_cmd("table.setXCells", serde_json::json!({ "sheet_id": ds_id.as_str(), "edits": x_edits }))?;
                last = Some(("table.setXCells", outcome));
            }
            if !data_edits.is_empty() {
                let outcome = a.execute_session_ui_cmd("table.setCells", serde_json::json!({ "sheet_id": ds_id.as_str(), "edits": data_edits }))?;
                last = Some(("table.setCells", outcome));
            }
            Ok::<_, EngineError>(last)
        })?;
        if let Some((cmd_id, outcome)) = last_outcome {
            return Ok(UiIntentOutcome::Engine { command_id: String::from(cmd_id), outcome });
        }
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn step_history_intent(app: &mut UiApp, forward: bool) -> Result<UiIntentOutcome, EngineError> {
    let target = if forward {
        if app.egui_views.can_go_forward() {
            app.egui_views.history_cursor += 1;
            app.egui_views.active_sheet_history.get(app.egui_views.history_cursor).cloned()
        } else {
            None
        }
    } else if app.egui_views.can_go_back() {
        app.egui_views.history_cursor -= 1;
        app.egui_views.active_sheet_history.get(app.egui_views.history_cursor).cloned()
    } else {
        None
    };

    if let Some(sheet_id) = target {
        let outcome = app.execute_session_ui_cmd("sheet.select", serde_json::json!({ "sheet_id": sheet_id.as_str() }))?;
        return Ok(UiIntentOutcome::Engine { command_id: String::from("sheet.select"), outcome });
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_save_plate_map_file(app: &mut UiApp, raw_path: &str) -> Result<UiIntentOutcome, EngineError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        app.grid.import_reshape.plate_status_message = Some(String::from("Enter a .cbplate file path before saving."));
        return Err(EngineError::with_code(ErrorCode::BadInput, "enter a .cbplate file path before saving"));
    }
    let mut path = std::path::PathBuf::from(trimmed);
    let has_ext = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(curvebook_format::PLATE_MAP_EXTENSION));
    if !has_ext {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("plate_layout");
        path.set_file_name(format!("{file_name}.{}", curvebook_format::PLATE_MAP_EXTENSION));
    }

    let geom = if app.grid.import_reshape.mode == crate::grid_view::ImportReshapeMode::Plate384 {
        curvebook_doc::PlateGeometry::Wells384
    } else {
        curvebook_doc::PlateGeometry::Wells96
    };
    let roles = super::super::plate_editor::materialize_plate_roles(&app.grid.import_reshape.plate_roles, geom);
    let name =
        if app.grid.import_reshape.title.trim().is_empty() { String::from("Plate Layout") } else { app.grid.import_reshape.title.trim().to_owned() };
    let x_title = if app.grid.import_reshape.x_title.trim().is_empty() {
        String::from("Concentration")
    } else {
        app.grid.import_reshape.x_title.trim().to_owned()
    };
    let file = curvebook_format::PlateMapFile::new(name, geom, x_title, roles).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(format!("Save failed: {msg}"));
        EngineError::with_code(ErrorCode::BadInput, msg)
    })?;
    curvebook_format::save_cbplate_file(&path, &file, true).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(format!("Save failed: {msg}"));
        EngineError::with_code(ErrorCode::Conflict, msg)
    })?;
    let path_str = path.to_string_lossy().into_owned();
    app.grid.import_reshape.plate_template_path = path_str.clone();
    app.grid.import_reshape.plate_status_message = Some(format!("Saved .cbplate ({} wells): {path_str}", file.wells.len()));
    app.egui_views.import_wizard.plate_cbplate_path_draft = path_str;
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_load_plate_map_file(app: &mut UiApp, raw_path: &str) -> Result<UiIntentOutcome, EngineError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        app.grid.import_reshape.plate_status_message = Some(String::from("Enter a .cbplate file path before loading."));
        return Err(EngineError::with_code(ErrorCode::BadInput, "enter a .cbplate file path before loading"));
    }
    let file = curvebook_format::load_cbplate_file(std::path::Path::new(trimmed)).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(format!("Load failed: {msg}"));
        EngineError::with_code(ErrorCode::BadInput, msg)
    })?;
    app.grid.import_reshape.mode = match file.geometry {
        curvebook_doc::PlateGeometry::Wells96 => crate::grid_view::ImportReshapeMode::Plate96,
        curvebook_doc::PlateGeometry::Wells384 => crate::grid_view::ImportReshapeMode::Plate384,
    };
    app.grid.import_reshape.x_title = file.x_title.clone();
    app.grid.import_reshape.plate_roles = file.wells.iter().map(super::super::plate_editor::role_to_json).collect();
    app.grid.import_reshape.plate_layout_source = crate::grid_view::PlateLayoutSourceMode::InteractiveGrid;
    app.grid.import_reshape.plate_template_path = trimmed.to_owned();
    app.grid.import_reshape.plate_status_message = Some(format!("Loaded .cbplate `{}` ({} wells)", file.name, file.wells.len()));
    app.egui_views.import_wizard.reshape_x_title_draft = file.x_title;
    app.egui_views.import_wizard.plate_cbplate_path_draft = trimmed.to_owned();
    app.egui_views.import_wizard.plate_subtab = import_wizard::PlateSubTab::Layout;
    Ok(UiIntentOutcome::ShellUpdated)
}

fn execute_load_plate_layout_from_sheet(
    app: &mut UiApp,
    layout_sheet_id: &SheetId,
    x_grid_sheet_id: Option<&SheetId>,
    blank_labels: Vec<String>,
    positive_control_labels: Vec<String>,
) -> Result<UiIntentOutcome, EngineError> {
    let doc = app.session.document();
    let Some(layout_sheet) = doc.data_sheet(layout_sheet_id).or_else(|| doc.result_sheet(layout_sheet_id).and_then(|r| r.analyzable_data.as_ref()))
    else {
        let msg = format!("plate layout sheet `{}` not found", layout_sheet_id.as_str());
        app.grid.import_reshape.plate_status_message = Some(msg.clone());
        return Err(EngineError::with_code(ErrorCode::BadInput, msg));
    };
    let (_, label_grid) = curvebook_transform::extract_plate_grid_from_datasheet(layout_sheet).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(msg.clone());
        EngineError::with_code(ErrorCode::BadInput, msg)
    })?;
    let x_grid = if let Some(xid) = x_grid_sheet_id {
        let Some(x_sheet) = doc.data_sheet(xid).or_else(|| doc.result_sheet(xid).and_then(|r| r.analyzable_data.as_ref())) else {
            let msg = format!("plate X-grid sheet `{}` not found", xid.as_str());
            app.grid.import_reshape.plate_status_message = Some(msg.clone());
            return Err(EngineError::with_code(ErrorCode::BadInput, msg));
        };
        let (_, xg) = curvebook_transform::extract_plate_x_grid_from_datasheet(x_sheet).map_err(|err| {
            let msg = err.to_string();
            app.grid.import_reshape.plate_status_message = Some(msg.clone());
            EngineError::with_code(ErrorCode::BadInput, msg)
        })?;
        Some(xg)
    } else {
        None
    };

    let name =
        if app.grid.import_reshape.title.trim().is_empty() { String::from("Plate Layout") } else { app.grid.import_reshape.title.trim().to_owned() };
    let x_title = if app.grid.import_reshape.x_title.trim().is_empty() {
        String::from("Concentration")
    } else {
        app.grid.import_reshape.x_title.trim().to_owned()
    };
    let opts = curvebook_transform::PlateGridBuildOptions {
        name,
        x_title,
        blank_labels: blank_labels.clone(),
        positive_control_labels: positive_control_labels.clone(),
        target_kind: super::super::plate_editor::to_target_archetype_kind(app.grid.import_reshape.target_kind),
    };
    let (template, warnings) = curvebook_transform::build_plate_template_from_grids(&label_grid, x_grid.as_deref(), &opts).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(msg.clone());
        EngineError::with_code(ErrorCode::BadInput, msg)
    })?;

    app.grid.import_reshape.mode = match template.geometry {
        curvebook_doc::PlateGeometry::Wells96 => crate::grid_view::ImportReshapeMode::Plate96,
        curvebook_doc::PlateGeometry::Wells384 => crate::grid_view::ImportReshapeMode::Plate384,
    };
    app.grid.import_reshape.plate_roles = template.wells.iter().map(super::super::plate_editor::role_to_json).collect();
    app.grid.import_reshape.plate_layout_source = crate::grid_view::PlateLayoutSourceMode::InteractiveGrid;
    app.grid.import_reshape.plate_layout_sheet_id = Some(layout_sheet_id.as_str().to_owned());
    app.grid.import_reshape.plate_x_grid_sheet_id = x_grid_sheet_id.map(|id| id.as_str().to_owned());
    app.grid.import_reshape.plate_blank_labels = blank_labels;
    app.grid.import_reshape.plate_positive_control_labels = positive_control_labels;
    let mut status = format!("Loaded {}×{} layout from sheet", template.geometry.rows(), template.geometry.cols());
    if let Some(first_warn) = warnings.first() {
        status.push_str(&format!(" ({first_warn})"));
    }
    app.grid.import_reshape.plate_status_message = Some(status);
    app.egui_views.import_wizard.plate_subtab = import_wizard::PlateSubTab::Layout;
    Ok(UiIntentOutcome::ShellUpdated)
}

fn parse_plate_text_matrix(raw: &str, decimal_separator: char) -> Vec<Vec<String>> {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let non_empty_lines: Vec<&str> = normalized.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if non_empty_lines.is_empty() {
        return Vec::new();
    }
    let delim = if non_empty_lines.iter().any(|l| l.contains('\t')) {
        '\t'
    } else if decimal_separator == ',' && non_empty_lines.iter().any(|l| l.contains(';')) {
        ';'
    } else if non_empty_lines.iter().any(|l| l.contains(',')) {
        ','
    } else if non_empty_lines.iter().any(|l| l.contains(';')) {
        ';'
    } else {
        '\t'
    };
    let opts = curvebook_format::CsvOptions {
        delimiter: delim,
        quote_char: '"',
        decimal_separator,
        thousands_separator: None,
        header_rows: 0,
        has_row_titles: false,
        parse_asterisk_exclusions: false,
        missing_sentinels: Vec::new(),
    };
    let joined = non_empty_lines.join("\n");
    curvebook_format::parse_delimited_records(&joined, &opts)
        .unwrap_or_else(|_| non_empty_lines.iter().map(|line| line.split(delim).map(|s| s.trim().to_owned()).collect()).collect())
}

fn execute_apply_plate_grids_text(
    app: &mut UiApp,
    label_grid_text: &str,
    x_grid_text: &str,
    blank_labels: Vec<String>,
    positive_control_labels: Vec<String>,
) -> Result<UiIntentOutcome, EngineError> {
    let dec = app.locale_preset.decimal_separator;
    let label_grid = parse_plate_text_matrix(label_grid_text, dec);
    let x_grid = if x_grid_text.trim().is_empty() {
        None
    } else {
        let raw_x = parse_plate_text_matrix(x_grid_text, dec);
        let parsed: Vec<Vec<Option<f64>>> = raw_x
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|tok| {
                        let t = tok.trim();
                        if t.is_empty() {
                            None
                        } else {
                            crate::grid_view::parse_localized_number_input(t, &app.locale_preset)
                                .or_else(|| t.replace(',', ".").parse::<f64>().ok())
                                .filter(|v| v.is_finite())
                        }
                    })
                    .collect()
            })
            .collect();
        Some(parsed)
    };

    let name =
        if app.grid.import_reshape.title.trim().is_empty() { String::from("Plate Layout") } else { app.grid.import_reshape.title.trim().to_owned() };
    let x_title = if app.grid.import_reshape.x_title.trim().is_empty() {
        String::from("Concentration")
    } else {
        app.grid.import_reshape.x_title.trim().to_owned()
    };
    let opts = curvebook_transform::PlateGridBuildOptions {
        name,
        x_title,
        blank_labels: blank_labels.clone(),
        positive_control_labels: positive_control_labels.clone(),
        target_kind: super::super::plate_editor::to_target_archetype_kind(app.grid.import_reshape.target_kind),
    };
    let (template, warnings) = curvebook_transform::build_plate_template_from_grids(&label_grid, x_grid.as_deref(), &opts).map_err(|err| {
        let msg = err.to_string();
        app.grid.import_reshape.plate_status_message = Some(msg.clone());
        EngineError::with_code(ErrorCode::BadInput, msg)
    })?;

    app.grid.import_reshape.mode = match template.geometry {
        curvebook_doc::PlateGeometry::Wells96 => crate::grid_view::ImportReshapeMode::Plate96,
        curvebook_doc::PlateGeometry::Wells384 => crate::grid_view::ImportReshapeMode::Plate384,
    };
    app.grid.import_reshape.plate_roles = template.wells.iter().map(super::super::plate_editor::role_to_json).collect();
    app.grid.import_reshape.plate_layout_source = crate::grid_view::PlateLayoutSourceMode::InteractiveGrid;
    app.grid.import_reshape.plate_blank_labels = blank_labels;
    app.grid.import_reshape.plate_positive_control_labels = positive_control_labels;
    let mut status = format!("Applied {}×{} pasted plate grid", template.geometry.rows(), template.geometry.cols());
    if let Some(first_warn) = warnings.first() {
        status.push_str(&format!(" ({first_warn})"));
    }
    app.grid.import_reshape.plate_status_message = Some(status);
    app.egui_views.import_wizard.plate_subtab = import_wizard::PlateSubTab::Layout;
    Ok(UiIntentOutcome::ShellUpdated)
}
