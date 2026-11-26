#![forbid(unsafe_code)]

//! Wave 2 UI remediation regression tests covering confirmed `AUD-09` (`01`..`20`, `22`..`49`),
//! `AUD-13` (`07`, `11`, `24`), and `EXT-01` (Analyze picker half) across `dialogs_canvas`,
//! `data_grid`, `trailing_studio`, `welcome_hub`, `info_view`, `power_studio`, `results_view`,
//! `sheet_find_bar`, and `command_palette`.

use std::collections::BTreeSet;
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CiBound, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, DescriptiveSpec,
    GraphSheet, InfoConstant, InfoSheet, LayoutItem, LayoutSpec, NestedGroupColumn, NestedTable, NonlinearFitSpec, ProjectDocument, RectPt,
    ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SourceCellId, SubcolumnMode, SurvivalTable, TableArchetype,
    VariableRole, VariableSource, XColumnMode, XyTable,
};
use curvebook_engine::{ErrorCode, ExternalPathPolicy, Session};
use curvebook_stats::power::{PowerSolveTarget, PowerTestFamily};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::{
    ActiveDialog, AnalyzeDialogState, BubblePlotInspectorState, DuplicateChoice, copy_result_table_to_clipboard,
};
use curvebook_ui_egui::egui_shell::command_palette::filter_palette_commands;
use curvebook_ui_egui::egui_shell::data_grid::{
    DataGridEguiState, DataGridRenderContext, GridInlineEditTarget, ResolvedGridSelection, build_duplicate_column_intent_for_col,
    column_design_hint_tag, commit_standard_inline_edit, push_clear_selection_intent, render_data_grid_for_sheet, resolve_grid_selection,
    selected_visual_subranges,
};
use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
use curvebook_ui_egui::egui_shell::power_studio::{
    PowerResultsSubTab, PowerStudioEguiState, build_narrative_paragraph, render_power_studio_modal, sync_anova_group_means,
};
use curvebook_ui_egui::egui_shell::results_view::{
    ResultsViewEguiState, format_result_cell_display_text, render_results_view_for_sheet, visible_row_index_for_raw_row,
};
use curvebook_ui_egui::egui_shell::sheet_find_bar::{
    DataSheetFindMatch, ResultSheetFindMatch, SheetFindBarState, X_OR_TIME_COL_SENTINEL, find_matches_in_data_sheet_cached,
    find_matches_in_data_sheet_with_locale, find_matches_in_result_sheet, find_matches_in_result_sheet_cached, render_sheet_find_bar,
};
use curvebook_ui_egui::egui_shell::trailing_studio::{
    GraphStudioTab, ResultsStudioTab, StudioContextSurface, TrailingStudioContext, TrailingStudioEguiState, arrange_layout_grid_items,
    collect_info_downstream_references, render_trailing_studio, resolve_studio_surface,
};
use curvebook_ui_egui::egui_shell::welcome_hub::{
    WelcomeArchetypeChoice, WelcomeHubEguiState, WelcomeLeftMode, build_sample_creation_intent, render_welcome_hub,
};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens, UiIntent};
use curvebook_ui_egui::i18n::LocalePreset;
use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardStage};
use curvebook_ui_egui::workspace_nav::SAMPLE_FIXTURES;
use curvebook_ui_egui::{StubPlatformClipboardAdapter, UiApp, is_desktop_host_routed_command};
use egui::{Pos2, Rect, Vec2};
use serde_json::{Map, Value, json};

fn run_egui_root_pass_with_ctx<R>(
    ctx: &egui::Context,
    size: Vec2,
    input: egui::RawInput,
    mut draw: impl FnMut(&egui::Context, &mut egui::Ui) -> R,
) -> Result<R, String> {
    let mut raw = input;
    raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
    let mut out = None;
    let mut full = ctx.run_ui(raw, |ui| {
        let ctx_clone = ui.ctx().clone();
        out = Some(draw(&ctx_clone, ui));
    });
    full.textures_delta.clear();
    out.ok_or_else(|| String::from("egui root pass did not execute"))
}

fn run_egui_pass_with_ctx<R>(
    ctx: &egui::Context,
    size: Vec2,
    input: egui::RawInput,
    mut draw: impl FnMut(&egui::Context, &mut egui::Ui) -> R,
) -> Result<R, String> {
    let mut raw = input;
    raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
    let mut out = None;
    let mut full = ctx.run_ui(raw, |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let ctx_clone = ui.ctx().clone();
            out = Some(draw(&ctx_clone, ui));
        });
    });
    full.textures_delta.clear();
    out.ok_or_else(|| String::from("egui pass did not run central panel"))
}

fn run_egui_pass<R>(size: Vec2, input: egui::RawInput, draw: impl FnMut(&egui::Context, &mut egui::Ui) -> R) -> Result<R, String> {
    let ctx = egui::Context::default();
    run_egui_pass_with_ctx(&ctx, size, input, draw)
}

fn click_input(pos: Pos2) -> egui::RawInput {
    egui::RawInput {
        events: vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        ..egui::RawInput::default()
    }
}

#[test]
fn rem_aud09_01_welcome_open_file_enforces_path_policy_and_keeps_launcher_on_failure() -> Result<(), String> {
    // 1. RejectExternal disables open_file controls and never emits project.open
    let mut app_reject = UiApp::new();
    app_reject.session_mut().set_path_policy(ExternalPathPolicy::reject_external());
    let tokens = Tokens::from_theme_spec(&app_reject.theme, &app_reject.locale);
    let mut state = WelcomeHubEguiState {
        left_mode: WelcomeLeftMode::OpenFile,
        open_file_path_draft: String::from("some-project.curvebook"),
        ..Default::default()
    };
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    run_egui_pass(Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app_reject, &mut state, &tokens, true, &mut meta, &mut intents);
    })?;
    let open_btn = meta.entries.iter().find(|e| e.widget_id == "welcome.btn.open_file").ok_or("missing welcome.btn.open_file")?;
    assert!(!open_btn.enabled, "RejectExternal must disable welcome.btn.open_file");
    let open_input = meta.entries.iter().find(|e| e.widget_id == "welcome.input.open_path").ok_or("missing welcome.input.open_path")?;
    assert!(!open_input.enabled, "RejectExternal must disable welcome.input.open_path");
    assert!(intents.intents().is_empty());

    // 2. Workdir policy rejects escaping path inline without emitting project.open
    let wd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut app_wd = UiApp::new();
    let policy = ExternalPathPolicy::workdir(&wd).map_err(|e| e.to_string())?;
    app_wd.session_mut().set_path_policy(policy);
    let mut state_wd = WelcomeHubEguiState {
        left_mode: WelcomeLeftMode::OpenFile,
        open_file_path_draft: String::from("../../../../etc/passwd"),
        ..Default::default()
    };
    let ctx_wd = egui::Context::default();
    let mut meta_wd = FrameInspectMeta::default();
    let mut intents_wd = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_wd, Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app_wd, &mut state_wd, &tokens, true, &mut meta_wd, &mut intents_wd);
    })?;
    let btn_rect = meta_wd.entries.iter().find(|e| e.widget_id == "welcome.btn.open_file").ok_or("missing btn")?.clipped_rect;
    let mut meta_click = FrameInspectMeta::default();
    let mut intents_click = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_wd, Vec2::new(1100.0, 720.0), click_input(btn_rect.center()), |_ctx, ui| {
        render_welcome_hub(ui, &app_wd, &mut state_wd, &tokens, true, &mut meta_click, &mut intents_click);
    })?;
    assert!(intents_click.intents().is_empty(), "escaped workdir path must not emit project.open");
    assert!(state_wd.open_file_error.is_some(), "escaped workdir path must populate inline error");

    // 3. Failed project.open under Unconstrained must NOT dismiss launcher unconditionally
    let mut app_unc = UiApp::new();
    let mut state_unc = WelcomeHubEguiState {
        left_mode: WelcomeLeftMode::OpenFile,
        open_file_path_draft: String::from("does-not-exist-curvebook-rem-test.curvebook"),
        ..Default::default()
    };
    let ctx_unc = egui::Context::default();
    let mut m0 = FrameInspectMeta::default();
    let mut i0 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_unc, Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app_unc, &mut state_unc, &tokens, true, &mut m0, &mut i0);
    })?;
    let unc_btn_rect = m0.entries.iter().find(|e| e.widget_id == "welcome.btn.open_file").ok_or("missing btn")?.clipped_rect;
    let mut m1 = FrameInspectMeta::default();
    let mut i1 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_unc, Vec2::new(1100.0, 720.0), click_input(unc_btn_rect.center()), |_ctx, ui| {
        render_welcome_hub(ui, &app_unc, &mut state_unc, &tokens, true, &mut m1, &mut i1);
    })?;
    assert!(
        i1.intents().iter().any(|it| matches!(it, UiIntent::EngineCommand { command_id, .. } if command_id == "project.open")),
        "should emit project.open"
    );
    assert!(
        !i1.intents().iter().any(|it| matches!(it, UiIntent::SetLauncherOpen { open: false })),
        "must not emit SetLauncherOpen(false) before project.open succeeds"
    );
    let _ = i1.commit_all_if_settled(&mut app_unc, false);
    assert!(app_unc.canvas_dialogs.status_banner.is_some(), "failed project.open sets status_banner");
    let mut m2 = FrameInspectMeta::default();
    let mut i2 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_unc, Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app_unc, &mut state_unc, &tokens, true, &mut m2, &mut i2);
    })?;
    assert!(state_unc.open_file_error.is_some(), "failed open should surface inline error");
    assert!(!i2.intents().iter().any(|it| matches!(it, UiIntent::SetLauncherOpen { open: false })), "launcher stays open after failed open");
    Ok(())
}

