#![forbid(unsafe_code)]

//! Wave B (`R15.6`) human-path regression tests for:
//! - B5 (`R15-GUI-19`): uncapped rectangular clipboard copy/cut (> 4,096 cells) and exact
//!   discontiguous multi-cell copy/cut without over-clearing the enclosing bounding box.
//! - B6 (`R15-GUI-20`): explicit `ImportPasteWizardState::target_sheet_id` separation from
//!   `source_sheet_id`, non-data active sheet safety, and self-link rejection.
//! - B10 (`R15-GUI-14`, `R15-GUI-25`, `R15-GUI-26`): Welcome Hub `.curvebook` i18n wording,
//!   staged-until-OK Preferences shortcuts (Cancel restores & clears draft; OK applies),
//!   and removal of unwired Graph Defaults / dead Recovery controls.

use curvebook_doc::{Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, GraphSheet, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::app_modals::PreferencesCategory;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Key, Modifiers};

struct ShellTestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_intent_error: Option<String>,
}

impl ShellTestState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::default(), last_intent_error: None }
    }
}

fn make_shell_driver(doc: ProjectDocument, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let app = UiApp::from_session(Session::from_document(doc));
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        ShellTestState::new(app),
        ViewportMetrics::new(w, h, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, st| {
            let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
        },
    );
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_intent_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn commit_pending(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
        st.last_intent_error = Some(err.to_string());
    }
    st.views = st.app.egui_views.clone();
    driver.run_settled();
}

fn cell_number(ds: &DataSheet, row: usize, col: usize) -> Option<f64> {
    match ds.cell(CellCoord::new(row, col, 0))?.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

fn build_large_column_doc(rows: usize) -> Result<ProjectDocument, String> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, rows).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Group A");
    col.data_sets[1].title = String::from("Group B");
    for r in 0..rows {
        col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        col.data_sets[1].cells[r][0] = Cell::number((10_000 + r + 1) as f64);
    }
    let mut doc = ProjectDocument::new("Large Clipboard Regression").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("ds-large", "Large Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-large"));
    Ok(doc)
}

fn build_3x6_column_doc() -> Result<ProjectDocument, String> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 6).map_err(|e| e.to_string())?;
    col.data_sets[0].title = String::from("Col A");
    col.data_sets[1].title = String::from("Col B");
    col.data_sets[2].title = String::from("Col C");
    for r in 0..6 {
        for c in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number(((r + 1) * 10 + (c + 1)) as f64);
        }
    }
    let mut doc = ProjectDocument::new("Discontiguous Cut Regression").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("ds-discontig", "Discontiguous Table", TableArchetype::Column(col)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-discontig"));
    Ok(doc)
}

#[test]
fn b5_r15_gui_19_large_2x5000_selection_copies_and_cuts_all_10000_cells_without_truncation() -> Result<(), String> {
    let rows = 5_000usize;
    let mut driver = make_shell_driver(build_large_column_doc(rows)?, 1280.0, 800.0);

    // Click A1 on the rendered grid to establish focus and selection anchor, then extend selection
    // to B5000 (10,000 cells > MAX_MATERIALIZED_SELECTION_COORDS = 4,096).
    driver.click_widget_id("grid.cell.0.0.0")?;
    {
        let st = driver.state_mut();
        let ds = st.app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large")?.clone();
        st.app.grid.extend_shift_range(&ds, GridCellCoord::new(rows - 1, 1, 0));
    }
    driver.run_settled();

    // Drive Cmd/Ctrl+C through the rendered grid widget.
    driver.press_egui_key(Key::C, Modifiers::COMMAND);
    let copied = driver.state().app.interaction.last_clipboard_text.clone().ok_or("missing clipboard text after large copy")?;
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), rows, "2x5,000 selection must copy all 5,000 rows without 4,096-cell truncation");
    assert_eq!(lines.first().copied(), Some("1\t10001"));
    assert_eq!(lines.last().copied(), Some("5000\t15000"));

    // Drive Cmd/Ctrl+X through the rendered grid widget and confirm row 4,999 is also cleared.
    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-large")).ok_or("missing ds-large after cut")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty), "row 0 col 0 must be cleared by Cut");
    assert!(ds.cell(CellCoord::new(2_500, 1, 0)).is_some_and(Cell::is_empty), "row 2,500 col 1 (> 4,096 cells) must be cleared by Cut");
    assert!(ds.cell(CellCoord::new(rows - 1, 0, 0)).is_some_and(Cell::is_empty), "row 4,999 col 0 must be cleared by Cut");
    assert!(ds.cell(CellCoord::new(rows - 1, 1, 0)).is_some_and(Cell::is_empty), "row 4,999 col 1 must be cleared by Cut");
    Ok(())
}

