//! Comprehensive Phase 6 / `SEC-15` regression suite (`FA-SEC15-01` through `FA-SEC15-15`).
//!
//! Covers behavioral invariants across active `grid_view`, `import_paste`, `dialogs_canvas`,
//! `egui_shell`, `i18n`, and `table.clearCells` (`region_id`):
//! - `FA-SEC15-01`: O(1) Shift+Arrow extension and uncapped `Range` clear beyond 4,096 cells
//! - `FA-SEC15-02`: Exact discontiguous `clear`/`copy`/`cut` without touching unselected gap cells
//! - `FA-SEC15-03`: Row-title alignment in `new_sheet` (`"Row"` header) & preservation on `existing_sheet`
//! - `FA-SEC15-04`: Non-live `external_csv` path commit & `TidyLongPivot` header/asterisk/row-title flags
//! - `FA-SEC15-06`: `ChangeGraphTypeState` committing `option_id`, `summary_measure`, and `error_bar_metric`
//! - `FA-SEC15-07`: Command palette confirming `CommandOrigin::Ui` via `ui.answer_dialog`
//! - `FA-SEC15-08`: Zoom clamping / non-finite rejection on `UiIntent::SetCanvasZoom`
//! - `FA-SEC15-10`: Region-scoped unlink in `table.clearCells` (`region_id`)
//! - `FA-SEC15-11`: Atomic `welcome_hub` sample instantiation
//! - `FA-SEC15-13`: Results `CopyResultTable` using `bound_tab_idx`, and `PowerWizardStage::from_index`
//! - `FA-SEC15-14`: 16-locale UI localization fixes & removal of internal milestone tags
//! - `FA-SEC15-15`: Removal of dead Wave 6 `cjk_rtl` / `shard_extra` / unused `eu.rs` scaffolding

#![forbid(unsafe_code)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ErrorBarMetric, ErrorCode,
    GraphicalDataObject, GroupedArrangement, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SummaryMeasure,
    TableArchetype, TableKind,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_platform::WorkdirGuard;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::dispatch_gui_action;
use curvebook_ui_egui::egui_shell::welcome_hub::build_sample_creation_intent;
use curvebook_ui_egui::egui_shell::{PendingUiIntents, UiIntent};
use curvebook_ui_egui::grid_view::handle_grid_key_down;
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, catalog_len, missing_translations, tr};
use curvebook_ui_egui::import_paste::commit_import_paste_dialog;
use curvebook_ui_egui::interaction::TextFieldEditState;
use curvebook_ui_egui::{ImportReshapeMode, KeyModifiers, UiApp};
use serde_json::json;

type TestRes = Result<(), String>;

use crate::common::temp_dir;

fn set_buffer(app: &mut UiApp, widget_id: &str, value: &str) {
    app.interaction.text_field_buffers.insert(String::from(widget_id), TextFieldEditState::new(value.to_owned(), false));
}

#[test]
fn fa_sec15_01_shift_arrow_and_delete_uncapped_past_4096_rows() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "big-col", "kind": "column", "cols": 1, "rows": 5000 }))
        .map_err(|e| e.to_string())?;
    app.execute_command(
        "table.setCells",
        json!({
            "sheet_id": "big-col",
            "edits": [
                { "row": 0, "col": 0, "value": 10.0 },
                { "row": 4096, "col": 0, "value": 20.0 },
                { "row": 4999, "col": 0, "value": 30.0 }
            ]
        }),
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "big-col" })).map_err(|e| e.to_string())?;

    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));

    let shift_jump = KeyModifiers { shift: true, meta: true, ..Default::default() };
    let nav_out = handle_grid_key_down(&mut app, "ArrowDown", shift_jump)?;
    assert!(nav_out.handled);
    let nav_out2 = handle_grid_key_down(&mut app, "ArrowDown", shift_jump)?;
    assert!(nav_out2.handled);

    assert!(
        app.grid.selected_coords.is_empty(),
        "Shift+Arrow range extension must keep selected_coords unmaterialized, got {} entries",
        app.grid.selected_coords.len()
    );
    let bounds = app.grid.tool.selection_bounds().ok_or_else(|| String::from("expected selection_bounds"))?;
    assert_eq!(bounds.0, GridCellCoord::new(0, 0, 0));
    assert_eq!(bounds.1, GridCellCoord::new(4999, 0, 0));

    let ds = app.document().data_sheet(&SheetId::from("big-col")).ok_or_else(|| String::from("missing big-col"))?;
    assert!(app.grid.is_cell_selected(ds, GridCellCoord::new(4096, 0, 0)));
    assert!(app.grid.is_cell_selected(ds, GridCellCoord::new(4999, 0, 0)));

    let del_out = handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    assert!(del_out.handled);
    assert_eq!(del_out.command_id.as_deref(), Some("table.clearCells"));

    let ds_after = app.document().data_sheet(&SheetId::from("big-col")).ok_or_else(|| String::from("missing big-col"))?;
    for row in [0usize, 4096, 4999] {
        let cell = ds_after.cell(CellCoord::new(row, 0, 0)).ok_or_else(|| format!("missing cell at row {row}"))?;
        assert!(cell.is_empty(), "expected row {row} to be cleared past 4096 cap, found {cell:?}");
    }
    Ok(())
}