#[test]
fn rem_aud09_03_power_studio_clipboard_adapter_and_copied_reset() -> Result<(), String> {
    let mut app = UiApp::new();
    let clip = Arc::new(StubPlatformClipboardAdapter::new());
    app.set_clipboard_adapter(clip.clone());
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    app.canvas_dialogs.power_wizard.stage = PowerWizardStage::Results;
    let _ = app.canvas_dialogs.power_wizard.recalculate();

    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut state = PowerStudioEguiState { results_tab: PowerResultsSubTab::NarrativeReport, ..Default::default() };
    let ctx_pw = egui::Context::default();
    let mut meta = FrameInspectMeta::default();
    for _ in 0..2 {
        meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        run_egui_pass_with_ctx(&ctx_pw, Vec2::new(1100.0, 760.0), egui::RawInput::default(), |ctx, _ui| {
            render_power_studio_modal(ctx, &app, &mut state, &tokens, &mut meta, &mut intents);
        })?;
    }
    let copy_rect = meta.entries.iter().find(|e| e.widget_id == "power.copy_narrative").ok_or("missing power.copy_narrative")?.clipped_rect;

    let mut meta2 = FrameInspectMeta::default();
    let mut intents2 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_pw, Vec2::new(1100.0, 760.0), click_input(copy_rect.center()), |ctx, _ui| {
        render_power_studio_modal(ctx, &app, &mut state, &tokens, &mut meta2, &mut intents2);
    })?;
    assert!(state.narrative_copied, "copy should succeed through custom clipboard adapter");
    assert!(clip.last_text().is_some_and(|t| t.contains("α =")), "clipboard adapter must receive narrative text");

    // Changing parameters changes narrative -> narrative_copied resets automatically
    app.canvas_dialogs.power_wizard.alpha = 0.01;
    let _ = app.canvas_dialogs.power_wizard.recalculate();
    let mut meta3 = FrameInspectMeta::default();
    let mut intents3 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_pw, Vec2::new(1100.0, 760.0), egui::RawInput::default(), |ctx, _ui| {
        render_power_studio_modal(ctx, &app, &mut state, &tokens, &mut meta3, &mut intents3);
    })?;
    assert!(!state.narrative_copied, "narrative_copied badge must clear when parameters/narrative change");

    // Unavailable clipboard adapter must NOT mark copied
    let mut app_unavail = UiApp::new();
    let unavail_clip = Arc::new(StubPlatformClipboardAdapter::unavailable());
    app_unavail.set_clipboard_adapter(unavail_clip.clone());
    app_unavail.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    app_unavail.canvas_dialogs.power_wizard.stage = PowerWizardStage::Results;
    let _ = app_unavail.canvas_dialogs.power_wizard.recalculate();
    let mut state_unavail = PowerStudioEguiState { results_tab: PowerResultsSubTab::NarrativeReport, ..Default::default() };
    let ctx_unavail = egui::Context::default();
    let mut meta_u0 = FrameInspectMeta::default();
    for _ in 0..2 {
        meta_u0 = FrameInspectMeta::default();
        let mut intents_u0 = PendingUiIntents::new();
        run_egui_pass_with_ctx(&ctx_unavail, Vec2::new(1100.0, 760.0), egui::RawInput::default(), |ctx, _ui| {
            render_power_studio_modal(ctx, &app_unavail, &mut state_unavail, &tokens, &mut meta_u0, &mut intents_u0);
        })?;
    }
    let u_copy_rect = meta_u0.entries.iter().find(|e| e.widget_id == "power.copy_narrative").ok_or("missing copy btn")?.clipped_rect;
    let mut meta_unavail = FrameInspectMeta::default();
    let mut intents_unavail = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_unavail, Vec2::new(1100.0, 760.0), click_input(u_copy_rect.center()), |ctx, _ui| {
        render_power_studio_modal(ctx, &app_unavail, &mut state_unavail, &tokens, &mut meta_unavail, &mut intents_unavail);
    })?;
    assert!(!unavail_clip.captured_texts().is_empty(), "click should invoke clipboard adapter");
    assert!(!state_unavail.narrative_copied, "unavailable clipboard must not report Copied");
    Ok(())
}

#[test]
fn rem_aud09_04_and_aud13_24_data_grid_selection_decomposition_and_precomputed_tag() -> Result<(), String> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "ds-grp",
            "title": "Grouped",
            "kind": "grouped",
            "cols": 2,
            "rows": 4,
            "subcolumn_mode": { "RawReplicates": 3 },
            "auto_graph": false
        }),
    )
    .map_err(|e| e.to_string())?;
    app.sync_views();
    let ds = app.document().data_sheet(&SheetId::new("ds-grp")).ok_or("missing ds-grp")?;

    // Partial subcolumn span across columns 0 (subcol 1..3) and 1 (subcol 0..2)
    let ranges = selected_visual_subranges(ds, GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 1));
    assert_eq!(ranges.len(), 2, "must decompose partial subcol span into 2 column-aligned ranges");
    assert_eq!((ranges[0].start.col, ranges[0].end.col, ranges[0].start.subcol, ranges[0].end.subcol), (0, 0, 1, 2));
    assert_eq!((ranges[1].start.col, ranges[1].end.col, ranges[1].start.subcol, ranges[1].end.subcol), (1, 1, 0, 1));

    // Discontiguous selected_coords takes precedence over bounding box
    app.grid.selected_coords = vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(3, 1, 2)];
    let ds = app.document().data_sheet(&SheetId::new("ds-grp")).ok_or("missing ds-grp")?;
    let resolved = resolve_grid_selection(&app, ds).ok_or("expected resolved selection")?;
    assert!(matches!(resolved, ResolvedGridSelection::Discontiguous(ref coords) if coords.len() == 2));

    let mut intents = PendingUiIntents::new();
    push_clear_selection_intent(ds, &resolved, &mut intents);
    assert_eq!(intents.intents().len(), 1, "clear selection must emit 1 atomic intent");

    // AUD-13-24: PrecomputedError design hint surfaces "Summary" tag even if subcolumn_mode is Single
    let col_t = ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 3).map_err(|e| e.to_string())?;
    assert_eq!(column_design_hint_tag(&col_t), "Summary");
    let col_unpaired = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    assert_eq!(column_design_hint_tag(&col_unpaired), "Unpaired");
    Ok(())
}

#[test]
fn rem_aud09_07_27_45_welcome_clone_samples_and_precomputed_radio() -> Result<(), String> {
    // AUD-09-27: Every sample fixture instantiates cleanly, preserving row titles, Nested subcol titles, and MV formulas
    for fx in SAMPLE_FIXTURES {
        let mut app = UiApp::new();
        let intent = build_sample_creation_intent(&app, fx.id, None, false, false)?;
        let mut intents = PendingUiIntents::new();
        intents.push(intent);
        let res = intents.commit_if_settled(&mut app, false);
        assert!(res.is_ok(), "sample fixture `{}` must execute cleanly: {:?}", fx.id, res);
        let ds = app.document().data_sheets.last().ok_or_else(|| format!("missing created sheet for {}", fx.id))?;
        match &ds.archetype {
            TableArchetype::PartsOfWhole(t) => {
                assert!(t.slice_labels.iter().any(|s| !s.is_empty()), "PartsOfWhole sample `{}` lost slice_labels", fx.id);
            }
            TableArchetype::Nested(t) => {
                assert!(t.groups.iter().any(|g| g.subcolumn_titles.iter().any(|s| !s.is_empty())), "Nested sample `{}` lost subcolumn_titles", fx.id);
            }
            TableArchetype::MultipleVariables(t) => {
                // Verify calculated variables preserve VariableSource::Calculated if present in fixture
                let raw =
                    curvebook_ui_egui::workspace_nav::instantiate_sample_fixture(fx.id, SheetId::new("raw-check")).map_err(|e| e.to_string())?;
                if let TableArchetype::MultipleVariables(raw_mv) = &raw.archetype {
                    for (idx, rv) in raw_mv.variables.iter().enumerate() {
                        if let VariableSource::Calculated { formula } = &rv.source {
                            assert_eq!(
                                t.variables.get(idx).map(|v| &v.source),
                                Some(&VariableSource::Calculated { formula: formula.clone() }),
                                "MV sample `{}` lost calculated variable formula",
                                fx.id
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // AUD-09-07: ClonePipeline with clear_values + keep_x must omit clear_data so X values survive
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-xy", "title": "XY Source", "kind": "xy", "cols": 1, "rows": 2, "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [1.0, 2.0] })).map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[10.0], [20.0]] })).map_err(|e| e.to_string())?;
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ClonePipeline, ..Default::default() };
    state.schema.clone_source_sheet_id = Some(SheetId::new("ds-xy"));
    state.schema.clone_clear_values = true;
    state.schema.clone_keep_x = true;

    let ctx_clone = egui::Context::default();
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_clone, Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app, &mut state, &tokens, true, &mut meta, &mut intents);
    })?;
    let create_rect = meta.entries.iter().find(|e| e.widget_id == "welcome.btn.create").ok_or("missing welcome.btn.create")?.clipped_rect;
    let mut meta2 = FrameInspectMeta::default();
    let mut intents2 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_clone, Vec2::new(1100.0, 720.0), click_input(create_rect.center()), |_ctx, ui| {
        render_welcome_hub(ui, &app, &mut state, &tokens, true, &mut meta2, &mut intents2);
    })?;
    let clone_params = intents2
        .intents()
        .iter()
        .find_map(|i| match i {
            UiIntent::EngineCommand { command_id, params } if command_id == "sheet.clonePipeline" => Some(params.clone()),
            _ => None,
        })
        .ok_or("missing sheet.clonePipeline intent")?;
    assert!(clone_params.get("clear_data").is_none(), "AUD-09-07: must not pass legacy clear_data alongside keep_x_values");
    assert_eq!(clone_params.get("keep_x_values"), Some(&json!(true)));
    assert_eq!(clone_params.get("clear_y_values"), Some(&json!(true)));

    // AUD-09-45: PrecomputedError normalizes unsupported y_subcolumn_kind (e.g. 5 = MeanLowHigh) -> 2 (MeanSdN)
    let mut state_col = WelcomeHubEguiState { left_mode: WelcomeLeftMode::NewTableArchetype(WelcomeArchetypeChoice::Column), ..Default::default() };
    state_col.schema.col_design_kind = 2;
    state_col.schema.y_subcolumn_kind = 5;
    let mut mc = FrameInspectMeta::default();
    let mut ic = PendingUiIntents::new();
    run_egui_pass(Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        render_welcome_hub(ui, &app, &mut state_col, &tokens, true, &mut mc, &mut ic);
    })?;
    assert_eq!(state_col.schema.y_subcolumn_kind, 2, "AUD-09-45: Column PrecomputedError must normalize unsupported y_subcolumn_kind to 2");
    Ok(())
}