#[test]
fn b5_r15_gui_19_discontiguous_cut_clears_only_selected_cells_and_leaves_b2_intact() -> Result<(), String> {
    let mut driver = make_shell_driver(build_3x6_column_doc()?, 1280.0, 800.0);

    // Click A1 (row 0, col 0) through the rendered grid, then toggle C5 (row 4, col 2) into a
    // discontiguous multi-cell selection (B2 = row 1, col 1 has value 22 and is NOT selected).
    driver.click_widget_id("grid.cell.0.0.0")?;
    {
        let st = driver.state_mut();
        let ds = st.app.document().data_sheet(&SheetId::new("ds-discontig")).ok_or("missing ds-discontig")?.clone();
        st.app.grid.toggle_cmd_selection(&ds, GridCellCoord::new(4, 2, 0));
    }
    driver.run_settled();

    // Press Cmd/Ctrl+X through the rendered grid widget.
    driver.press_egui_key(Key::X, Modifiers::COMMAND);
    let copied = driver.state().app.interaction.last_clipboard_text.clone().ok_or("missing clipboard text after discontiguous cut")?;
    assert!(copied.contains("11"), "clipboard TSV must contain A1 (11), got: {copied:?}");
    assert!(copied.contains("53"), "clipboard TSV must contain C5 (53), got: {copied:?}");
    assert!(!copied.contains("22"), "clipboard TSV must not leak unselected B2 (22), got: {copied:?}");

    let ds = driver.state().app.document().data_sheet(&SheetId::new("ds-discontig")).ok_or("missing ds-discontig after cut")?;
    assert!(ds.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty), "selected A1 must be cleared");
    assert!(ds.cell(CellCoord::new(4, 2, 0)).is_some_and(Cell::is_empty), "selected C5 must be cleared");
    assert_eq!(cell_number(ds, 1, 1), Some(22.0), "unselected B2 inside enclosing bounding box must stay untouched");
    assert_eq!(cell_number(ds, 0, 2), Some(13.0), "unselected C1 inside enclosing bounding box must stay untouched");
    assert_eq!(cell_number(ds, 4, 0), Some(51.0), "unselected A5 inside enclosing bounding box must stay untouched");
    Ok(())
}

fn build_two_sheets_and_graph_doc() -> Result<ProjectDocument, String> {
    let mut col_a = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    col_a.data_sets[0].title = String::from("Ctrl");
    col_a.data_sets[1].title = String::from("Drug");
    col_a.data_sets[0].cells[0][0] = Cell::number(10.0);
    col_a.data_sets[0].cells[1][0] = Cell::number(20.0);
    col_a.data_sets[0].cells[2][0] = Cell::number(30.0);
    col_a.data_sets[1].cells[0][0] = Cell::number(40.0);
    col_a.data_sets[1].cells[1][0] = Cell::number(50.0);
    col_a.data_sets[1].cells[2][0] = Cell::number(60.0);

    let mut col_b = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    col_b.data_sets[0].title = String::from("Target 1");
    col_b.data_sets[1].title = String::from("Target 2");

    let mut doc = ProjectDocument::new("Import Target Separation Regression").map_err(|e| e.to_string())?;
    let ds_a = DataSheet::new("ds-a", "Sheet A", TableArchetype::Column(col_a)).map_err(|e| e.to_string())?;
    let ds_b = DataSheet::new("ds-b", "Sheet B", TableArchetype::Column(col_b)).map_err(|e| e.to_string())?;
    let graph = GraphSheet::new_uninitialized_for_data("graph-1", &ds_a).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds_a).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds_b).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph).map_err(|e| e.to_string())?;
    doc.active_sheet_id = Some(SheetId::new("ds-a"));
    Ok(doc)
}

