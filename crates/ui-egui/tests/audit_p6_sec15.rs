//! Comprehensive Phase 6 / `SEC-15` regression suite (`FA-SEC15-01` through `FA-SEC15-15`).
//!
//! Covers all 15 audit findings across `views/*`, `i18n/*`, and `table.clearCells` (`region_id`):
//! - `FA-SEC15-01`: O(1) Shift+Arrow extension and uncapped `Range` clear beyond 4,096 cells
//! - `FA-SEC15-02`: Explicit no-op/error on empty selection, exact discontiguous `clear`/`copy`/`cut`, no `doc.clone()`
//! - `FA-SEC15-03`: Row-title alignment in `new_sheet` (`"Row"` header) & preservation on `existing_sheet`
//! - `FA-SEC15-04`: Non-live `external_csv` path commit & `TidyLongPivot` header/asterisk/row-title flags
//! - `FA-SEC15-05`: MV Inspector `mv.metric:GeometricMean` vs rejected `"Count"`
//! - `FA-SEC15-06`: `graph_type:apply` committing `option_id`, `summary_measure`, and `error_bar_metric`
//! - `FA-SEC15-07`: Command palette `"palette:run"` routing `CommandOrigin::Ui` via `dispatch_ui_command`
//! - `FA-SEC15-08`: Zoom & pan rejecting non-finite `NaN`/`inf` payloads
//! - `FA-SEC15-09`: Localized comma-decimals & explicit errors on blocked profile CI / Robust covariance in `fit_params`
//! - `FA-SEC15-10`: Region-scoped unlink in `table.clearCells` (`region_id`), `grid_core`, and `toolbar_menus`
//! - `FA-SEC15-11`: Atomic `welcome` sample instantiation
//! - `FA-SEC15-12`: Inspector tick-interval preservation, `hi <= lo` error, `sig_only:toggle` `command_id`, layout overflow rows, Uniform min clamp
//! - `FA-SEC15-13`: Results `copy_selection` using `bound_tab_idx`, and `power_wizard` rejecting `"stage:0"`
//! - `FA-SEC15-14`: 16-locale UI localization fixes & removal of internal milestone tags
//! - `FA-SEC15-15`: Removal of dead Wave 6 `cjk_rtl` / `shard_extra` / unused `eu.rs` scaffolding

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AxisScale, AxisSegment, AxisSpec, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet,
    DescriptiveSpec, ErrorBarMetric, ErrorCode, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, LayoutItem, LayoutSheet, LayoutSpec,
    MultivariateGraphEncoding, MultivariatePlotMode, NonlinearFitSpec, ParameterScope, ProjectDocument, RectPt, ResultCell, ResultSheet, ResultTab,
    ResultTabKind, SheetId, SummaryMeasure, TableArchetype, TableKind,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_platform::WorkdirGuard;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_r9::SimDistKind;
use curvebook_ui_egui::i18n::{EXTRA_SHARD_CATALOGS, LOCALES, LocalePreset, all_keys, catalog_len, missing_translations, tr};
use curvebook_ui_egui::interaction::TextFieldEditState;
use curvebook_ui_egui::views::grid_core::handle_grid_key_down;
use curvebook_ui_egui::views::import_paste::commit_import_paste_dialog;
use curvebook_ui_egui::views::{ViewShardStates, dispatch_custom_view_action};
use curvebook_ui_egui::{ImportReshapeMode, KeyModifiers, UiApp};
use serde_json::json;