#[test]
fn rem_aud09_08_43_info_view_notes_flush_editing_isolation_and_locale() -> Result<(), String> {
    let mut doc = ProjectDocument::new("Info View Test").map_err(|e| e.to_string())?;
    doc.info_sheets.push(InfoSheet::new_preseeded(SheetId::new("info-1"), "Info 1").map_err(|e| e.to_string())?);
    doc.info_sheets.push(InfoSheet::new_preseeded(SheetId::new("info-2"), "Info 2").map_err(|e| e.to_string())?);
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_locale("de");
    app.execute_command("info.setConstant", json!({ "sheet_id": "info-1", "name": "Dose", "value": 12.5 })).map_err(|e| e.to_string())?;
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);

    let mut state = InfoViewEguiState {
        notes_draft: Some((SheetId::new("info-1"), String::from("Updated protocol note"))),
        notes_dirty: true,
        editing_sheet_id: Some(SheetId::new("info-1")),
        editing_key: Some(String::from("Dose")),
        ..Default::default()
    };

    // AUD-09-43: read-only constant value uses German comma decimal ("12,5")
    let mut state_ro = InfoViewEguiState::default();
    let mut meta_ro = FrameInspectMeta::default();
    let mut intents_ro = PendingUiIntents::new();
    run_egui_pass(Vec2::new(900.0, 500.0), egui::RawInput::default(), |_ctx, ui| {
        render_info_view_for_sheet(ui, &app, Some(&SheetId::new("info-1")), &mut state_ro, &tokens, true, &mut meta_ro, &mut intents_ro);
    })?;
    let dose_entry = meta_ro.entries.iter().find(|e| e.label == "Dose").ok_or("missing Dose constant row")?;
    let val_str = dose_entry.value.as_ref().and_then(|v| v.get("value")).and_then(Value::as_str).unwrap_or("");
    assert!(val_str.contains("12,5"), "AUD-09-43: de locale must format read-only constant with comma decimal, got `{val_str}`");

    // AUD-09-43: Escape with enabled == false (modal open) must NOT cancel inline constant edit
    let esc_input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..egui::RawInput::default()
    };
    let mut meta_esc = FrameInspectMeta::default();
    let mut intents_esc = PendingUiIntents::new();
    run_egui_pass(Vec2::new(900.0, 500.0), esc_input, |_ctx, ui| {
        render_info_view_for_sheet(ui, &app, Some(&SheetId::new("info-1")), &mut state, &tokens, false, &mut meta_esc, &mut intents_esc);
    })?;
    assert_eq!(state.editing_key.as_deref(), Some("Dose"), "AUD-09-43: disabled InfoView must ignore Escape");

    // AUD-09-08: switching to info-2 flushes dirty notes for info-1 and clears stale editing_key/editing_sheet_id
    let mut meta_sw = FrameInspectMeta::default();
    let mut intents_sw = PendingUiIntents::new();
    run_egui_pass(Vec2::new(900.0, 500.0), egui::RawInput::default(), |_ctx, ui| {
        render_info_view_for_sheet(ui, &app, Some(&SheetId::new("info-2")), &mut state, &tokens, true, &mut meta_sw, &mut intents_sw);
    })?;
    assert!(
        intents_sw.intents().iter().any(|i| matches!(
            i,
            UiIntent::EngineCommand { command_id, params }
            if command_id == "info.setNote" && params.get("sheet_id") == Some(&json!("info-1")) && params.get("text") == Some(&json!("Updated protocol note"))
        )),
        "switching InfoSheet must flush dirty notes for previous sheet"
    );
    assert!(state.editing_key.is_none() && state.editing_sheet_id.is_none(), "switching InfoSheet must clear stale constant editing state");
    Ok(())
}

#[test]
fn rem_aud09_09_28_power_studio_anova_groups_and_solve_target_narrative() -> Result<(), String> {
    let mut app = UiApp::new();
    app.canvas_dialogs.active_dialog = ActiveDialog::PowerAndSampleSize;
    let pw = &mut app.canvas_dialogs.power_wizard;
    pw.set_family(PowerTestFamily::OneWayAnova);
    pw.effect_mode = PowerEffectInputMode::MeansAndSd;
    pw.anova_groups = 10;
    sync_anova_group_means(pw);
    assert_eq!(pw.group_means.len(), 10, "sync_anova_group_means must expand to 10 groups");

    pw.anova_groups = 3;
    sync_anova_group_means(pw);
    assert_eq!(pw.group_means.len(), 3, "sync_anova_group_means must truncate back to 3 groups");

    // AUD-09-28: narrative branches cleanly across SampleSize, Power, and DetectableEffect
    pw.set_family(PowerTestFamily::TwoSampleUnpairedT);
    for (target, expected_phrase) in [
        (PowerSolveTarget::SampleSize, "required sample size"),
        (PowerSolveTarget::Power, "achieved statistical power is"),
        (PowerSolveTarget::DetectableEffect, "minimum detectable standardized effect"),
    ] {
        pw.solve_target = target;
        let _ = pw.recalculate();
        let res = pw.result.as_ref().ok_or("expected solved power result")?;
        let text = build_narrative_paragraph(pw, res);
        assert!(text.contains(expected_phrase), "narrative for {target:?} must mention `{expected_phrase}`, got: {text}");
    }
    Ok(())
}

#[test]
fn rem_aud09_19_layout_grid_arrange_expands_rows_for_excess_panels() {
    let spec = LayoutSpec {
        items: (0..5)
            .map(|i| LayoutItem::LiveGraph {
                id: format!("p-{i}"),
                graph_sheet_id: SheetId::new(format!("g-{i}")),
                bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
                scale_percent: 100.0,
                badge_override: None,
            })
            .collect(),
        ..LayoutSpec::default()
    };
    let arranged = arrange_layout_grid_items(&spec, 2, 2, 10);
    assert_eq!(arranged.len(), 5);
    let page = spec.page_size_pt();
    for item in &arranged {
        if let LayoutItem::LiveGraph { bounds_pt: b, .. } = item {
            assert!(b.x >= 0.0 && b.y >= 0.0, "panel origin must be non-negative: {b:?}");
            assert!(b.x + b.width <= page.width + 1e-6, "panel right edge exceeds page width: {b:?} vs {}", page.width);
            assert!(b.y + b.height <= page.height + 1e-6, "panel bottom edge exceeds page height: {b:?} vs {}", page.height);
        }
    }
}