#[test]
fn b6_r15_gui_20_paste_link_explicit_target_and_graph_active_no_self_link() -> Result<(), String> {
    // Scenario A: Sheet A is active, Paste Link is opened explicitly for target Sheet B.
    // Picking Sheet A as the internal link source and clicking Import must link into B (NOT A).
    {
        let mut driver = make_shell_driver(build_two_sheets_and_graph_doc()?, 1280.0, 800.0);
        driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::InternalDataSheet, true, Some(SheetId::new("ds-b")));
        commit_pending(&mut driver);

        assert_eq!(driver.state().app.grid.import_wizard.target_sheet_id, Some(SheetId::new("ds-b")));
        assert_eq!(driver.state().app.grid.import_wizard.source_sheet_id, None, "opening wizard for target B must not overwrite source_sheet_id");

        driver.click_widget_id("import_paste.source_sheet.ds-a")?;
        driver.click_widget_id("modal.ok")?;

        assert!(driver.state().last_intent_error.is_none(), "unexpected error: {:?}", driver.state().last_intent_error);
        assert!(!driver.state().app.grid.import_wizard.open, "wizard should close after committing to target B");

        let ds_a = driver.state().app.document().data_sheet(&SheetId::new("ds-a")).ok_or("missing ds-a")?;
        let ds_b = driver.state().app.document().data_sheet(&SheetId::new("ds-b")).ok_or("missing ds-b")?;
        assert!(ds_a.linked_regions.is_empty(), "source Sheet A must not receive linked regions");
        assert_eq!(ds_b.linked_regions.len(), 1, "explicit target Sheet B must receive the live link");
        assert_eq!(cell_number(ds_b, 0, 0), Some(10.0));
    }

    // Scenario B: Graph G is active, wizard opened with no explicit target.
    // Picking Sheet A as link source and clicking Import must NOT overwrite or self-link Sheet A.
    {
        let mut doc = build_two_sheets_and_graph_doc()?;
        doc.active_sheet_id = Some(SheetId::new("graph-1"));
        let mut driver = make_shell_driver(doc, 1280.0, 800.0);

        driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::InternalDataSheet, true, None);
        commit_pending(&mut driver);

        assert_eq!(driver.state().app.grid.import_wizard.target_sheet_id, None);
        driver.click_widget_id("import_paste.source_sheet.ds-a")?;
        driver.click_widget_id("modal.ok")?;

        assert!(
            driver.state().last_intent_error.is_some(),
            "committing import wizard with Graph active and no target sheet must fail instead of falling back to source Sheet A"
        );
        assert!(driver.state().app.grid.import_wizard.open, "wizard should remain open when commit is rejected");
        let ds_a = driver.state().app.document().data_sheet(&SheetId::new("ds-a")).ok_or("missing ds-a")?;
        assert!(ds_a.linked_regions.is_empty(), "Sheet A must never be self-linked");
        assert_eq!(cell_number(ds_a, 0, 0), Some(10.0));
    }

    // Scenario C: Explicit self-link (target == A, source == A) is rejected cleanly.
    {
        let mut driver = make_shell_driver(build_two_sheets_and_graph_doc()?, 1280.0, 800.0);
        driver.state_mut().intents.push_open_import_wizard(ImportOriginKind::InternalDataSheet, true, Some(SheetId::new("ds-a")));
        commit_pending(&mut driver);

        driver.click_widget_id("import_paste.source_sheet.ds-a")?;
        driver.click_widget_id("modal.ok")?;

        let err = driver.state().last_intent_error.as_deref().unwrap_or("");
        assert!(err.contains("onto itself"), "expected explicit self-link rejection message, got: {err:?}");
        assert!(driver.state().app.grid.import_wizard.open, "wizard must stay open after self-link rejection");
        let ds_a = driver.state().app.document().data_sheet(&SheetId::new("ds-a")).ok_or("missing ds-a")?;
        assert!(ds_a.linked_regions.is_empty(), "self-link rejection must leave Sheet A unlinked");
    }

    Ok(())
}

