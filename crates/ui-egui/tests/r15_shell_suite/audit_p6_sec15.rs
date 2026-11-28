//! Comprehensive Phase 6 / `SEC-15` regression suite (`FA-SEC15-01`..`FA-SEC15-15`;
//! `FA-SEC15-11` atomic sample undo is covered in `audit_p6_atomic_undo`).
#![forbid(unsafe_code)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ErrorBarMetric, ErrorCode,
    GraphicalDataObject, GroupedArrangement, ProjectDocument, ResultCell, ResultSheet, ResultTabKind, SheetId, SummaryMeasure, TableArchetype,
    TableKind,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_platform::WorkdirGuard;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::egui_shell::{PendingUiIntents, UiIntent};
use curvebook_ui_egui::grid_view::handle_grid_key_down;
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, all_keys, catalog_len, missing_translations, tr};
use curvebook_ui_egui::import_paste::commit_import_paste_dialog;
use curvebook_ui_egui::interaction::TextFieldEditState;
use curvebook_ui_egui::{ImportReshapeMode, KeyModifiers, UiApp};
use serde_json::json;

use crate::common::{TestRes, cell_number, temp_dir};
use crate::helpers::{exec, gui, make_result_tab};

fn set_buffer(app: &mut UiApp, wid: &str, value: &str) {
    app.interaction.text_field_buffers.insert(wid.to_owned(), TextFieldEditState::new(value.to_owned(), false));
}

fn commit_single(app: &mut UiApp, intent: UiIntent) {
    let mut intents = PendingUiIntents::new();
    intents.push(intent);
    let _ = intents.commit_all_if_settled(app, false);
}

#[test]
fn fa_sec15_01_shift_arrow_and_delete_uncapped_past_4096_rows() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "big-col", "kind": "column", "cols": 1, "rows": 5000 }))?;
    exec(
        &mut app,
        "table.setCells",
        json!({ "sheet_id": "big-col", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 4096, "col": 0, "value": 20.0 }, { "row": 4999, "col": 0, "value": 30.0 }] }),
    )?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "big-col" }))?;
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));

    let shift_jump = KeyModifiers { shift: true, meta: true, ..Default::default() };
    assert!(handle_grid_key_down(&mut app, "ArrowDown", shift_jump)?.handled);
    assert!(handle_grid_key_down(&mut app, "ArrowDown", shift_jump)?.handled);
    assert!(app.grid.selected_coords.is_empty());
    assert_eq!(app.grid.tool.selection_bounds(), Some((GridCellCoord::new(0, 0, 0), GridCellCoord::new(4999, 0, 0))));

    let ds = app.document().data_sheet(&SheetId::from("big-col")).ok_or("missing big-col")?;
    assert!(app.grid.is_cell_selected(ds, GridCellCoord::new(4096, 0, 0)) && app.grid.is_cell_selected(ds, GridCellCoord::new(4999, 0, 0)));

    let del = handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    assert!(del.handled && del.command_id.as_deref() == Some("table.clearCells"));
    let ds_after = app.document().data_sheet(&SheetId::from("big-col")).ok_or("missing big-col")?;
    for row in [0usize, 4096, 4999] {
        assert!(ds_after.cell(CellCoord::new(row, 0, 0)).is_some_and(Cell::is_empty), "row {row} not cleared");
    }
    Ok(())
}