#[test]
fn rem_aud09_22_23_24_25_39_40_find_bar_and_results_view() -> Result<(), String> {
    // AUD-09-22: locale decimals + XY X column + Survival Time column + cached API
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 2)
        .map_err(|e| e.to_string())?;
    xy.x_title = String::from("Conc_uM");
    xy.x_cells[0][0] = Cell::number(12.5);
    xy.data_sets[0].cells[1][0] = Cell::number(42.5);
    let ds_xy = DataSheet::new("ds-xy-find", "XY Find", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?;

    let de = LocalePreset::for_locale("de");
    let x_title_hits = find_matches_in_data_sheet_with_locale(&ds_xy, "conc_um", false, true, &de);
    assert_eq!(x_title_hits, vec![DataSheetFindMatch::ColumnTitle { col: X_OR_TIME_COL_SENTINEL }]);
    let x_cell_hits = find_matches_in_data_sheet_with_locale(&ds_xy, "12,5", false, true, &de);
    assert_eq!(x_cell_hits, vec![DataSheetFindMatch::Cell(GridCellCoord::new(0, X_OR_TIME_COL_SENTINEL, 0))]);
    let cached_hits = find_matches_in_data_sheet_cached(7, &ds_xy, "42,5", false, true, &de);
    assert_eq!(cached_hits, vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, 0, 0))]);

    let mut surv = SurvivalTable::new(1, 2).map_err(|e| e.to_string())?;
    surv.time_title = String::from("FollowUpDays");
    surv.times[1] = Cell::number(90.0);
    let ds_surv = DataSheet::new("ds-surv-find", "Surv", TableArchetype::Survival(surv)).map_err(|e| e.to_string())?;
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_surv, "followupdays", false, true, &LocalePreset::default()),
        vec![DataSheetFindMatch::ColumnTitle { col: X_OR_TIME_COL_SENTINEL }]
    );
    assert_eq!(
        find_matches_in_data_sheet_with_locale(&ds_surv, "90", false, true, &LocalePreset::default()),
        vec![DataSheetFindMatch::Cell(GridCellCoord::new(1, X_OR_TIME_COL_SENTINEL, 0))]
    );

    // AUD-09-23: semantic formatter (ambiguous CI, blank NaN, separate dedup namespaces, empty tab guard)
    let mut nl_spec = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_spec.output_significant_digits = Some(3);
    let spec = AnalysisSpec::new("rs-find", "ds-xy-find", AnalysisKind::NonlinearRegression(nl_spec)).map_err(|e| e.to_string())?;
    let mut rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    rs.status = ResultStatus::UpToDate;
    let empty_tab = ResultTab {
        id: String::from("empty"),
        title: String::from("Empty"),
        kind: ResultTabKind::Diagnostics,
        column_headers: vec![String::from("Estimate")],
        row_labels: Vec::new(),
        rows: Vec::new(),
        footnotes: Vec::new(),
    };
    let main_tab = ResultTab {
        id: String::from("main"),
        title: String::from("Main"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Mean"), String::from("95% CI")],
        row_labels: vec![String::from("Mean"), String::from("Unreached")],
        rows: vec![
            vec![ResultCell::Text(String::from("Mean")), ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Open }],
            vec![ResultCell::Number { value: f64::NAN, prefix: None, units: None }, ResultCell::Number { value: 12.3456, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    };
    rs.tabs = vec![empty_tab, main_tab];
    assert!(find_matches_in_result_sheet(&rs, 0, false, "Estimate", false, false).is_empty(), "empty tab must not emit OOB row_idx 0");
    let amb_hits = find_matches_in_result_sheet(&rs, 1, false, "??? (ambiguous)", false, true);
    assert_eq!(amb_hits, vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 1 }]);
    assert!(find_matches_in_result_sheet(&rs, 1, false, "NaN", false, false).is_empty(), "non-finite numbers must stay blank and not match NaN");
    let mean_hits = find_matches_in_result_sheet(&rs, 1, false, "Mean", true, true);
    assert_eq!(
        mean_hits,
        vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 0 }],
        "AUD-09-23: overlapping header/row-label/cell hits at `(0, 0)` must deduplicate so Next/Prev advances cleanly"
    );
    let hdr_hit = find_matches_in_result_sheet(&rs, 1, false, "95% CI", true, true);
    assert_eq!(hdr_hit, vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 0, col_idx: 1 }]);
    let row_lbl_hit = find_matches_in_result_sheet(&rs, 1, false, "Unreached", true, true);
    assert_eq!(row_lbl_hit, vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 1, col_idx: 0 }]);
    let sig_hits = find_matches_in_result_sheet_cached(1, &rs, 1, false, "12.3", false, true);
    assert_eq!(sig_hits, vec![ResultSheetFindMatch { tab_idx: 1, row_idx: 1, col_idx: 1 }]);
    assert_eq!(format_result_cell_display_text(&rs.tabs[1].rows[0][1], &rs, &rs.tabs[1]), "??? (ambiguous)");

    // AUD-09-40: visible_row_index_for_raw_row accounts for collapsed sections
    let sec_tab = ResultTab {
        id: String::from("sec"),
        title: String::from("Sections"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("Section A"), String::from("A1"), String::from("A2"), String::from("Section B"), String::from("B1")],
        rows: vec![
            vec![ResultCell::Empty],
            vec![ResultCell::Number { value: 1.0, prefix: None, units: None }],
            vec![ResultCell::Number { value: 2.0, prefix: None, units: None }],
            vec![ResultCell::Empty],
            vec![ResultCell::Number { value: 3.0, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    };
    let mut collapsed = BTreeSet::new();
    collapsed.insert(String::from("rs-find:sec:0"));
    assert_eq!(visible_row_index_for_raw_row(&SheetId::new("rs-find"), &sec_tab, 4, &collapsed), 2);

    // AUD-09-24 & AUD-09-25: secondary split ResultSheet maintains independent tab + copy + selection sync
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 2, "rows": 3, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.setCells", json!({ "sheet_id": "ds-1", "rows": [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]] })).map_err(|e| e.to_string())?;
    app.execute_command("analysis.run", json!({ "source_sheet_id": "ds-1", "kind": AnalysisKind::DescriptiveStats(DescriptiveSpec::default()) }))
        .map_err(|e| e.to_string())?;
    let rs_id = app.document().result_sheets.first().ok_or("missing result sheet")?.id.clone();
    // Active sheet is DataSheet ("ds-1"), simulating Data + Results split view!
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut rv_state = ResultsViewEguiState::default();
    rv_state.per_sheet_active_tab.insert(rs_id.clone(), 0);
    rv_state.scroll_to_match(rs_id.clone(), 0, 0, 0);
    assert!(rv_state.selection_sync_needed, "scroll_to_match must arm selection_sync_needed");

    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    let report = run_egui_pass(Vec2::new(800.0, 500.0), egui::RawInput::default(), |_ctx, ui| {
        render_results_view_for_sheet(ui, &app, Some(&rs_id), &mut rv_state, &tokens, true, &mut meta, &mut intents)
    })?;
    assert_eq!(report.active_tab_index, 0);
    assert!(
        intents.intents().iter().any(|i| matches!(i, UiIntent::SyncResultSelection { sheet_id, tab_index: 0, .. } if sheet_id == &rs_id)),
        "AUD-09-25: scroll_to_match must emit SyncResultSelection"
    );

    // Secondary copy uses per_sheet_active_tab cleanly
    app.egui_views.results_view.per_sheet_active_tab.insert(rs_id.clone(), 0);
    let copy_out = copy_result_table_to_clipboard(&mut app, rs_id.as_str(), true);
    assert!(copy_out.handled, "copy_result_table_to_clipboard must succeed for split ResultSheet");

    // AUD-09-25 & AUD-09-24: live tab switch clears stale selection and copies the newly active tab even if bound_tab_idx still points to Tab 0
    let mut sw_doc = ProjectDocument::new("Tab Switch Copy").map_err(|e| e.to_string())?;
    let sw_ds = DataSheet::new(
        "ds-sw",
        "DS",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    sw_doc.add_data_sheet(sw_ds).map_err(|e| e.to_string())?;
    let mut sw_spec =
        AnalysisSpec::new("SW", SheetId::new("ds-sw"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    sw_spec.id = SheetId::new("res-sw");
    let mut sw_rs = ResultSheet::new(sw_spec).map_err(|e| e.to_string())?;
    sw_rs.status = ResultStatus::UpToDate;
    sw_rs.tabs = vec![
        ResultTab {
            id: String::from("t0"),
            title: String::from("TabZero"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Val")],
            row_labels: vec![String::from("R0")],
            rows: vec![vec![ResultCell::Text(String::from("STALE_TAB_ZERO"))]],
            footnotes: Vec::new(),
        },
        ResultTab {
            id: String::from("t1"),
            title: String::from("TabOne"),
            kind: ResultTabKind::Diagnostics,
            column_headers: vec![String::from("Val")],
            row_labels: vec![String::from("R0")],
            rows: vec![vec![ResultCell::Text(String::from("FRESH_TAB_ONE"))]],
            footnotes: Vec::new(),
        },
    ];
    sw_doc.add_result_sheet(sw_rs).map_err(|e| e.to_string())?;
    sw_doc.active_sheet_id = Some(SheetId::new("res-sw"));
    let mut sw_app = UiApp::from_session(Session::from_document(sw_doc));
    sw_app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    sw_app.sync_views();
    // Simulate a prior cell selection on Tab 0 (stale bound_tab_idx = Some(0))
    sw_app.canvas_dialogs.result_view.bound_sheet_id = Some(String::from("res-sw"));
    sw_app.canvas_dialogs.result_view.bound_tab_idx = Some(0);
    sw_app.canvas_dialogs.result_view.selected_anchor = Some((0, 0));
    sw_app.canvas_dialogs.result_view.selected_focus = Some((0, 0));
    let mut sw_rv = ResultsViewEguiState {
        selected_cell: Some((SheetId::new("res-sw"), 0, 0, 0)),
        selection_anchor: Some((SheetId::new("res-sw"), 0, 0, 0)),
        ..ResultsViewEguiState::default()
    };
    let sw_ctx = egui::Context::default();
    let sw_tokens = Tokens::from_theme_spec(&sw_app.theme, &sw_app.locale);
    let mut sw_meta = FrameInspectMeta::default();
    let mut sw_intents = PendingUiIntents::new();
    run_egui_pass_with_ctx(&sw_ctx, Vec2::new(800.0, 500.0), egui::RawInput::default(), |ctx, ui| {
        sw_meta.begin_pass(ctx);
        render_results_view_for_sheet(ui, &sw_app, Some(&SheetId::new("res-sw")), &mut sw_rv, &sw_tokens, true, &mut sw_meta, &mut sw_intents);
    })?;
    let tab1_rect = sw_meta.entries.iter().find(|e| e.widget_id == "result.tab.t1").ok_or("missing result.tab.t1")?.clipped_rect;
    let mut sw_meta2 = FrameInspectMeta::default();
    let mut sw_intents2 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&sw_ctx, Vec2::new(800.0, 500.0), click_input(tab1_rect.center()), |ctx, ui| {
        sw_meta2.begin_pass(ctx);
        render_results_view_for_sheet(ui, &sw_app, Some(&SheetId::new("res-sw")), &mut sw_rv, &sw_tokens, true, &mut sw_meta2, &mut sw_intents2);
    })?;
    assert!(sw_rv.selected_cell.is_none() && sw_rv.selection_anchor.is_none(), "AUD-09-25: tab click must clear local selection");
    let _ = sw_intents2.commit_all_if_settled(&mut sw_app, false);
    sw_app.egui_views.results_view = sw_rv.clone();
    let sw_copy = copy_result_table_to_clipboard(&mut sw_app, "res-sw", false);
    let copied_txt = sw_copy.command_result.as_ref().and_then(|v| v.get("copied_text")).and_then(Value::as_str).unwrap_or("");
    assert!(
        copied_txt.contains("FRESH_TAB_ONE") && !copied_txt.contains("STALE_TAB_ZERO"),
        "AUD-09-25: copy after tab switch must copy Tab 1, got `{copied_txt}`"
    );

    // AUD-09-39: render_sheet_find_bar returns state.open (true while open, false when closed)
    let mut find_state = SheetFindBarState::default();
    find_state.open_for_sheet(Some(SheetId::new("ds-1")));
    let mut fmeta = FrameInspectMeta::default();
    let mut fintents = PendingUiIntents::new();
    let still_open = run_egui_pass(Vec2::new(900.0, 500.0), egui::RawInput::default(), |ctx, _ui| {
        render_sheet_find_bar(
            ctx,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0)),
            true,
            false,
            2,
            &mut find_state,
            &tokens,
            &mut fmeta,
            &mut fintents,
        )
    })?;
    assert!(still_open, "AUD-09-39: render_sheet_find_bar must return true while bar stays open");
    find_state.close();
    let closed_ret = run_egui_pass(Vec2::new(900.0, 500.0), egui::RawInput::default(), |ctx, _ui| {
        render_sheet_find_bar(
            ctx,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0)),
            true,
            false,
            2,
            &mut find_state,
            &tokens,
            &mut fmeta,
            &mut fintents,
        )
    })?;
    assert!(!closed_ret, "AUD-09-39: render_sheet_find_bar must return false once closed");
    Ok(())
}

#[test]
fn rem_aud09_26_44_command_palette_and_aud13_07_11_ext01() -> Result<(), String> {
    // AUD-09-26: .prism/.pzfx/.xlsx imports route via desktop host commands
    for cmd in ["table.importPrism", "table.importPzfx", "table.importXlsx"] {
        assert!(is_desktop_host_routed_command(cmd), "{cmd} must be desktop-host routed");
    }

    // AUD-09-44: ui.sheet.find hidden when active sheet is InfoSheet, visible on DataSheet
    let mut doc_cp = ProjectDocument::new("Palette Find Test").map_err(|e| e.to_string())?;
    doc_cp.info_sheets.push(InfoSheet::new_preseeded(SheetId::new("info-1"), "Info").map_err(|e| e.to_string())?);
    let mut app = UiApp::from_session(Session::from_document(doc_cp));
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "xy", "cols": 1, "rows": 5, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "info-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    assert!(
        filter_palette_commands(&app, "ui.sheet.find", 24).iter().all(|r| r.command_id != "ui.sheet.find"),
        "AUD-09-44: ui.sheet.find must not route on InfoSheet"
    );
    app.execute_command("sheet.select", json!({ "sheet_id": "ds-1" })).map_err(|e| e.to_string())?;
    app.sync_views();
    assert!(
        filter_palette_commands(&app, "ui.sheet.find", 24).iter().any(|r| r.command_id == "ui.sheet.find"),
        "AUD-09-44: ui.sheet.find must be available on DataSheet"
    );

    // EXT-01: Raw positive concentrations default to agonist_vs_response_4pl; log10 concentrations default to log_agonist_vs_response_4pl
    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "values": [0.001, 0.01, 0.1, 1.0, 10.0] })).map_err(|e| e.to_string())?;
    let ds_raw = app.document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
    let mut analyze = AnalyzeDialogState::default();
    analyze.open_for_data_sheet(ds_raw);
    assert_eq!(analyze.equation_id, "agonist_vs_response_4pl", "EXT-01: raw concentration table must default to agonist_vs_response_4pl");

    app.execute_command("table.setXCells", json!({ "sheet_id": "ds-1", "values": [-9.0, -8.0, -7.0, -6.0, -5.0] })).map_err(|e| e.to_string())?;
    let ds_log = app.document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
    analyze.open_for_data_sheet(ds_log);
    assert_eq!(analyze.equation_id, "log_agonist_vs_response_4pl", "EXT-01: log10 concentration table must default to log_agonist_vs_response_4pl");

    // AUD-13-11: DuplicateAnalysisPrompt handles top-level `choice`
    app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: SheetId::new("ds-1"),
        pending_entry_id: String::from("fit.nonlinear"),
        existing_result_ids: vec![SheetId::new("rs-existing")],
    };
    let mut fields = Map::new();
    fields.insert(String::from("choice"), json!("modify_existing"));
    let mut patch_intents = PendingUiIntents::new();
    patch_intents.push(UiIntent::PatchActiveDialog { fields });
    patch_intents.commit_if_settled(&mut app, false).map_err(|e| e.to_string())?;
    assert_eq!(app.canvas_dialogs.analyze.duplicate_choice, DuplicateChoice::ModifyExisting);
    assert_eq!(app.canvas_dialogs.analyze.duplicate_target_id.as_deref(), Some("rs-existing"));

    // AUD-13-07: BubblePlotInspectorState resolves ResultSheet::analyzable_data
    let mut mv_app = UiApp::new();
    mv_app
        .execute_command(
            "sheet.createData",
            json!({
                "sheet_id": "mv-1",
                "kind": "multiple_variables",
                "cols": 3,
                "rows": 3,
                "variables": [
                    { "title": "XVar", "role": "continuous" },
                    { "title": "YVar", "role": "continuous" },
                    { "title": "Grp", "role": "categorical" }
                ],
                "auto_graph": false
            }),
        )
        .map_err(|e| e.to_string())?;
    let mv_ds = mv_app.document().data_sheet(&SheetId::new("mv-1")).ok_or("missing mv-1")?.clone();
    let mut doc = mv_app.document().clone();
    let tr_spec = AnalysisSpec::new("rs-tr", "mv-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    let mut tr_rs = ResultSheet::new(tr_spec).map_err(|e| e.to_string())?;
    tr_rs.status = ResultStatus::UpToDate;
    tr_rs.analyzable_data = Some(mv_ds.clone());
    doc.result_sheets.push(tr_rs);
    let mut gs = GraphSheet::new_uninitialized_for_data("gs-tr", &mv_ds).map_err(|e| e.to_string())?;
    gs.spec.primary_source_sheet_id = SheetId::new("rs-tr");
    let mut bubble = BubblePlotInspectorState::default();
    bubble.sync_from_graph(&gs, &doc);
    assert_eq!(bubble.available_columns.len(), 3, "AUD-13-07: BubblePlotInspector must resolve columns from ResultSheet::analyzable_data");
    Ok(())
}