type TestRes = Result<(), String>;

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-audit-p6-sec15-{label}-{}-{seq}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path).map_err(|err| format!("create temp dir `{}`: {err}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn dispatch_shard(app: &mut UiApp, shards: &mut ViewShardStates, domain: &str, payload: &str) -> Result<curvebook_ui_egui::UiEventOutcome, String> {
    dispatch_custom_view_action(app, shards, domain, payload)
}

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

    let mut shards = ViewShardStates::default();
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));

    let shift_jump = KeyModifiers { shift: true, meta: true, ..Default::default() };
    let nav_out = handle_grid_key_down(&mut app, &mut shards.grid_core, "ArrowDown", shift_jump)?;
    assert!(nav_out.handled);
    let nav_out2 = handle_grid_key_down(&mut app, &mut shards.grid_core, "ArrowDown", shift_jump)?;
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

    let del_out = handle_grid_key_down(&mut app, &mut shards.grid_core, "Delete", KeyModifiers::default())?;
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

    let mut shards = ViewShardStates::default();

    // 1. Empty selection must never synthesize (0, 0, 0).
    app.grid.tool.active = None;
    app.grid.tool.anchor = None;
    app.grid.selected_coords.clear();
    app.grid.selected_rows.clear();

    let _ = dispatch_shard(&mut app, &mut shards, "toolbar_menus", "ctx.clear_cells")?;
    let ds = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.0));

    let excl_err = dispatch_shard(&mut app, &mut shards, "toolbar_menus", "ctx.exclude");
    assert!(excl_err.is_err(), "ctx.exclude with empty selection must return Err");
    let ds = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert!(!ds.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded));

    // 2. Discontiguous selection: (0, 0) and (2, 1) only; gap cells (1, 0), (1, 1), (0, 1), (2, 0) untouched.
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    {
        let ds_ref = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?.clone();
        app.grid.toggle_cmd_selection(&ds_ref, GridCellCoord::new(2, 1, 0));
    }

    dispatch_shard(&mut app, &mut shards, "toolbar_menus", "clipboard.copy")?;
    let copied = app.interaction.last_clipboard_text.as_deref().ok_or_else(|| String::from("missing copied text"))?;
    assert!(copied.contains('1') && copied.contains('6'));
    assert!(!copied.contains('3') && !copied.contains('4'), "discontiguous copy must omit unselected bounding-box gaps, got `{copied}`");

    dispatch_shard(&mut app, &mut shards, "toolbar_menus", "ctx.clear_cells")?;
    let ds_cleared = app.document().data_sheet(&SheetId::from("d2")).ok_or_else(|| String::from("missing d2"))?;
    assert!(ds_cleared.cell(CellCoord::new(0, 0, 0)).is_some_and(Cell::is_empty));
    assert!(ds_cleared.cell(CellCoord::new(2, 1, 0)).is_some_and(Cell::is_empty));
    assert_eq!(ds_cleared.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(2.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(3.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(4.0));
    assert_eq!(ds_cleared.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(5.0));

    // Re-seed and verify clipboard.cut also preserves unselected gap cells.
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
    dispatch_shard(&mut app, &mut shards, "toolbar_menus", "clipboard.cut")?;
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
    app.view_shards.import_paste.source_origin = String::from("csv_text");
    app.view_shards.import_paste.has_header_row = true;
    app.view_shards.import_paste.has_row_titles = true;
    app.view_shards.import_paste.target_mode = String::from("new_sheet");
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
    app.view_shards.import_paste.target_mode = String::from("existing_sheet");
    app.view_shards.import_paste.target_sheet_id = Some(String::from("dst-col"));
    app.view_shards.import_paste.anchor_row = 1;
    app.view_shards.import_paste.placement_mode = String::from("overwrite");

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
    let dir = TempDir::new("ext-csv")?;
    let csv_file = dir.path.join("sample.csv");
    fs::write(&csv_file, "Ctrl,Tx\n1.5,2.5\n3.5,4.5\n").map_err(|e| e.to_string())?;
    let guard = WorkdirGuard::new(&dir.path).map_err(|e| e.to_string())?;

    let mut app = UiApp::new();
    app.session_mut().set_path_policy(ExternalPathPolicy::Workdir(guard));
    app.grid.import_reshape.tidy_delimiter = ',';
    app.grid.import_reshape.target_kind = TableKind::Column;
    app.view_shards.import_paste.source_origin = String::from("external_csv");
    app.view_shards.import_paste.paste_as_live_link = false;
    app.view_shards.import_paste.target_mode = String::from("new_sheet");
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
    app2.view_shards.import_paste.source_origin = String::from("reshape");
    app2.view_shards.import_paste.has_header_row = false;
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
    app3.view_shards.import_paste.source_origin = String::from("reshape");
    app3.view_shards.import_paste.parse_asterisk_exclusions = false;
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
fn fa_sec15_05_mv_inspector_metric_geometric_mean_vs_count() -> TestRes {
    let mut doc = ProjectDocument::default();
    let mv_tbl = curvebook_doc::MultipleVariablesTable::new(2, 3).map_err(|e| e.to_string())?;
    let mv_ds = DataSheet::new("mv-1", "MV", TableArchetype::MultipleVariables(mv_tbl)).map_err(|e| e.to_string())?;
    let mut spec = GraphSpec::default_for_data_sheet(&mv_ds);
    if let Some(enc) = spec.multivariate.as_mut() {
        enc.plot_mode = MultivariatePlotMode::BubbleScatter;
        enc.x_column = 0;
        enc.y_column = 1;
    }
    let mut gs = GraphSheet::new_uninitialized_for_data("mv-graph", &mv_ds).map_err(|e| e.to_string())?;
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;
    gs.spec = spec;
    gs.initialized = true;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("mv-graph")).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let mut shards = ViewShardStates::default();

    let count_out = dispatch_shard(&mut app, &mut shards, "inspector", "mv.metric:Count")?;
    assert!(!count_out.handled, "mv.metric:Count must not silently map to GeometricMean");

    let gm_out = dispatch_shard(&mut app, &mut shards, "inspector", "mv.metric:GeometricMean")?;
    assert!(gm_out.handled);
    let gs_after = app.document().graph_sheet(&SheetId::from("mv-graph")).ok_or_else(|| String::from("missing mv-graph"))?;
    let Some(MultivariateGraphEncoding { plot_mode: MultivariatePlotMode::CategoricalSummary { gdo, .. }, .. }) = &gs_after.spec.multivariate else {
        return Err(String::from("expected CategoricalSummary plot_mode after GeometricMean"));
    };
    match gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => {
            assert_eq!(*summary, SummaryMeasure::GeometricMean);
        }
        other => return Err(format!("unexpected GDO after GeometricMean: {other:?}")),
    }
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

    let mut shards = ViewShardStates::default();
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:select:grouped.interleaved")?;
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:summary:median")?;
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:errorbar:ci95")?;
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:apply")?;

    let gs = app.document().graph_sheet(&g_sid).ok_or_else(|| String::from("missing graph"))?;
    assert_eq!(gs.spec.grouped_arrangement, GroupedArrangement::Interleaved);
    let first = gs.spec.series.first().ok_or_else(|| String::from("missing series[0]"))?;
    match &first.gdo {
        GraphicalDataObject::BarWithReplicates { summary, .. } => assert_eq!(*summary, SummaryMeasure::Median),
        other => return Err(format!("expected BarWithReplicates with Median summary, got {other:?}")),
    }
    assert_eq!(first.y_error_bars.as_ref().map(|eb| eb.metric), Some(ErrorBarMetric::ConfidenceInterval95));

    // Selecting grouped.stacked must pass option_id + stacked arrangement and drop error bars.
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:select:grouped.stacked")?;
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "graph_type:apply")?;
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

    let mut shards = ViewShardStates::default();
    let out = dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "palette:run")?;
    assert!(out.handled);
    assert_eq!(out.command_id.as_deref(), Some("ui.gallery.toggle"));
    assert_eq!(app.nav.gallery.open, !initial_gallery, "UI command from command palette must execute cleanly");
    Ok(())
}