#[test]
fn fa_sec15_02_empty_selection_and_discontiguous_clear_copy_cut() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "d2", "kind": "column", "cols": 2, "rows": 3 })).map_err(|e| e.to_string())?;
    app.execute_command("table.paste", json!({ "sheet_id": "d2", "start_row": 0, "start_col": 0, "text": "1\t2\n3\t4\n5\t6" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "d2" })).map_err(|e| e.to_string())?;

    // 1. Empty selection Delete must never synthesize (0, 0, 0).
    app.grid.tool.active = None;
    app.grid.tool.anchor = None;
    app.grid.selected_coords.clear();
    app.grid.selected_rows.clear();

    let empty_del = handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    assert_eq!(empty_del.action, "grid_core.clear_selection_empty");
    let ds = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.0));

    // 2. Discontiguous selection: (0, 0) and (2, 1) only; gap cells (1, 0), (1, 1), (0, 1), (2, 0) untouched.
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    {
        let ds_ref = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?.clone();
        app.grid.toggle_cmd_selection(&ds_ref, GridCellCoord::new(2, 1, 0));
    }

    let mut copy_intents = PendingUiIntents::new();
    copy_intents.push(UiIntent::GridCopySelection);
    let _ = copy_intents.commit_all_if_settled(&mut app, false);
    let copied = app.interaction.last_clipboard_text.as_deref().ok_or_else(|| String::from("missing copied text"))?;
    assert!(copied.contains('1') && copied.contains('6'));
    assert!(!copied.contains('3') && !copied.contains('4'), "discontiguous copy must omit unselected bounding-box gaps, got `{copied}`");

    let _ = handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    let ds_cleared = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert!(ds_cleared.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty));
    assert!(ds_cleared.cell(CellCoord::new(2, 1, 0)).is_some_and(Cell::is_empty));
    assert_eq!(ds_cleared.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(2.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(3.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(4.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(5.0));

    // Re-seed and verify GridCutSelection also preserves unselected gap cells.
    app.execute_command(
        "table.setCells",
        json!({ "sheet_id": "d2", "edits": [{ "row": 0, "col": 0, "value": 1.0 }, { "row": 2, "col": 1, "value": 6.0 }] }),
    )
    .map_err(|e| e.to_string())?;
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    {
        let ds_ref = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?.clone();
        app.grid.toggle_cmd_selection(&ds_ref, GridCellCoord::new(2, 1, 0));
    }
    let mut cut_intents = PendingUiIntents::new();
    cut_intents.push(UiIntent::GridCutSelection);
    let _ = cut_intents.commit_all_if_settled(&mut app, false);
    let ds_cut = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert!(ds_cut.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty));
    assert!(ds_cut.cell(CellCoord::new(2, 1, 0)).is_some_and(Cell::is_empty));
    assert_eq!(ds_cut.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(3.0));
    Ok(())
}

#[test]
fn fa_sec15_03_import_paste_row_titles_new_and_existing_sheet() -> TestRes {
    let mut app = UiApp::new();
    app.grid.import_reshape.tidy_delimiter = '\t';
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_reshape.title = String::from("Assay With Row Titles");
    app.grid.import_paste.source_origin = String::from("csv_text");
    app.grid.import_paste.has_header_row = true;
    app.grid.import_paste.has_row_titles = true;
    app.grid.import_paste.target_mode = String::from("new_sheet");
    set_buffer(&mut app, "import_paste.input.csv", "RowLabel\tDose\tResponse\nR1\t10\t100\nR2\t20\t200");

    let created = commit_import_paste_dialog(&mut app)?;
    let new_sid = SheetId::try_new(created["sheet_id"].as_str().ok_or_else(|| String::from("missing sheet_id"))?).map_err(|e| e.to_string())?;
    let new_ds = app.document().data_sheet(&new_sid).ok_or_else(|| String::from("missing imported sheet"))?;
    let TableArchetype::Column(col_new) = &new_ds.archetype else {
        return Err(String::from("expected Column archetype"));
    };
    assert_eq!(col_new.data_sets.len(), 2);
    assert_eq!(col_new.data_sets[0].title, "Dose", "header must not left-shift");
    assert_eq!(col_new.data_sets[1].title, "Response", "second header must not be dropped");
    assert_eq!(col_new.row_titles, vec![String::from("R1"), String::from("R2")]);

    // Existing sheet paste preserves row titles aligned to placed_start_row.
    app.execute_command("sheet.createData", json!({ "sheet_id": "dst-col", "kind": "column", "cols": 2, "rows": 4 })).map_err(|e| e.to_string())?;
    app.grid.import_paste.target_mode = String::from("existing_sheet");
    app.grid.import_paste.target_sheet_id = Some(String::from("dst-col"));
    app.grid.import_paste.anchor_row = 1;
    app.grid.import_paste.placement_mode = String::from("overwrite");

    let _ = commit_import_paste_dialog(&mut app)?;
    let dst_ds = app.document().data_sheet(&SheetId::from("dst-col")).ok_or_else(|| String::from("missing dst-col"))?;
    let TableArchetype::Column(col_dst) = &dst_ds.archetype else {
        return Err(String::from("expected Column archetype on dst-col"));
    };
    assert_eq!(dst_ds.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(10.0));
    assert_eq!(dst_ds.cell(CellCoord::new(2, 1, 0)).and_then(Cell::active_number), Some(200.0));
    assert_eq!(col_dst.row_titles.get(1).map(String::as_str), Some("R1"));
    assert_eq!(col_dst.row_titles.get(2).map(String::as_str), Some("R2"));
    Ok(())
}

#[test]
fn fa_sec15_04_external_csv_path_non_live_and_tidy_pivot_flags() -> TestRes {
    // 1. Non-live external_csv honors WID_PATH without inline WID_CSV.
    let dir = temp_dir("ext-csv")?;
    let csv_file = dir.path().join("sample.csv");
    fs::write(&csv_file, "Ctrl,Tx\n1.5,2.5\n3.5,4.5\n").map_err(|e| e.to_string())?;
    let guard = WorkdirGuard::new(dir.path()).map_err(|e| e.to_string())?;

    let mut app = UiApp::new();
    app.session_mut().set_path_policy(ExternalPathPolicy::Workdir(guard));
    app.grid.import_reshape.tidy_delimiter = ',';
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_paste.source_origin = String::from("external_csv");
    app.grid.import_paste.paste_as_live_link = false;
    app.grid.import_paste.target_mode = String::from("new_sheet");
    set_buffer(&mut app, "import_paste.input.path", &csv_file.to_string_lossy());

    let out = commit_import_paste_dialog(&mut app)?;
    let ext_sid = SheetId::try_new(out["sheet_id"].as_str().ok_or_else(|| String::from("missing sheet_id"))?).map_err(|e| e.to_string())?;
    let ext_ds = app.document().data_sheet(&ext_sid).ok_or_else(|| String::from("missing ext_ds"))?;
    assert!(ext_ds.linked_regions.is_empty(), "non-live external CSV import must not attach live links");
    assert_eq!(ext_ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.5));
    assert_eq!(ext_ds.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(4.5));

    // 2. TidyLongPivot with has_header_row = false must keep first row as data.
    let mut app2 = UiApp::new();
    app2.grid.import_reshape.mode = ImportReshapeMode::TidyLongPivot;
    app2.grid.import_reshape.tidy_delimiter = ',';
    app2.grid.import_reshape.target_kind = TableKind::Column;
    app2.grid.import_reshape.group_col = String::from("A");
    app2.grid.import_reshape.value_col = String::from("B");
    app2.grid.import_reshape.tidy_csv = String::from("Ctrl,10\nTx,20");
    app2.grid.import_paste.source_origin = String::from("reshape");
    app2.grid.import_paste.has_header_row = false;
    set_buffer(&mut app2, "import_paste.input.csv", "Ctrl,10\nTx,20");
    set_buffer(&mut app2, "import_paste.input.group_col", "A");
    set_buffer(&mut app2, "import_paste.input.value_col", "B");

    let tidy_hdrless = commit_import_paste_dialog(&mut app2)?;
    let hdrless_sid =
        SheetId::try_new(tidy_hdrless["sheet_id"].as_str().ok_or_else(|| String::from("missing sheet_id"))?).map_err(|e| e.to_string())?;
    let hdrless_ds = app2.document().data_sheet(&hdrless_sid).ok_or_else(|| String::from("missing hdrless_ds"))?;
    let TableArchetype::Column(col_hdrless) = &hdrless_ds.archetype else {
        return Err(String::from("expected Column archetype"));
    };
    assert_eq!(col_hdrless.data_sets.len(), 2, "headerless tidy pivot must keep both Ctrl and Tx groups");
    assert_eq!(hdrless_ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(10.0));
    assert_eq!(hdrless_ds.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(20.0));

    // 3. TidyLongPivot with parse_asterisk_exclusions = false must preserve `*` literals un-excluded.
    let mut app3 = UiApp::new();
    app3.grid.import_reshape.mode = ImportReshapeMode::TidyLongPivot;
    app3.grid.import_reshape.tidy_delimiter = ',';
    app3.grid.import_reshape.target_kind = TableKind::Column;
    app3.grid.import_reshape.group_col = String::from("Group");
    app3.grid.import_reshape.value_col = String::from("Value");
    app3.grid.import_reshape.tidy_csv = String::from("Group,Value\nCtrl,12.5*\nTx,20");
    app3.grid.import_paste.source_origin = String::from("reshape");
    app3.grid.import_paste.parse_asterisk_exclusions = false;
    set_buffer(&mut app3, "import_paste.input.csv", "Group,Value\nCtrl,12.5*\nTx,20");
    set_buffer(&mut app3, "import_paste.input.group_col", "Group");
    set_buffer(&mut app3, "import_paste.input.value_col", "Value");

    let tidy_star = commit_import_paste_dialog(&mut app3)?;
    let star_sid = SheetId::try_new(tidy_star["sheet_id"].as_str().ok_or_else(|| String::from("missing sheet_id"))?).map_err(|e| e.to_string())?;
    let star_ds = app3.document().data_sheet(&star_sid).ok_or_else(|| String::from("missing star_ds"))?;
    let star_cell = star_ds.cell(CellCoord::new(0, 0, 0)).ok_or_else(|| String::from("missing cell (0,0)"))?;
    assert!(!star_cell.excluded, "parse_asterisk_exclusions=false must not exclude `12.5*`");
    assert_eq!(star_cell.value, CellValue::Text(String::from("12.5*")));
    Ok(())
}