#[test]
fn rem_aud09_05_06_42_data_grid_inline_edit_duplicate_col_and_factor_discard() -> Result<(), String> {
    // AUD-09-05: Comma-locale excluded inline edit ("10,5*") stays numeric + excluded; "10,5" un-excludes cleanly
    let mut app_de = UiApp::new();
    app_de.set_locale("de");
    app_de
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-de", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    app_de
        .execute_command("table.setCell", json!({ "sheet_id": "ds-de", "row": 0, "col": 0, "subcol": 0, "value": 10.5, "excluded": false }))
        .map_err(|e| e.to_string())?;
    let ds_de = app_de.document().data_sheet(&SheetId::new("ds-de")).ok_or("missing ds-de")?.clone();
    let mut intents_ex = PendingUiIntents::new();
    commit_standard_inline_edit(&app_de, &ds_de, GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)), String::from("10,5*"), &mut intents_ex);
    assert_eq!(intents_ex.intents().len(), 1, "AUD-09-05: excluded comma-decimal edit must emit table.setCell");
    intents_ex.commit_if_settled(&mut app_de, false).map_err(|e| e.to_string())?;
    let cell_after_ex = app_de.document().data_sheet(&SheetId::new("ds-de")).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).cloned();
    assert_eq!(cell_after_ex, Some(Cell { value: CellValue::Number(10.5), excluded: true }), "AUD-09-05: 10,5* must stay Number(10.5) excluded");

    let ds_de2 = app_de.document().data_sheet(&SheetId::new("ds-de")).ok_or("missing ds-de")?.clone();
    let mut intents_unex = PendingUiIntents::new();
    commit_standard_inline_edit(&app_de, &ds_de2, GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)), String::from("10,5"), &mut intents_unex);
    assert_eq!(intents_unex.intents().len(), 1, "AUD-09-05: removing * must not no-op");
    intents_unex.commit_if_settled(&mut app_de, false).map_err(|e| e.to_string())?;
    let cell_after_unex = app_de.document().data_sheet(&SheetId::new("ds-de")).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).cloned();
    assert_eq!(cell_after_unex, Some(Cell { value: CellValue::Number(10.5), excluded: false }), "AUD-09-05: removing * must clear excluded");

    // AUD-09-06 (a): Column duplicate preserves excluded cells and reverts in 1 atomic Undo step
    let mut app_dup = UiApp::new();
    app_dup
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-col", "kind": "column", "cols": 1, "rows": 2, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    app_dup.execute_command("table.setTitles", json!({ "sheet_id": "ds-col", "col": 0, "title": "Ctrl" })).map_err(|e| e.to_string())?;
    app_dup
        .execute_command("table.setCell", json!({ "sheet_id": "ds-col", "row": 0, "col": 0, "subcol": 0, "value": 42.0, "excluded": true }))
        .map_err(|e| e.to_string())?;
    app_dup
        .execute_command("table.setCell", json!({ "sheet_id": "ds-col", "row": 1, "col": 0, "subcol": 0, "value": 7.0, "excluded": false }))
        .map_err(|e| e.to_string())?;
    let ds_col = app_dup.document().data_sheet(&SheetId::new("ds-col")).ok_or("missing ds-col")?.clone();
    let dup_intent = build_duplicate_column_intent_for_col(&ds_col, 0).ok_or("expected column duplicate intent")?;
    let mut dup_intents = PendingUiIntents::new();
    dup_intents.push(dup_intent);
    dup_intents.commit_if_settled(&mut app_dup, false).map_err(|e| e.to_string())?;
    let ds_col_after = app_dup.document().data_sheet(&SheetId::new("ds-col")).ok_or("missing ds-col")?;
    assert_eq!(ds_col_after.archetype.column_count(), 2);
    assert_eq!(ds_col_after.cell(CellCoord::new(0, 1, 0)), Some(&Cell { value: CellValue::Number(42.0), excluded: true }));
    assert_eq!(ds_col_after.cell(CellCoord::new(1, 1, 0)), Some(&Cell { value: CellValue::Number(7.0), excluded: false }));
    app_dup.execute_command("edit.undo", json!({})).map_err(|e| e.to_string())?;
    assert_eq!(
        app_dup.document().data_sheet(&SheetId::new("ds-col")).map(|d| d.archetype.column_count()),
        Some(1),
        "AUD-09-06: single Undo must atomically revert duplicated column + title + exclusions"
    );

    // AUD-09-06 (b): Equal-unit multi-unit Nested preserves 3-unit group, subcolumn titles, and excluded cells;
    // unequal-unit Nested cleanly refuses instead of splitting into broken groups
    let mut app_n = UiApp::new();
    app_n
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-nest", "kind": "nested", "cols": 2, "rows": 2, "replicates": 3, "auto_graph": false }),
        )
        .map_err(|e| e.to_string())?;
    app_n
        .execute_command("table.setSubcolumnTitles", json!({ "sheet_id": "ds-nest", "col": 0, "subcol": 0, "title": "MouseA" }))
        .map_err(|e| e.to_string())?;
    app_n
        .execute_command("table.setSubcolumnTitles", json!({ "sheet_id": "ds-nest", "col": 0, "subcol": 2, "title": "MouseC" }))
        .map_err(|e| e.to_string())?;
    app_n
        .execute_command("table.setCell", json!({ "sheet_id": "ds-nest", "row": 0, "col": 0, "subcol": 2, "value": 99.0, "excluded": true }))
        .map_err(|e| e.to_string())?;
    let ds_nest = app_n.document().data_sheet(&SheetId::new("ds-nest")).ok_or("missing ds-nest")?.clone();
    let nest_intent = build_duplicate_column_intent_for_col(&ds_nest, 0).ok_or("expected equal-unit nested duplicate intent")?;
    let mut ni = PendingUiIntents::new();
    ni.push(nest_intent);
    ni.commit_if_settled(&mut app_n, false).map_err(|e| e.to_string())?;
    let ds_nest_after = app_n.document().data_sheet(&SheetId::new("ds-nest")).ok_or("missing ds-nest")?;
    let TableArchetype::Nested(nt_after) = &ds_nest_after.archetype else {
        return Err(String::from("expected Nested archetype"));
    };
    assert_eq!(nt_after.groups.len(), 3, "must append exactly 1 Nested group (not 3)");
    assert_eq!(nt_after.groups[2].biological_unit_count(), 3);
    assert_eq!(nt_after.groups[2].subcolumn_titles[0], "MouseA");
    assert_eq!(nt_after.groups[2].subcolumn_titles[2], "MouseC");
    assert_eq!(nt_after.groups[2].cells[0][2], Cell { value: CellValue::Number(99.0), excluded: true });

    let unequal_nt = NestedTable {
        row_titles: vec![String::new(); 2],
        row_titles_expanded: false,
        groups: vec![NestedGroupColumn::new("G0", 2, 2).map_err(|e| e.to_string())?, NestedGroupColumn::new("G1", 4, 2).map_err(|e| e.to_string())?],
    };
    let ds_unequal = DataSheet::new("ds-unequal", "Unequal", TableArchetype::Nested(unequal_nt)).map_err(|e| e.to_string())?;
    assert!(
        build_duplicate_column_intent_for_col(&ds_unequal, 1).is_none(),
        "AUD-09-06: unequal-unit Nested group must be refused cleanly instead of splitting"
    );

    // AUD-09-06 (c): MV calculated & categorical columns preserve formula and role
    let mut app_mv = UiApp::new();
    app_mv
        .execute_command(
            "sheet.createData",
            json!({
                "sheet_id": "ds-mv",
                "kind": "multiple_variables",
                "cols": 2,
                "rows": 2,
                "variables": [{ "title": "Val", "role": "continuous" }, { "title": "Arm", "role": "categorical" }],
                "auto_graph": false
            }),
        )
        .map_err(|e| e.to_string())?;
    app_mv.execute_command("table.setCell", json!({ "sheet_id": "ds-mv", "row": 0, "col": 0, "value": 4.0 })).map_err(|e| e.to_string())?;
    app_mv.execute_command("table.setCell", json!({ "sheet_id": "ds-mv", "row": 0, "col": 1, "value": "Treat" })).map_err(|e| e.to_string())?;
    app_mv
        .execute_command("table.setCalculatedVariable", json!({ "sheet_id": "ds-mv", "title": "Doubled", "formula": "Val * 2" }))
        .map_err(|e| e.to_string())?;
    let ds_mv = app_mv.document().data_sheet(&SheetId::new("ds-mv")).ok_or("missing ds-mv")?.clone();
    let cat_dup = build_duplicate_column_intent_for_col(&ds_mv, 1).ok_or("expected categorical dup")?;
    let calc_dup = build_duplicate_column_intent_for_col(&ds_mv, 2).ok_or("expected calculated dup")?;
    let mut mv_i = PendingUiIntents::new();
    mv_i.push(cat_dup);
    mv_i.push(calc_dup);
    mv_i.commit_all_if_settled(&mut app_mv, false);
    let TableArchetype::MultipleVariables(mv_after) = &app_mv.document().data_sheet(&SheetId::new("ds-mv")).ok_or("missing ds-mv")?.archetype else {
        return Err(String::from("expected MV"));
    };
    assert_eq!(mv_after.variables[3].role, VariableRole::Categorical);
    assert_eq!(mv_after.variables[4].source, VariableSource::Calculated { formula: String::from("Val * 2") });

    // AUD-09-42: render_factor_title_editor preserves edit buffer on `will_discard()` pass and commits on real pass
    let mut app_grp = UiApp::new();
    app_grp
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-fac", "kind": "grouped", "cols": 2, "rows": 2, "replicates": 2, "auto_graph": false }),
        )
        .map_err(|e| e.to_string())?;
    app_grp.execute_command("table.setTitles", json!({ "sheet_id": "ds-fac", "row_factor_title": "Time" })).map_err(|e| e.to_string())?;
    app_grp.sync_views();
    let tokens_grp = Tokens::from_theme_spec(&app_grp.theme, &app_grp.locale);
    let mut grid_st = DataGridEguiState::default();
    let ctx_grp = egui::Context::default();
    let mut m0 = FrameInspectMeta::default();
    let mut i0 = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_grp, Vec2::new(900.0, 500.0), egui::RawInput::default(), |ctx, ui| {
        m0.begin_pass(ctx);
        render_data_grid_for_sheet(
            ui,
            &app_grp,
            Some(&SheetId::new("ds-fac")),
            &mut grid_st,
            DataGridRenderContext::new(&tokens_grp, false),
            &mut m0,
            &mut i0,
        );
    })?;
    let band_rect = m0.entries.iter().find(|e| e.widget_id == "grid.row_titles.band").ok_or("missing grid.row_titles.band")?.clipped_rect;
    // Double-click to open factor title editor
    let dbl_click = egui::RawInput {
        events: vec![
            egui::Event::PointerMoved(band_rect.center()),
            egui::Event::PointerButton {
                pos: band_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: band_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: band_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: band_rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..egui::RawInput::default()
    };
    let mut m_open = FrameInspectMeta::default();
    let mut i_open = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_grp, Vec2::new(900.0, 500.0), dbl_click, |_ctx, ui| {
        render_data_grid_for_sheet(
            ui,
            &app_grp,
            Some(&SheetId::new("ds-fac")),
            &mut grid_st,
            DataGridRenderContext::new(&tokens_grp, false),
            &mut m_open,
            &mut i_open,
        );
    })?;
    // Type new text on a normal frame so draft != original
    let type_input = egui::RawInput { events: vec![egui::Event::Text(String::from("_Updated"))], ..egui::RawInput::default() };
    let mut m_type = FrameInspectMeta::default();
    let mut i_type = PendingUiIntents::new();
    run_egui_pass_with_ctx(&ctx_grp, Vec2::new(900.0, 500.0), type_input, |_ctx, ui| {
        render_data_grid_for_sheet(
            ui,
            &app_grp,
            Some(&SheetId::new("ds-fac")),
            &mut grid_st,
            DataGridRenderContext::new(&tokens_grp, false),
            &mut m_type,
            &mut i_type,
        );
    })?;
    assert_eq!(m_type.entries.iter().find(|e| e.widget_id == "grid.row_titles.band").map(|e| e.role.as_str()), Some("textbox"));
    // Press Enter during a `will_discard()` pass: must NOT consume/drop the edit buffer!
    let enter_input = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..egui::RawInput::default()
    };
    let mut m_disc = FrameInspectMeta::default();
    let mut i_disc = PendingUiIntents::new();
    let mut pass_idx = 0usize;
    let mut pass0_had_discard = false;
    let mut pass0_intents = usize::MAX;
    run_egui_pass_with_ctx(&ctx_grp, Vec2::new(900.0, 500.0), enter_input, |ctx, ui| {
        m_disc.begin_pass(ctx);
        i_disc = PendingUiIntents::new();
        if pass_idx == 0 {
            ctx.request_discard("test discard pass");
            pass0_had_discard = ctx.will_discard();
        }
        render_data_grid_for_sheet(
            ui,
            &app_grp,
            Some(&SheetId::new("ds-fac")),
            &mut grid_st,
            DataGridRenderContext::new(&tokens_grp, false),
            &mut m_disc,
            &mut i_disc,
        );
        if pass_idx == 0 {
            pass0_intents = i_disc.intents().len();
        }
        pass_idx += 1;
    })?;
    assert!(pass0_had_discard && pass_idx >= 2, "egui must execute discarded pass 0 followed by settled pass 1");
    assert_eq!(pass0_intents, 0, "AUD-09-42: discarded pass 0 must not emit/drop factor title commit");
    assert!(
        i_disc.intents().iter().any(|it| matches!(it, UiIntent::EngineCommand { command_id, params } if command_id == "table.setTitles" && params.get("row_factor_title").is_some())),
        "AUD-09-42: settled pass 1 after discard must commit surviving factor title edit"
    );
    Ok(())
}