#[test]
fn fa_sec15_08_zoom_and_pan_reject_non_finite() -> TestRes {
    let mut app = UiApp::new();
    app.canvas_dialogs.canvas.zoom = 1.25;
    app.canvas_dialogs.pan_zoom.zoom = 1.25;
    app.canvas_dialogs.canvas.pan_pt = curvebook_chart::PointPt::new(12.0, -8.0);
    app.canvas_dialogs.pan_zoom.pan_pt = [12.0, -8.0];

    let mut shards = ViewShardStates::default();
    for bad in ["zoom:set:main:NaN", "zoom:set:main:inf", "zoom:set:main:-5", "pan:nudge:main:NaN:10", "pan:nudge:main:10:infinity"] {
        let res = dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", bad);
        assert!(res.is_err(), "expected error for `{bad}`");
    }
    assert!((app.canvas_dialogs.canvas.zoom - 1.25).abs() < 1e-12);
    assert!((app.canvas_dialogs.pan_zoom.zoom - 1.25).abs() < 1e-12);
    assert!(app.canvas_dialogs.canvas.pan_pt.x.is_finite() && app.canvas_dialogs.canvas.pan_pt.y.is_finite());
    assert!((app.canvas_dialogs.canvas.pan_pt.x - 12.0).abs() < 1e-12);
    assert!((app.canvas_dialogs.canvas.pan_pt.y + 8.0).abs() < 1e-12);
    Ok(())
}