#[test]
fn fa_sec15_02_empty_selection_and_discontiguous_clear_copy_cut() -> TestRes {
    let mut app = UiApp::new();
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "d2", "kind": "column", "cols": 2, "rows": 3 }))?;
    exec(&mut app, "table.paste", json!({ "sheet_id": "d2", "start_row": 0, "start_col": 0, "text": "1\t2\n3\t4\n5\t6" }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": "d2" }))?;

    app.grid.tool.active = None;
    app.grid.tool.anchor = None;
    app.grid.selected_coords.clear();
    app.grid.selected_rows.clear();
    assert_eq!(handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?.action, "grid_core.clear_selection_empty");
    assert_eq!(cell_number(app.document().data_sheet(&SheetId::from("d2")).ok_or("missing d2")?, 0, 0), Some(1.0));

    let select_discontig = |app: &mut UiApp| {
        app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
        let ds_ref = app.document().data_sheet(&SheetId::from("d2")).unwrap().clone();
        app.grid.toggle_cmd_selection(&ds_ref, GridCellCoord::new(2, 1, 0));
    };
    select_discontig(&mut app);
    commit_single(&mut app, UiIntent::GridCopySelection);
    let copied = app.interaction.last_clipboard_text.as_deref().ok_or("missing copied text")?;
    assert!(copied.contains('1') && copied.contains('6') && !copied.contains('3') && !copied.contains('4'), "got `{copied}`");

    handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    let ds_c = app.document().data_sheet(&SheetId::from("d2")).ok_or("missing d2")?;
    assert!(ds_c.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty) && ds_c.cell(CellCoord::new(2, 1, 0)).is_some_and(Cell::is_empty));
    for (r, c, v) in [(0, 1, 2.0), (1, 0, 3.0), (1, 1, 4.0), (2, 0, 5.0)] {
        assert_eq!(cell_number(ds_c, r, c), Some(v));
    }

    exec(
        &mut app,
        "table.setCells",
        json!({ "sheet_id": "d2", "edits": [{ "row": 0, "col": 0, "value": 1.0 }, { "row": 2, "col": 1, "value": 6.0 }] }),
    )?;
    select_discontig(&mut app);
    commit_single(&mut app, UiIntent::GridCutSelection);
    let ds_cut = app.document().data_sheet(&SheetId::from("d2")).ok_or("missing d2")?;
    assert!(ds_cut.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty) && ds_cut.cell(CellCoord::new(2, 1, 0)).is_some_and(Cell::is_empty));
    assert_eq!(cell_number(ds_cut, 1, 0), Some(3.0));
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

    let new_sid =
        SheetId::try_new(commit_import_paste_dialog(&mut app)?["sheet_id"].as_str().ok_or("missing sheet_id")?).map_err(|e| e.to_string())?;
    let TableArchetype::Column(col_new) = &app.document().data_sheet(&new_sid).ok_or("missing imported")?.archetype else {
        return Err(String::from("expected Column"));
    };
    assert_eq!((col_new.data_sets.len(), col_new.data_sets[0].title.as_str(), col_new.data_sets[1].title.as_str()), (2, "Dose", "Response"));
    assert_eq!(col_new.row_titles, vec!["R1", "R2"]);

    exec(&mut app, "sheet.createData", json!({ "sheet_id": "dst-col", "kind": "column", "cols": 2, "rows": 4 }))?;
    app.grid.import_paste.target_mode = String::from("existing_sheet");
    app.grid.import_paste.target_sheet_id = Some(String::from("dst-col"));
    app.grid.import_paste.anchor_row = 1;
    app.grid.import_paste.placement_mode = String::from("overwrite");
    commit_import_paste_dialog(&mut app)?;

    let dst_ds = app.document().data_sheet(&SheetId::from("dst-col")).ok_or("missing dst-col")?;
    let TableArchetype::Column(col_dst) = &dst_ds.archetype else {
        return Err(String::from("expected Column on dst-col"));
    };
    assert_eq!((cell_number(dst_ds, 1, 0), cell_number(dst_ds, 2, 1)), (Some(10.0), Some(200.0)));
    assert_eq!((col_dst.row_titles.get(1).map(String::as_str), col_dst.row_titles.get(2).map(String::as_str)), (Some("R1"), Some("R2")));
    Ok(())
}