#[test]
fn fa_sec15_06_graph_type_apply_preserves_option_summary_and_error_bars() -> TestRes {
    let mut app = UiApp::new();
    let created = app
        .execute_command(
            "sheet.createData",
            json!({ "sheet_id": "grp-1", "kind": "grouped", "cols": 2, "rows": 3, "replicates": 3, "auto_graph": true }),
        )
        .map_err(|e| e.to_string())?;
    let g_sid = SheetId::try_new(
        created.payload["auto_graph_sheet_id"]
            .as_str()
            .or_else(|| app.document().graph_sheets.first().map(|g| g.id.as_str()))
            .ok_or_else(|| String::from("missing auto_graph_sheet_id"))?,
    )
    .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": g_sid.as_str() })).map_err(|e| e.to_string())?;
    {
        let doc = app.document().clone();
        let g = doc.graph_sheet(&g_sid).ok_or_else(|| String::from("missing graph"))?;
        app.canvas_dialogs.change_graph_type.open_for_graph(g, &doc);
    }

    app.canvas_dialogs.change_graph_type.selected_option_id = String::from("grouped.interleaved");
    app.canvas_dialogs.change_graph_type.summary_measure = SummaryMeasure::Median;
    app.canvas_dialogs.change_graph_type.error_bar_metric = ErrorBarMetric::ConfidenceInterval95;
    let doc = app.document().clone();
    for intent in app.canvas_dialogs.change_graph_type.commit_intents(&doc) {
        app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
    }

    let gs = app.document().graph_sheet(&g_sid).ok_or_else(|| String::from("missing graph"))?;
    assert_eq!(gs.spec.grouped_arrangement, GroupedArrangement::Interleaved);
    let first = gs.spec.series.first().ok_or_else(|| String::from("missing series[0]"))?;
    match &first.gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } => assert_eq!(*summary, SummaryMeasure::Median),
        other => return Err(format!("expected BarWithReplicates with Median summary, got {other:?}")),
    }
    assert_eq!(first.y_error_bars.as_ref().map(|eb| eb.metric), Some(ErrorBarMetric::ConfidenceInterval95));

    // Selecting grouped.stacked must pass option_id + stacked arrangement and drop error bars.
    app.canvas_dialogs.change_graph_type.selected_option_id = String::from("grouped.stacked");
    let doc2 = app.document().clone();
    for intent in app.canvas_dialogs.change_graph_type.commit_intents(&doc2) {
        app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
    }
    let gs2 = app.document().graph_sheet(&g_sid).ok_or_else(|| String::from("missing graph"))?;
    assert_eq!(gs2.spec.grouped_arrangement, GroupedArrangement::Stacked);
    assert!(gs2.spec.series.first().is_some_and(|s| s.y_error_bars.is_none()));
    Ok(())
}