#[test]
fn fa_sec15_09_fit_params_localized_decimals_and_explicit_errors() -> TestRes {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({ "sheet_id": "xy-nl", "kind": "xy", "cols": 1, "rows": 5 })).map_err(|e| e.to_string())?;
    app.locale = String::from("de");
    app.locale_preset = LocalePreset::for_locale("de");

    let nl_spec =
        AnalysisSpec::new("DoseFit", SheetId::from("xy-nl"), AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("log_agonist_vs_response_4pl")))
            .map_err(|e| e.to_string())?;
    app.canvas_dialogs.edit_params.draft = Some(nl_spec);

    let mut shards = ViewShardStates::default();

    // 1. Comma-decimal inputs in Constant / Bounds / Initial / CI level / X range.
    dispatch_shard(&mut app, &mut shards, "fit_params", "set_scope:constant")?;
    set_buffer(&mut app, "fit.con.const.input", "1,5");
    dispatch_shard(&mut app, &mut shards, "fit_params", "apply_param_rule")?;

    let check_const = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::NonlinearRegression(nl)) => nl.parameter_rules.first().map(|r| r.scope.clone()),
        _ => None,
    };
    assert_eq!(check_const, Some(ParameterScope::Constant(1.5)));

    dispatch_shard(&mut app, &mut shards, "fit_params", "set_scope:local")?;
    set_buffer(&mut app, "fit.con.lo.input", "0,25");
    set_buffer(&mut app, "fit.con.hi.input", "9,75");
    dispatch_shard(&mut app, &mut shards, "fit_params", "apply_param_rule")?;

    set_buffer(&mut app, "fit.init.val.input", "2,5");
    dispatch_shard(&mut app, &mut shards, "fit_params", "apply_init_input")?;

    set_buffer(&mut app, "fit.ci.lvl.input", "95,5");
    dispatch_shard(&mut app, &mut shards, "fit_params", "apply_ci_level")?;

    set_buffer(&mut app, "fit.wr.xmin.input", "-1,5");
    set_buffer(&mut app, "fit.wr.xmax.input", "3,5");
    dispatch_shard(&mut app, &mut shards, "fit_params", "apply_range")?;

    let draft = app.canvas_dialogs.edit_params.draft.as_ref().ok_or_else(|| String::from("missing draft"))?;
    assert!((draft.confidence_level.percent() - 95.5).abs() < 1e-9);
    let AnalysisKind::NonlinearRegression(nl) = &draft.kind else {
        return Err(String::from("expected NonlinearRegression"));
    };
    let rule0 = nl.parameter_rules.first().ok_or_else(|| String::from("missing rule0"))?;
    assert_eq!(rule0.lower_bound, Some(0.25));
    assert_eq!(rule0.upper_bound, Some(9.75));
    assert_eq!(rule0.initial_override, Some(2.5));
    assert_eq!(nl.x_range_filter, Some((Some(-1.5), Some(3.5))));

    // 2. Malformed numeric text surfaces Err cleanly.
    set_buffer(&mut app, "fit.init.val.input", "not-a-number");
    assert!(dispatch_shard(&mut app, &mut shards, "fit_params", "apply_init_input").is_err());

    // 3. Method guards: ROUT blocks profile CI but allows toggle_emit_cov; RobustCauchy blocks both.
    dispatch_shard(&mut app, &mut shards, "fit_params", "method:rout")?;
    assert!(dispatch_shard(&mut app, &mut shards, "fit_params", "ci_method:profile").is_err());
    let cov_rout = dispatch_shard(&mut app, &mut shards, "fit_params", "toggle_emit_cov")?;
    assert!(cov_rout.handled, "ROUT must permit toggle_emit_cov");

    dispatch_shard(&mut app, &mut shards, "fit_params", "method:robust")?;
    assert!(dispatch_shard(&mut app, &mut shards, "fit_params", "ci_method:profile").is_err());
    assert!(dispatch_shard(&mut app, &mut shards, "fit_params", "toggle_emit_cov").is_err());
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

    let mut shards = ViewShardStates::default();
    // grid_core link:unlink:{region_id} detaches ONLY region_ids[0].
    dispatch_shard(&mut app, &mut shards, "grid_core", &format!("link:unlink:{}", region_ids[0]))?;
    let dst_mid = app.document().data_sheet(&SheetId::from("dst")).ok_or_else(|| String::from("missing dst"))?;
    assert_eq!(dst_mid.linked_regions.len(), 1);
    assert_eq!(dst_mid.linked_regions[0].id, region_ids[1]);

    // Re-add a second link at col 0, focus col 1, and unlink via toolbar_menus ctx.unlink_region.
    app.execute_command("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-a", "anchor": { "row": 0, "col": 0 } }))
        .map_err(|e| e.to_string())?;
    app.grid.tool.active = Some(GridCellCoord::new(0, 1, 0));
    dispatch_shard(&mut app, &mut shards, "toolbar_menus", "ctx.unlink_region")?;
    let dst_after = app.document().data_sheet(&SheetId::from("dst")).ok_or_else(|| String::from("missing dst"))?;
    assert_eq!(dst_after.linked_regions.len(), 1);
    assert_ne!(dst_after.linked_regions[0].id, region_ids[1], "ctx.unlink_region must detach only the targeted col-1 region");
    Ok(())
}