fn run_tidy_pivot(csv: &str, group_col: &str, value_col: &str, has_header: bool, parse_asterisk: bool) -> Result<(UiApp, SheetId), String> {
    let mut app = UiApp::new();
    app.grid.import_reshape.mode = ImportReshapeMode::TidyLongPivot;
    app.grid.import_reshape.tidy_delimiter = ',';
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_reshape.group_col = group_col.to_owned();
    app.grid.import_reshape.value_col = value_col.to_owned();
    app.grid.import_reshape.tidy_csv = csv.to_owned();
    app.grid.import_paste.source_origin = String::from("reshape");
    app.grid.import_paste.has_header_row = has_header;
    app.grid.import_paste.parse_asterisk_exclusions = parse_asterisk;
    set_buffer(&mut app, "import_paste.input.csv", csv);
    set_buffer(&mut app, "import_paste.input.group_col", group_col);
    set_buffer(&mut app, "import_paste.input.value_col", value_col);
    let out = commit_import_paste_dialog(&mut app)?;
    let sid = SheetId::try_new(out["sheet_id"].as_str().ok_or("missing sheet_id")?).map_err(|e| e.to_string())?;
    Ok((app, sid))
}

#[test]
fn fa_sec15_04_external_csv_path_non_live_and_tidy_pivot_flags() -> TestRes {
    let dir = temp_dir("ext-csv")?;
    let csv_file = dir.path().join("sample.csv");
    fs::write(&csv_file, "Ctrl,Tx\n1.5,2.5\n3.5,4.5\n").map_err(|e| e.to_string())?;
    let mut app = UiApp::new();
    app.session_mut().set_path_policy(ExternalPathPolicy::Workdir(WorkdirGuard::new(dir.path()).map_err(|e| e.to_string())?));
    app.grid.import_reshape.tidy_delimiter = ',';
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.grid.import_paste.source_origin = String::from("external_csv");
    app.grid.import_paste.paste_as_live_link = false;
    app.grid.import_paste.target_mode = String::from("new_sheet");
    set_buffer(&mut app, "import_paste.input.path", &csv_file.to_string_lossy());
    let ext_sid =
        SheetId::try_new(commit_import_paste_dialog(&mut app)?["sheet_id"].as_str().ok_or("missing sheet_id")?).map_err(|e| e.to_string())?;
    let ext_ds = app.document().data_sheet(&ext_sid).ok_or("missing ext_ds")?;
    assert!(ext_ds.linked_regions.is_empty());
    assert_eq!((cell_number(ext_ds, 0, 0), cell_number(ext_ds, 1, 1)), (Some(1.5), Some(4.5)));

    let (app2, hdr_sid) = run_tidy_pivot("Ctrl,10\nTx,20", "A", "B", false, true)?;
    let hdr_ds = app2.document().data_sheet(&hdr_sid).ok_or("missing hdrless_ds")?;
    let TableArchetype::Column(col_hdr) = &hdr_ds.archetype else {
        return Err(String::from("expected Column"));
    };
    assert_eq!((col_hdr.data_sets.len(), cell_number(hdr_ds, 0, 0), cell_number(hdr_ds, 0, 1)), (2, Some(10.0), Some(20.0)));

    let (app3, star_sid) = run_tidy_pivot("Group,Value\nCtrl,12.5*\nTx,20", "Group", "Value", true, false)?;
    let star_cell = app3.document().data_sheet(&star_sid).ok_or("missing star_ds")?.cell(CellCoord::new(0, 0, 0)).ok_or("missing (0,0)")?;
    assert!(!star_cell.excluded && star_cell.value == CellValue::Text(String::from("12.5*")));
    Ok(())
}