#[test]
fn rem_aud09_18_20_41_46_trailing_studio_and_aud09_48_bad_input() -> Result<(), String> {
    let layout = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: 1440.0,
        viewport_h_pt: 900.0,
        empty_workspace: false,
        navigator_open: true,
        inspector_open: true,
        family_open: true,
        leading_user_collapsed: false,
        trailing_user_collapsed: false,
        leading_width_override_pt: None,
        trailing_width_override_pt: None,
        host_draws_menu_bar: false,
        is_rtl: false,
    });

    // AUD-09-18: MV Trailing Studio `+ New` chip creates a new calculated column without clobbering existing columns
    let mut mv_app = UiApp::new();
    mv_app
        .execute_command(
            "sheet.createData",
            json!({
                "sheet_id": "mv-18",
                "kind": "multiple_variables",
                "cols": 2,
                "rows": 2,
                "variables": [{ "title": "A", "role": "continuous" }, { "title": "B", "role": "continuous" }],
                "auto_graph": false
            }),
        )
        .map_err(|e| e.to_string())?;
    mv_app.execute_command("sheet.select", json!({ "sheet_id": "mv-18" })).map_err(|e| e.to_string())?;
    mv_app.sync_views();
    let tokens_mv = Tokens::from_theme_spec(&mv_app.theme, &mv_app.locale);
    let mut ts_mv = TrailingStudioEguiState::default();
    let ctx_mv = egui::Context::default();
    let mut m_mv0 = FrameInspectMeta::default();
    let mut i_mv0 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        m_mv0.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv0, &mut i_mv0);
    })?;
    let new_chip_rect =
        m_mv0.entries.iter().find(|e| e.widget_id == "inspector.mv.new_calc_col").ok_or("missing inspector.mv.new_calc_col")?.clipped_rect;
    let mut m_mv1 = FrameInspectMeta::default();
    let mut i_mv1 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), click_input(new_chip_rect.center()), |ctx, ui| {
        m_mv1.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv1, &mut i_mv1);
    })?;
    assert_eq!(ts_mv.mv_selected_col, 2, "AUD-09-18: + New chip selects col_count slot");
    ts_mv.mv_formula_draft = String::from("[A] + [B]");
    let mut m_mv2 = FrameInspectMeta::default();
    let mut i_mv2 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        m_mv2.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv2, &mut i_mv2);
    })?;
    let add_btn_rect =
        m_mv2.entries.iter().find(|e| e.widget_id == "inspector.mv.apply_formula").ok_or("missing inspector.mv.apply_formula")?.clipped_rect;
    let mut m_mv3 = FrameInspectMeta::default();
    let mut i_mv3 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), click_input(add_btn_rect.center()), |ctx, ui| {
        m_mv3.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv3, &mut i_mv3);
    })?;
    assert!(
        i_mv3.intents().iter().any(|it| matches!(it, UiIntent::EngineCommand { command_id, params } if command_id == "table.setCalculatedVariable" && params.get("formula") == Some(&json!("[A] + [B]")) && params.get("col").is_none())),
        "AUD-09-18: + New slot apply must emit table.setCalculatedVariable without overwriting existing col"
    );
    // Also verify selecting an existing Entered column offers `inspector.mv.add_new_formula` without clobbering col 0
    ts_mv.mv_selected_col = 0;
    ts_mv.mv_synced_col = Some(0);
    ts_mv.mv_formula_draft = String::from("[A] * 2");
    let mut m_mv4 = FrameInspectMeta::default();
    let mut i_mv4 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        m_mv4.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv4, &mut i_mv4);
    })?;
    let add_new_rect =
        m_mv4.entries.iter().find(|e| e.widget_id == "inspector.mv.add_new_formula").ok_or("missing inspector.mv.add_new_formula")?.clipped_rect;
    let mut m_mv5 = FrameInspectMeta::default();
    let mut i_mv5 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_mv, Vec2::new(1200.0, 800.0), click_input(add_new_rect.center()), |ctx, ui| {
        m_mv5.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_mv,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &mv_app, &mut ts_mv, ctx_in, &mut m_mv5, &mut i_mv5);
    })?;
    assert!(
        i_mv5.intents().iter().any(|it| matches!(it, UiIntent::EngineCommand { command_id, params } if command_id == "table.setCalculatedVariable" && params.get("formula") == Some(&json!("[A] * 2")) && params.get("col").is_none())),
        "AUD-09-18: Add as New Calculated Column from entered col must omit col"
    );

    // AUD-09-20: info_sheet_id filter + blank Date emits Value::Null without `kind` + Remove Constant gated on draft name match
    let mut info_doc = ProjectDocument::new("Info Hooks").map_err(|e| e.to_string())?;
    let mut info_a = InfoSheet::new_preseeded(SheetId::new("info-a"), "Info A").map_err(|e| e.to_string())?;
    info_a.constants.push(InfoConstant::new("RunDate", ConstantValue::Date(String::from("2026-05-01")), None).map_err(|e| e.to_string())?);
    info_a.constants.push(InfoConstant::new("Factor", ConstantValue::Number(2.0), None).map_err(|e| e.to_string())?);
    let mut info_b = InfoSheet::new_preseeded(SheetId::new("info-b"), "Info B").map_err(|e| e.to_string())?;
    info_b.constants.push(InfoConstant::new("Factor", ConstantValue::Number(5.0), None).map_err(|e| e.to_string())?);
    info_doc.info_sheets.push(info_a.clone());
    info_doc.info_sheets.push(info_b.clone());
    let mut mv_bound = curvebook_doc::MultipleVariablesTable::new(2, 2).map_err(|e| e.to_string())?;
    mv_bound.variables[0].title = String::from("A");
    mv_bound.variables[1].title = String::from("Scaled");
    mv_bound.variables[1].role = VariableRole::Calculated;
    mv_bound.variables[1].source = VariableSource::Calculated { formula: String::from("[A] * Info(\"Factor\")") };
    let mut ds_bound = DataSheet::new("ds-bound", "Bound", TableArchetype::MultipleVariables(mv_bound)).map_err(|e| e.to_string())?;
    ds_bound.info_sheet_id = Some(SheetId::new("info-a"));
    info_doc.data_sheets.push(ds_bound);
    info_doc.active_sheet_id = Some(SheetId::new("info-a"));
    assert_eq!(collect_info_downstream_references(&info_doc, &info_a).len(), 1);
    assert!(collect_info_downstream_references(&info_doc, &info_b).is_empty(), "AUD-09-20: unbound InfoSheet must not list unrelated DataSheet");

    let info_app = UiApp::from_session(Session::from_document(info_doc));
    let tokens_info = Tokens::from_theme_spec(&info_app.theme, &info_app.locale);
    let mut ts_info = TrailingStudioEguiState {
        info_selected_constant: Some(String::from("RunDate")),
        info_synced_key: Some((SheetId::new("info-a"), String::from("RunDate"))),
        info_name_draft: String::from("RunDate"),
        info_value_draft: String::new(),
        info_kind_draft: 2, // Date cleared to blank
        ..Default::default()
    };
    let ctx_info = egui::Context::default();
    let mut mi0 = FrameInspectMeta::default();
    let mut ii0 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_info, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        mi0.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_info,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &info_app, &mut ts_info, ctx_in, &mut mi0, &mut ii0);
    })?;
    let save_c_rect = mi0.entries.iter().find(|e| e.widget_id == "inspector.info.save_const").ok_or("missing save_const")?.clipped_rect;
    let mut mi1 = FrameInspectMeta::default();
    let mut ii1 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_info, Vec2::new(1200.0, 800.0), click_input(save_c_rect.center()), |ctx, ui| {
        mi1.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_info,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &info_app, &mut ts_info, ctx_in, &mut mi1, &mut ii1);
    })?;
    let save_params = ii1
        .intents()
        .iter()
        .find_map(|it| match it {
            UiIntent::EngineCommand { command_id, params } if command_id == "info.setConstant" => Some(params.clone()),
            _ => None,
        })
        .ok_or("missing info.setConstant")?;
    assert_eq!(save_params.get("value"), Some(&Value::Null), "AUD-09-20: blank Date must emit null value");
    assert!(save_params.get("kind").is_none(), "AUD-09-20: blank Date must omit `kind` so engine clears cleanly");

    ts_info.info_name_draft = String::from("DifferentName");
    let mut mi2 = FrameInspectMeta::default();
    let mut ii2 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_info, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        mi2.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_info,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &info_app, &mut ts_info, ctx_in, &mut mi2, &mut ii2);
    })?;
    assert!(
        mi0.entries.iter().any(|e| e.widget_id == "inspector.info.remove_const"),
        "AUD-09-20: Remove Constant should render when draft name matches selected constant"
    );
    assert!(
        mi2.entries.iter().all(|e| e.widget_id != "inspector.info.remove_const"),
        "AUD-09-20: Remove Constant must hide when draft name does not match selected constant"
    );

    // AUD-09-41: deleted brushed sheet falls back to GraphInspector; no positional series[brushed.col] fallback
    let mut g_app = UiApp::new();
    g_app
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-g41", "kind": "column", "cols": 2, "rows": 2, "auto_graph": true, "graph_sheet_id": "g-41" }),
        )
        .map_err(|e| e.to_string())?;
    let mut g_doc = g_app.document().clone();
    if let Some(gs) = g_doc.graph_sheet_mut(&SheetId::new("g-41"))
        && gs.spec.series.len() >= 2
    {
        gs.spec.series.remove(0); // Only series[0] with data_set_index == 1 remains
    }
    g_doc.active_sheet_id = Some(SheetId::new("g-41"));
    let mut g_app = UiApp::from_session(Session::from_document(g_doc));
    g_app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("deleted-sheet"), row: 0, col: 0, subcol: 0 });
    assert_eq!(
        resolve_studio_surface(&g_app),
        StudioContextSurface::GraphInspector,
        "AUD-09-41: deleted brushed sheet must fall back to GraphInspector"
    );

    g_app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-g41"), row: 0, col: 0, subcol: 0 });
    let tokens_g = Tokens::from_theme_spec(&g_app.theme, &g_app.locale);
    let mut ts_g = TrailingStudioEguiState::default();
    let ctx_g = egui::Context::default();
    let mut mg = FrameInspectMeta::default();
    let mut ig = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_g, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        mg.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_g,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &g_app, &mut ts_g, ctx_in, &mut mg, &mut ig);
    })?;
    let fmt_btn = mg.entries.iter().find(|e| e.widget_id == "inspector.drill_back.format_point").ok_or("missing format_point")?;
    assert!(!fmt_btn.enabled, "AUD-09-41: brushed col 0 must not fall back positionally to series[0] (data_set_index 1)");

    // AUD-09-46 (a): GraphInspector Y2 target uses fresh AxisSpec::linear("Right Y") instead of cloning y_axis
    g_app.brushed_cell = None;
    let mut ts_y2 = TrailingStudioEguiState { graph_tab: GraphStudioTab::Axes, graph_axis_target: 2, ..Default::default() };
    let ctx_y2 = egui::Context::default();
    let mut my0 = FrameInspectMeta::default();
    let mut iy0 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_y2, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        my0.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_g,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &g_app, &mut ts_y2, ctx_in, &mut my0, &mut iy0);
    })?;
    let log10_rect = my0.entries.iter().find(|e| e.widget_id == "inspector.graph.scale.Log10").ok_or("missing scale.Log10")?.clipped_rect;
    let mut my1 = FrameInspectMeta::default();
    let mut iy1 = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_y2, Vec2::new(1200.0, 800.0), click_input(log10_rect.center()), |ctx, ui| {
        my1.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_g,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &g_app, &mut ts_y2, ctx_in, &mut my1, &mut iy1);
    })?;
    let y2_title = iy1.intents().iter().find_map(|it| match it {
        UiIntent::EngineCommand { command_id, params } if command_id == "graph.setAxes" => {
            params.get("y2_axis").and_then(|a| a.get("title")).and_then(Value::as_str).map(str::to_owned)
        }
        _ => None,
    });
    assert_eq!(y2_title.as_deref(), Some("Right Y"), "AUD-09-46: new Y2 axis must initialize from AxisSpec::linear(\"Right Y\")");

    // AUD-09-46 (b): ResultsInterpret Methods tab resets sticky last_copied_methods when format/report changes
    let mut r_app = UiApp::new();
    r_app
        .execute_command("sheet.createData", json!({ "sheet_id": "ds-r46", "kind": "column", "cols": 2, "rows": 3, "auto_graph": false }))
        .map_err(|e| e.to_string())?;
    r_app
        .execute_command("table.setCells", json!({ "sheet_id": "ds-r46", "rows": [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0]] }))
        .map_err(|e| e.to_string())?;
    r_app
        .execute_command("analysis.run", json!({ "source_sheet_id": "ds-r46", "kind": AnalysisKind::DescriptiveStats(DescriptiveSpec::default()) }))
        .map_err(|e| e.to_string())?;
    let r46_id = r_app.document().result_sheets.first().ok_or("missing r46")?.id.clone();
    r_app.execute_command("sheet.select", json!({ "sheet_id": r46_id.as_str() })).map_err(|e| e.to_string())?;
    r_app.sync_views();
    let tokens_r = Tokens::from_theme_spec(&r_app.theme, &r_app.locale);
    let mut ts_r = TrailingStudioEguiState {
        results_tab: ResultsStudioTab::Methods,
        methods_format: String::from("markdown"),
        last_copied_methods: true,
        last_copied_methods_text: String::from("stale previous methods text"),
        ..Default::default()
    };
    let ctx_r = egui::Context::default();
    let mut mr = FrameInspectMeta::default();
    let mut ir = PendingUiIntents::new();
    run_egui_root_pass_with_ctx(&ctx_r, Vec2::new(1200.0, 800.0), egui::RawInput::default(), |ctx, ui| {
        mr.begin_pass(ctx);
        let ctx_in = TrailingStudioContext {
            tokens: &tokens_r,
            layout,
            enabled: true,
            focused: true,
            selected_panel_index: None,
            selected_graph_annotation: None,
        };
        render_trailing_studio(ui, &r_app, &mut ts_r, ctx_in, &mut mr, &mut ir);
    })?;
    assert!(!ts_r.last_copied_methods, "AUD-09-46: stale last_copied_methods must reset when live report text differs");

    // AUD-09-48: 2^64 float rejected in SimulateDistributions (`n`) and RandomSampleRows (`sample_size`) with ErrorCode::BadInput
    let mut app_48 = UiApp::new();
    app_48.canvas_dialogs.active_dialog = ActiveDialog::SimulateDistributions;
    let mut f_sim = Map::new();
    f_sim.insert(String::from("n"), json!(18_446_744_073_709_551_616.0_f64));
    let mut pi_sim = PendingUiIntents::new();
    pi_sim.push(UiIntent::PatchActiveDialog { fields: f_sim });
    let err_sim = pi_sim.commit_if_settled(&mut app_48, false).expect_err("2^64 float must fail");
    assert_eq!(err_sim.code, ErrorCode::BadInput, "AUD-09-48: SimulateDistributions 2^64 float must return BadInput, got {err_sim:?}");

    app_48.canvas_dialogs.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(SheetId::new("ds-r46")) };
    let mut f_rs = Map::new();
    f_rs.insert(String::from("sample_size"), json!(18_446_744_073_709_551_616.0_f64));
    let mut pi_rs = PendingUiIntents::new();
    pi_rs.push(UiIntent::PatchActiveDialog { fields: f_rs });
    let err_rs = pi_rs.commit_if_settled(&mut app_48, false).expect_err("2^64 float must fail");
    assert_eq!(err_rs.code, ErrorCode::BadInput, "AUD-09-48: RandomSampleRows 2^64 float must return BadInput, got {err_rs:?}");
    Ok(())
}