#[test]
fn fa_sec15_11_welcome_sample_instantiate_roundtrip() -> TestRes {
    let mut app = UiApp::new();
    let mut shards = ViewShardStates::default();
    dispatch_shard(&mut app, &mut shards, "welcome", "fixture:dose-response-4pl")?;
    let out = dispatch_shard(&mut app, &mut shards, "welcome", "action:create")?;
    assert!(out.handled);
    assert_eq!(app.document().data_sheets.len(), 1);
    Ok(())
}

#[test]
fn fa_sec15_12_inspector_and_sim_dist_precision_fixes() -> TestRes {
    let mut doc = ProjectDocument::default();
    let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).map_err(|e| e.to_string())?;
    let ds = DataSheet::new("d1", "Groups", TableArchetype::Column(col_tbl)).map_err(|e| e.to_string())?;
    let mut gspec = GraphSpec::default_for_data_sheet(&ds);
    gspec.x_axis = AxisSpec {
        segments: vec![AxisSegment {
            min: Some(0.0),
            max: Some(100.0),
            scale: AxisScale::Linear,
            major_tick_interval: Some(10.0),
            minor_ticks_per_interval: 2,
            length_share: 1.0,
        }],
        ..AxisSpec::linear("X")
    };
    let mut gs = GraphSheet::new_uninitialized_for_data("g1", &ds).map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    gs.spec = gspec;
    gs.initialized = true;
    doc.add_graph_sheet(gs).map_err(|e| e.to_string())?;

    // Layout with 5 panels (> 2x2) must expand rows instead of overlapping at row 0.
    let mut lspec = LayoutSpec { grid_template: Some((2, 2)), ..LayoutSpec::default() };
    for idx in 0..5 {
        lspec.items.push(LayoutItem::LiveGraph {
            id: format!("p-{idx}"),
            graph_sheet_id: SheetId::from("g1"),
            bounds_pt: RectPt::new(10.0, 10.0, 50.0, 50.0),
            scale_percent: 100.0,
            badge_override: None,
        });
    }
    let ls = LayoutSheet::new("lay-1", "Layout", lspec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(ls).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("g1")).map_err(|e| e.to_string())?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let mut shards = ViewShardStates::default();

    // 1. Axis breaks preserve major_tick_interval.
    dispatch_shard(&mut app, &mut shards, "inspector", "axes.target:x")?;
    dispatch_shard(&mut app, &mut shards, "inspector", "axes.breaks:2")?;
    let g_after_breaks = app.document().graph_sheet(&SheetId::from("g1")).ok_or_else(|| String::from("missing g1"))?;
    assert_eq!(g_after_breaks.spec.x_axis.segments.len(), 2);
    assert!(g_after_breaks.spec.x_axis.segments.iter().all(|seg| seg.major_tick_interval == Some(10.0)));

    // 2. Inverted axis range returns Err.
    set_buffer(&mut app, "inspector.axes.min.input", "50");
    set_buffer(&mut app, "inspector.axes.max.input", "10");
    assert!(dispatch_shard(&mut app, &mut shards, "inspector", "axes.apply_range").is_err());

    // 3. pw.sig_only:toggle omits command_id when no comparison brackets exist.
    let sig_out = dispatch_shard(&mut app, &mut shards, "inspector", "pw.sig_only:toggle")?;
    assert!(sig_out.handled);
    assert_eq!(sig_out.command_id, None, "must not claim graph.addComparisonBrackets ran when nothing was executed");

    // 4. Layout 2x2 grid with 5 panels expands to 3x2 without duplicate bounds_pt.
    app.execute_command("sheet.select", json!({ "sheet_id": "lay-1" })).map_err(|e| e.to_string())?;
    dispatch_shard(&mut app, &mut shards, "inspector", "layout.grid:2x2")?;
    let lay_after = app.document().layout_sheet(&SheetId::from("lay-1")).ok_or_else(|| String::from("missing lay-1"))?;
    assert_eq!(lay_after.spec.grid_template, Some((3, 2)));
    let coords_set: BTreeSet<(i64, i64)> = lay_after
        .spec
        .items
        .iter()
        .filter_map(|it| match it {
            LayoutItem::LiveGraph { bounds_pt, .. } => Some(((bounds_pt.x * 100.0).round() as i64, (bounds_pt.y * 100.0).round() as i64)),
            _ => None,
        })
        .collect();
    assert_eq!(coords_set.len(), 5, "all 5 layout panels must occupy distinct non-overlapping origins");

    // 5. SimDistKind::Uniform min step clamps to <= max - 0.5.
    app.canvas_dialogs.simulate_distributions.columns[0].distribution = SimDistKind::Uniform;
    app.canvas_dialogs.simulate_distributions.columns[0].min = 0.0;
    app.canvas_dialogs.simulate_distributions.columns[0].max = 1.0;
    dispatch_shard(&mut app, &mut shards, "dialogs_graph_misc", "sim_dist:param:p1_inc")?;
    let col0 = &app.canvas_dialogs.simulate_distributions.columns[0];
    assert!(col0.min <= col0.max - 0.5, "Uniform min ({}) must stay <= max - 0.5 ({})", col0.min, col0.max - 0.5);
    Ok(())
}

#[test]
fn fa_sec15_13_results_copy_bound_tab_and_power_stage_zero() -> TestRes {
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

    let mut shards = ViewShardStates::default();
    dispatch_shard(&mut app, &mut shards, "results", "select_cell:1:1:1:res-tabs")?;
    dispatch_shard(&mut app, &mut shards, "results", "copy_selection:res-tabs")?;

    let copied = app.interaction.last_clipboard_text.as_deref().ok_or_else(|| String::from("missing copied result text"))?;
    assert_eq!(copied, "42", "must copy selected cell from bound_tab_idx=1 rather than active_tab_index=0, got `{copied}`");

    let stage_zero_err = dispatch_shard(&mut app, &mut shards, "power_wizard", "stage:0");
    assert!(stage_zero_err.is_err(), "power_wizard stage:0 must be rejected instead of coalescing to stage 1");
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
    assert_eq!(catalog_len(), 720);
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