#[test]
fn fa_sec15_06_graph_type_apply_preserves_option_summary_and_error_bars() -> TestRes {
    let mut app = UiApp::new();
    let created = exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "grp-1", "kind": "grouped", "cols": 2, "rows": 3, "replicates": 3, "auto_graph": true }),
    )?;
    let g_sid = SheetId::try_new(
        created.payload["auto_graph_sheet_id"]
            .as_str()
            .or_else(|| app.document().graph_sheets.first().map(|g| g.id.as_str()))
            .ok_or("missing graph")?,
    )
    .map_err(|e| e.to_string())?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": g_sid.as_str() }))?;
    let doc = app.document().clone();
    app.canvas_dialogs.change_graph_type.open_for_graph(doc.graph_sheet(&g_sid).ok_or("missing graph")?, &doc);

    app.canvas_dialogs.change_graph_type.selected_option_id = String::from("grouped.interleaved");
    app.canvas_dialogs.change_graph_type.summary_measure = SummaryMeasure::Median;
    app.canvas_dialogs.change_graph_type.error_bar_metric = ErrorBarMetric::ConfidenceInterval95;
    for intent in app.canvas_dialogs.change_graph_type.commit_intents(&doc) {
        exec(&mut app, &intent.command_id, intent.params)?;
    }
    let gs = app.document().graph_sheet(&g_sid).ok_or("missing graph")?;
    assert_eq!(gs.spec.grouped_arrangement, GroupedArrangement::Interleaved);
    let first = gs.spec.series.first().ok_or("missing series[0]")?;
    assert!(matches!(&first.gdo, GraphicalDataObject::BarWithReplicates { summary: SummaryMeasure::Median, .. }));
    assert_eq!(first.y_error_bars.as_ref().map(|eb| eb.metric), Some(ErrorBarMetric::ConfidenceInterval95));

    app.canvas_dialogs.change_graph_type.selected_option_id = String::from("grouped.stacked");
    let doc2 = app.document().clone();
    for intent in app.canvas_dialogs.change_graph_type.commit_intents(&doc2) {
        exec(&mut app, &intent.command_id, intent.params)?;
    }
    let gs2 = app.document().graph_sheet(&g_sid).ok_or("missing graph")?;
    assert_eq!(gs2.spec.grouped_arrangement, GroupedArrangement::Stacked);
    assert!(gs2.spec.series.first().is_some_and(|s| s.y_error_bars.is_none()));
    Ok(())
}

#[test]
fn fa_sec15_07_command_palette_run_dispatches_ui_commands() -> TestRes {
    let mut app = UiApp::new();
    let initial = app.nav.gallery.open;
    app.canvas_dialogs.open_command_palette();
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "palette.query" }))?;
    gui(&mut app, "ui.type_text", json!({ "text": "> ui.gallery.toggle" }))?;
    gui(&mut app, "ui.press_key", json!({ "key": "Enter" }))?;
    assert_eq!(app.nav.gallery.open, !initial);
    Ok(())
}

#[test]
fn fa_sec15_10_unlink_region_detaches_only_target_region() -> TestRes {
    let mut app = UiApp::new();
    for (sid, v0, v1) in [("src-a", 1.0, 2.0), ("src-b", 3.0, 4.0)] {
        exec(&mut app, "sheet.createData", json!({ "sheet_id": sid, "kind": "column", "cols": 1, "rows": 2 }))?;
        exec(
            &mut app,
            "table.setCells",
            json!({ "sheet_id": sid, "edits": [{ "row": 0, "col": 0, "value": v0 }, { "row": 1, "col": 0, "value": v1 }] }),
        )?;
    }
    exec(&mut app, "sheet.createData", json!({ "sheet_id": "dst", "kind": "column", "cols": 2, "rows": 2 }))?;
    for (col, src) in [(0, "src-a"), (1, "src-b")] {
        exec(&mut app, "table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": src, "anchor": { "row": 0, "col": col } }))?;
    }
    exec(&mut app, "sheet.select", json!({ "sheet_id": "dst" }))?;

    let region_ids: Vec<String> =
        app.document().data_sheet(&SheetId::from("dst")).ok_or("missing dst")?.linked_regions.iter().map(|r| r.id.clone()).collect();
    assert_eq!(region_ids.len(), 2);

    let err = app
        .execute_command("table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": "no-such-link" }))
        .err()
        .ok_or("unknown region_id must fail")?;
    assert_eq!(err.code, ErrorCode::NotFound);
    let dst_before = app.document().data_sheet(&SheetId::from("dst")).ok_or("missing dst")?;
    assert_eq!((dst_before.linked_regions.len(), cell_number(dst_before, 0, 0)), (2, Some(1.0)));

    exec(&mut app, "table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": region_ids[0] }))?;
    let dst_mid = app.document().data_sheet(&SheetId::from("dst")).ok_or("missing dst")?;
    assert_eq!((dst_mid.linked_regions.len(), dst_mid.linked_regions[0].id.as_str()), (1, region_ids[1].as_str()));
    Ok(())
}