#[test]
fn wave3_aud09_22_center_stage_frame1_sheet_find_uses_app_locale_preset() -> Result<(), String> {
    use curvebook_ui_egui::egui_shell::EguiShellViewState;
    use curvebook_ui_egui::egui_shell::center_stage::{CenterStageContext, render_center_stage};
    use curvebook_ui_egui::egui_shell::inspect_bridge::HybridCoexistenceArbiter;
    use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;

    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).map_err(|e| e.to_string())?;
    tbl.data_sets[0].title = String::from("Dose");
    tbl.data_sets[0].cells[0][0] = Cell::number(12.5);
    tbl.data_sets[0].cells[1][0] = Cell::number(99.0);
    let mut doc = ProjectDocument::new("German Find").map_err(|e| e.to_string())?;
    doc.add_data_sheet(DataSheet::new("ds-de", "Messwerte", TableArchetype::Column(tbl)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-de"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    let _ = app.set_locale("de");
    app.sync_views();

    let mut views = EguiShellViewState::default();
    views.sheet_find.open_for_sheet(Some(SheetId::new("ds-de")));
    views.sheet_find.query = String::from("12,5");

    // Deliberately pass English `Tokens::default()` to prove `center_stage` uses `&app.locale_preset`
    // directly on Frame 1 rather than waiting for `render_sheet_find_bar`'s `tokens.locale` fallback.
    let tokens = Tokens::default();
    let layout = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: 1100.0,
        viewport_h_pt: 720.0,
        empty_workspace: false,
        navigator_open: false,
        inspector_open: false,
        family_open: false,
        leading_user_collapsed: false,
        trailing_user_collapsed: false,
        leading_width_override_pt: None,
        trailing_width_override_pt: None,
        host_draws_menu_bar: false,
        is_rtl: false,
    });
    let decision = HybridCoexistenceArbiter::evaluate_with_views(&app, &views, false);
    let mut meta = FrameInspectMeta::default();
    let mut intents = PendingUiIntents::new();
    run_egui_pass(Vec2::new(1100.0, 720.0), egui::RawInput::default(), |_ctx, ui| {
        let _ = render_center_stage(
            ui,
            &app,
            &mut views,
            CenterStageContext { tokens: &tokens, decision: &decision, layout, focused_pane: ActiveWorkspacePane::MainCanvasOrGrid, enabled: true },
            &mut meta,
            &mut intents,
        );
    })?;
    assert_eq!(views.sheet_find.last_match_count, 1, "AUD-09-22: frame-1 center_stage find must match localized `12,5` immediately");
    assert!(views.column_grid.active_find_matches.is_some(), "AUD-09-22: frame-1 highlight overlay must be populated immediately");
    Ok(())
}