#[test]
fn b10_r15_gui_14_welcome_hub_open_pane_uses_curvebook_extension_wording() -> Result<(), String> {
    let doc = ProjectDocument::new("Welcome Hub Extension Regression").map_err(|e| e.to_string())?;
    let mut driver = make_shell_driver(doc, 1280.0, 800.0);
    driver.state_mut().app.egui_views.launcher_open = true;
    driver.state_mut().views.launcher_open = true;
    driver.run_settled();

    // Switch to Open Existing File pane via the rendered Welcome Hub rail button.
    driver.click_widget_id("welcome.action.open_existing")?;

    let subtitle = driver.frame_meta().find_by_widget_id("welcome.open.subtitle").ok_or("missing welcome.open.subtitle")?;
    assert!(subtitle.label.contains(".curvebook"), "Open subtitle must reference .curvebook, got: {:?}", subtitle.label);
    assert!(!subtitle.label.contains(".cbk"), "Open subtitle must not mention .cbk, got: {:?}", subtitle.label);

    let pipeline = driver.frame_meta().find_by_widget_id("welcome.preview.pipeline").ok_or("missing welcome.preview.pipeline")?;
    assert!(pipeline.label.contains(".curvebook"), "Open pipeline caption must reference .curvebook, got: {:?}", pipeline.label);
    assert!(!pipeline.label.contains(".cbk"), "Open pipeline caption must not mention .cbk, got: {:?}", pipeline.label);

    for entry in &driver.frame_meta().entries {
        assert!(!entry.label.contains(".cbk"), "widget `{}` leaked stale `.cbk` label: {:?}", entry.widget_id, entry.label);
        if let Some(val) = entry.value.as_ref().and_then(serde_json::Value::as_str) {
            assert!(!val.contains(".cbk"), "widget `{}` leaked stale `.cbk` value: {:?}", entry.widget_id, val);
        }
    }

    // Verify localized `.curvebook` wording across non-English locales as well.
    for locale in ["de", "ja"] {
        driver.state_mut().app.set_locale(locale);
        driver.run_settled();
        let sub = driver.frame_meta().find_by_widget_id("welcome.open.subtitle").ok_or("missing localized subtitle")?;
        assert!(sub.label.contains(".curvebook"), "locale `{locale}` subtitle must reference .curvebook, got: {:?}", sub.label);
        assert!(!sub.label.contains(".cbk"), "locale `{locale}` subtitle must not mention .cbk, got: {:?}", sub.label);
    }

    Ok(())
}

#[test]
fn b10_r15_gui_25_preferences_shortcut_cancel_restores_and_resets_draft_while_ok_applies() -> Result<(), String> {
    let mut driver = make_shell_driver(build_3x6_column_doc()?, 1280.0, 800.0);
    let original_combo = driver.state().app.shortcuts.active_combo("project.save").ok_or("missing project.save default shortcut")?.to_owned();

    // Open Preferences directly on Keyboard Shortcuts via intent pipeline.
    driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::KeyboardShortcuts));
    commit_pending(&mut driver);

    // Dead Graph Defaults tab must no longer exist.
    assert!(driver.frame_meta().find_by_widget_id("edit.tab.prefs.graph_defaults").is_none(), "unwired Graph Defaults tab must be removed");

    // Filter and select `project.save`, stage a new non-conflicting combo, and click Apply Shortcut.
    driver.click_widget_id("prefs.shortcuts.search")?;
    driver.type_text("Save")?;
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;

    assert!(driver.state().views.app_modals.preferences.shortcut_conflict_error.is_none());
    assert_eq!(
        driver.state().app.shortcuts.active_combo("project.save"),
        Some(original_combo.as_str()),
        "applying inside Preferences must only stage the edit, leaving live app.shortcuts untouched before OK"
    );

    // Click Cancel -> original shortcut remains active and draft is discarded cleanly.
    driver.click_widget_id("modal.cancel")?;
    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(
        driver.state().app.shortcuts.active_combo("project.save"),
        Some(original_combo.as_str()),
        "Cancel must keep original shortcut binding"
    );
    assert!(driver.state().app.egui_views.app_modals.preferences.staged_shortcuts.is_none());
    assert!(driver.state().app.egui_views.app_modals.preferences.shortcut_combo_draft.is_empty());

    // Reopen Preferences -> no stale draft survives from the cancelled edit.
    driver.state_mut().intents.push_open_preferences(Some(PreferencesCategory::KeyboardShortcuts));
    commit_pending(&mut driver);
    assert!(driver.state().views.app_modals.preferences.shortcut_combo_draft.is_empty());
    let reopened_staged =
        driver.state().views.app_modals.preferences.staged_shortcuts.as_ref().and_then(|s| s.active_combo("project.save")).map(str::to_owned);
    assert_eq!(reopened_staged.as_deref(), Some(original_combo.as_str()), "reopened draft must match live shortcuts");

    // Now rebind and click OK -> live app.shortcuts updates to `Ctrl+Alt+Y`.
    driver.click_widget_id("prefs.shortcuts.row.project.save")?;
    driver.state_mut().views.app_modals.preferences.shortcut_combo_draft = String::from("Ctrl+Alt+Y");
    driver.run_settled();
    driver.click_widget_id("prefs.shortcuts.apply")?;
    driver.click_widget_id("modal.ok")?;

    assert!(!driver.state().app.egui_views.app_modals.preferences_open);
    assert_eq!(
        driver.state().app.shortcuts.active_combo("project.save"),
        Some("Ctrl+Alt+Y"),
        "OK must commit the staged shortcut binding to app.shortcuts"
    );
    Ok(())
}