#[test]
fn fa_sec15_13_results_copy_bound_tab() -> TestRes {
    let mut doc = ProjectDocument::default();
    let ds = DataSheet::new(
        "d1",
        "Groups",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).map_err(|e| e.to_string())?),
    )
    .map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let mut rspec =
        AnalysisSpec::new("ANOVA", SheetId::from("d1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).map_err(|e| e.to_string())?;
    rspec.id = SheetId::from("res-tabs");
    let mut rs = ResultSheet::new(rspec).map_err(|e| e.to_string())?;
    rs.tabs = vec![
        make_result_tab("tab-0", "Summary", ResultTabKind::MainSummary, &["Mean"], &["G1"], vec![vec![ResultCell::Text("WRONG_TAB_0".into())]]),
        make_result_tab(
            "tab-1",
            "Details",
            ResultTabKind::AnovaTable,
            &["Stat", "Val"],
            &["R0", "R1"],
            vec![
                vec![ResultCell::Text("alpha".into()), ResultCell::Text("11".into())],
                vec![ResultCell::Text("beta".into()), ResultCell::Text("42".into())],
            ],
        ),
    ];
    doc.add_result_sheet(rs).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.canvas_dialogs.result_view.active_tab_index = 0;
    let mut intents = PendingUiIntents::new();
    intents.push(UiIntent::SyncResultSelection { sheet_id: SheetId::from("res-tabs"), tab_index: 1, anchor: (1, 1), focus: (1, 1) });
    intents.push(UiIntent::CopyResultTable { sheet_id: SheetId::from("res-tabs"), force_full: false });
    assert!(intents.commit_all_if_settled(&mut app, false).results.iter().all(Result::is_ok));
    assert_eq!(app.interaction.last_clipboard_text.as_deref(), Some("42"));
    Ok(())
}

#[test]
fn fa_sec15_14_i18n_localization_quality_and_no_qol7_tag() {
    for &loc in &LOCALES[1..] {
        assert!(!tr(loc, "menu.file.import_reshape").contains("Tidy Long"), "{loc} retained Tidy Long");
        assert!(!tr(loc, "cmd.ui.result.applyRemediation").contains("QOL-7"), "{loc} leaked QOL-7");
    }
    for &loc in &["de", "es", "fr", "pt-BR", "it", "nl", "pl", "tr"] {
        assert!(!tr(loc, "pw.section.sensitivity").contains("Alpha x Power"), "{loc} sensitivity");
        assert!(tr(loc, "pw.param.alpha").contains('α'), "{loc} alpha");
        assert!(tr(loc, "pw.param.target_power").contains("1 - β"), "{loc} target_power");
        let title = tr(loc, "pw.plot.title");
        assert!(!title.contains("vs.") && !title.contains(" vs "), "{loc} plot.title: `{title}`");
    }
    for &loc in &["de", "es", "fr", "pt-BR", "it", "pl"] {
        let note = tr(loc, "import_paste.notes.record_info");
        assert!(!note.contains("Info-Blatt") && !note.split_whitespace().any(|w| w == "Info"), "{loc} record_info: `{note}`");
    }
}

#[test]
fn fa_sec15_15_i18n_dead_scaffolding_removed() {
    assert!(EXTRA_SHARD_CATALOGS.is_empty() && catalog_len() == 1120 && missing_translations().is_empty());
    assert!(all_keys().all(|k| !k.starts_with("w6.cjk_rtl.")));
    for key in [
        "w6.view.btn.create",
        "w6.view.status.ready",
        "w6.view.shell.filter_template",
        "w6.view.modal.result_viewer",
        "w6.view.results.status_pending",
    ] {
        assert!(!tr("de", key).is_empty(), "live key `{key}` missing");
    }
}