#[test]
fn fa_sec15_07_command_palette_run_dispatches_ui_commands() -> TestRes {
    let mut app = UiApp::new();
    let initial_gallery = app.nav.gallery.open;
    app.canvas_dialogs.open_command_palette();
    app.canvas_dialogs.command_palette.query = String::from("ui.gallery.toggle");
    app.canvas_dialogs.command_palette.selected_index = 0;

    let _ = dispatch_gui_action(&mut app, "ui.answer_dialog", json!({ "action": "ok" })).map_err(|e| e.to_string())?;
    assert_eq!(app.nav.gallery.open, !initial_gallery, "UI command from command palette must execute cleanly");
    Ok(())
}

#[test]
fn fa_sec15_10_unlink_region_detaches_only_target_region() -> TestRes {
    let mut app = UiApp::new();
    for (sid, vals) in [("src-a", "[1.0, 2.0]"), ("src-b", "[3.0, 4.0]")] {
        app.execute_command("sheet.createData", json!({ "sheet_id": sid, "kind": "column", "cols": 1, "rows": 2 })).map_err(|e| e.to_string())?;
        let arr: Vec<f64> = serde_json::from_str(vals).map_err(|e| e.to_string())?;
        app.execute_command(
            "table.setCells",
            json!({ "sheet_id": sid, "edits": [{ "row": 0, "col": 0, "value": arr[0] }, { "row": 1, "col": 0, "value": arr[1] }] }),
        )
        .map_err(|e| e.to_string())?;
    }
    app.execute_command("sheet.createData", json!({ "sheet_id": "dst", "kind": "column", "cols": 2, "rows": 2 })).map_err(|e| e.to_string())?;
    app.execute_command("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-a", "anchor": { "row": 0, "col": 0 } }))
        .map_err(|e| e.to_string())?;
    app.execute_command("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-b", "anchor": { "row": 0, "col": 1 } }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "dst" })).map_err(|e| e.to_string())?;

    let region_ids: Vec<String> = app
        .document()
        .data_sheet(&SheetId::from("dst"))
        .ok_or_else(|| String::from("missing dst"))?
        .linked_regions
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(region_ids.len(), 2);

    // Engine rejects unknown region_id BEFORE clearing cells or detaching regions.
    let Err(err) =
        app.execute_command("table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": "no-such-link" }))
    else {
        return Err(String::from("unknown region_id must fail"));
    };
    assert_eq!(err.code, ErrorCode::NotFound);
    let dst_before = app.document().data_sheet(&SheetId::from("dst")).ok_or_else(|| String::from("missing dst"))?;
    assert_eq!(dst_before.linked_regions.len(), 2);
    assert_eq!(dst_before.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.0));

    // Unlink ONLY region_ids[0].
    app.execute_command("table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": region_ids[0] }))
        .map_err(|e| e.to_string())?;
    let dst_mid = app.document().data_sheet(&SheetId::from("dst")).ok_or_else(|| String::from("missing dst"))?;
    assert_eq!(dst_mid.linked_regions.len(), 1);
    assert_eq!(dst_mid.linked_regions[0].id, region_ids[1]);
    Ok(())
}

#[test]
fn fa_sec15_11_welcome_sample_instantiate_roundtrip() -> TestRes {
    let mut app = UiApp::new();
    let intent = build_sample_creation_intent(&app, "dose-response-4pl", None, false, false)?;
    let mut intents = PendingUiIntents::new();
    intents.push(intent);
    let batch = intents.commit_all_if_settled(&mut app, false);
    assert!(batch.results.first().is_some_and(Result::is_ok));
    assert_eq!(app.document().data_sheets.len(), 1);
    Ok(())
}

#[test]
fn fa_sec15_13_results_copy_bound_tab() -> TestRes {
    let mut doc = ProjectDocument::default();
    let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).map_err(|e| e.to_string())?;
    let ds = DataSheet::new("d1", "Groups", TableArchetype::Column(col_tbl)).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;

    let mut rspec =
        AnalysisSpec::new("ANOVA", SheetId::from("d1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    rspec.id = SheetId::from("res-tabs");
    let mut rs = ResultSheet::new(rspec).map_err(|e| e.to_string())?;
    rs.tabs = vec![
        ResultTab {
            id: String::from("tab-0"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Mean")],
            row_labels: vec![String::from("G1")],
            rows: vec![vec![ResultCell::Text(String::from("WRONG_TAB_0"))]],
            footnotes: Vec::new(),
        },
        ResultTab {
            id: String::from("tab-1"),
            title: String::from("Details"),
            kind: ResultTabKind::AnovaTable,
            column_headers: vec![String::from("Stat"), String::from("Val")],
            row_labels: vec![String::from("R0"), String::from("R1")],
            rows: vec![
                vec![ResultCell::Text(String::from("alpha")), ResultCell::Text(String::from("11"))],
                vec![ResultCell::Text(String::from("beta")), ResultCell::Text(String::from("42"))],
            ],
            footnotes: Vec::new(),
        },
    ];
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.result_view.active_tab_index = 0;

    let mut intents = PendingUiIntents::new();
    intents.push(UiIntent::SyncResultSelection { sheet_id: SheetId::from("res-tabs"), tab_index: 1, anchor: (1, 1), focus: (1, 1) });
    intents.push(UiIntent::CopyResultTable { sheet_id: SheetId::from("res-tabs"), force_full: false });
    let batch = intents.commit_all_if_settled(&mut app, false);
    assert!(batch.results.iter().all(Result::is_ok));

    let copied = app.interaction.last_clipboard_text.as_deref().ok_or_else(|| String::from("missing copied result text"))?;
    assert_eq!(copied, "42", "must copy selected cell from bound_tab_idx=1 rather than active_tab_index=0, got `{copied}`");
    Ok(())
}

#[test]
fn fa_sec15_14_i18n_localization_quality_and_no_qol7_tag() {
    for &loc in &LOCALES[1..] {
        let import_lbl = tr(loc, "menu.file.import_reshape");
        assert!(!import_lbl.contains("Tidy Long"), "{loc} menu.file.import_reshape retained English `Tidy Long`: `{import_lbl}`");
        let rem_lbl = tr(loc, "cmd.ui.result.applyRemediation");
        assert!(!rem_lbl.contains("QOL-7"), "{loc} cmd.ui.result.applyRemediation leaked `QOL-7`: `{rem_lbl}`");
    }
    for &loc in &["de", "es", "fr", "pt-BR", "it", "nl", "pl", "tr"] {
        let sens = tr(loc, "pw.section.sensitivity");
        let alpha = tr(loc, "pw.param.alpha");
        let power = tr(loc, "pw.param.target_power");
        let title = tr(loc, "pw.plot.title");
        assert!(!sens.contains("Alpha x Power"), "{loc} sensitivity retained English: `{sens}`");
        assert!(alpha.contains('α'), "{loc} pw.param.alpha should use `α`: `{alpha}`");
        assert!(power.contains("1 - β"), "{loc} pw.param.target_power should use `1 - β`: `{power}`");
        assert!(!title.contains("vs.") && !title.contains(" vs "), "{loc} pw.plot.title retained `vs`: `{title}`");
    }
    for &loc in &["de", "es", "fr", "pt-BR", "it", "pl"] {
        let note = tr(loc, "import_paste.notes.record_info");
        assert!(!note.contains("Info-Blatt") && !note.split_whitespace().any(|w| w == "Info"), "{loc} record_info retained English `Info`: `{note}`");
    }
}

#[test]
fn fa_sec15_15_i18n_dead_scaffolding_removed() {
    assert!(EXTRA_SHARD_CATALOGS.is_empty());
    assert_eq!(catalog_len(), 796);
    assert!(missing_translations().is_empty());
    assert!(all_keys().all(|k| !k.starts_with("w6.cjk_rtl.")));
    for used_w6 in [
        "w6.view.btn.create",
        "w6.view.status.ready",
        "w6.view.shell.filter_template",
        "w6.view.modal.result_viewer",
        "w6.view.results.status_pending",
    ] {
        assert!(!tr("de", used_w6).is_empty(), "live key `{used_w6}` missing");
    }
}
