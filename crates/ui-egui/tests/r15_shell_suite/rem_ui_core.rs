//! Comprehensive regression tests for `fix-ui-core` remediation
//! (`AUD-07-01`..`27`, `AUD-08-01`..`18`, and assigned `AUD-13` UI-core halves).
#![forbid(unsafe_code)]

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, ColumnDesignHint, ColumnTable, CurveSamplePoint, DataSheet, DescriptiveSpec,
    EquationDisplayMode, ErrorCode, FittedCurveOverlay, GlmSpec, GraphSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec,
    LinearRegressionSpec, LinkSource, LinkedDataRegion, MultipleVariablesTable, NestedTable, NonlinearFitSpec, ProjectDocument, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SimpleLogisticSpec, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, XyTable,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_render::{RasterImage, RenderTargetSpec};
use curvebook_stats::PowerTestFamily;
use curvebook_tools::GridCellCoord;
use curvebook_tools::canvas::PhysicalUnit;
use curvebook_ui_egui::control_io::host_routing::{
    MAX_READ_ONLY_NESTING_DEPTH, command_replaces_project, commit_active_modal_dialog, contextual_command_params, has_non_null_path_param,
    is_command_read_only, is_read_only_ui_command,
};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::dialogs_r9::{InsertModelEquationDialogState, RandomSampleRowsDialogState};
use curvebook_ui_egui::egui_shell::canvas_view::{compute_ruler_ticks, resolve_live_equation_tex};
use curvebook_ui_egui::egui_shell::center_stage::ProgressModalState;
use curvebook_ui_egui::egui_shell::inspect_bridge::{
    HybridCoexistenceArbiter, HybridModalSurface, finalize_screenshot_payload_with_limit, parse_answer_dialog_params, parse_pointer_gesture_params,
    parse_switch_tab_params, prepare_answer_dialog_patch, prepare_grid_type_text_target, render_egui_shell_frame, resolve_power_curve_hover_target,
};
use curvebook_ui_egui::egui_shell::intents::{PendingUiIntents, StatParamsIntent, UiIntent};
use curvebook_ui_egui::egui_shell::leading_dock::{LeadingDockEguiState, collect_visible_sheet_ids};
use curvebook_ui_egui::egui_shell::rasterizer::crop_raster_rgba;
use curvebook_ui_egui::egui_shell::top_bar_fit::{RightWidths, SearchVisibility, fit_right_group};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, FrameInspectNodeMeta};
use curvebook_ui_egui::grid_view::{
    ImportOriginKind, ImportPasteWizardState, handle_grid_key_down, handle_grid_text_input, linked_region_for_cell_subcol,
};
use curvebook_ui_egui::i18n::LOCALES;
use curvebook_ui_egui::import_paste::commit_import_paste_dialog;
use curvebook_ui_egui::interaction::{KeyModifiers, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder, dispatch_ui_event};
use curvebook_ui_egui::menus::curated_menu_bar;
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use curvebook_ui_egui::power_wizard::{PowerEffectInputMode, PowerWizardState};
use curvebook_ui_egui::search_engine::{SearchRecents, UnifiedSearchAction, UnifiedSearchItem, UnifiedSearchKind, query_unified};
use curvebook_ui_egui::workspace_nav::NavigatorNestingMode;
use curvebook_ui_egui::{ControlResponse, ImportWarningItem, UiApp, ZLayer, dispatch_gui_action, handle_control_line};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn control_call(app: &mut UiApp, req: &Value) -> Result<ControlResponse, Box<dyn Error>> {
    let line = serde_json::to_string(req)?;
    let resp_line = handle_control_line(app, &line);
    let resp: ControlResponse = serde_json::from_str(&resp_line)?;
    Ok(resp)
}

fn temp_test_dir(tag: &str) -> Result<PathBuf, Box<dyn Error>> {
    let raw_base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target").join("rem-ui-core-test-tmp");
    std::fs::create_dir_all(&raw_base)?;
    let base = std::fs::canonicalize(&raw_base)?;
    let pid = std::process::id();
    let dir = base.join(format!("{tag}-{pid}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn build_minimal_xlsx_bytes() -> Result<Vec<u8>, Box<dyn Error>> {
    let rels_root = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="SheetA" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#;
    let wb_rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;
    let sheet1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Val</t></is></c></row>
    <row r="2"><c r="A2"><v>1.0</v></c></row>
    <row r="3"><c r="A3"><v>2.0</v></c></row>
  </sheetData>
</worksheet>"#;
    let mut zw = curvebook_format::ZipWriter::new();
    zw.add_file("_rels/.rels", rels_root.as_bytes().to_vec())?;
    zw.add_file("xl/workbook.xml", workbook.as_bytes().to_vec())?;
    zw.add_file("xl/_rels/workbook.xml.rels", wb_rels.as_bytes().to_vec())?;
    zw.add_file("xl/worksheets/sheet1.xml", sheet1.as_bytes().to_vec())?;
    Ok(zw.finish()?)
}

fn minimal_column_app(rows: usize, cols: usize) -> Result<UiApp, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("RemUiCore")?;
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows)?;
    for c in 0..cols {
        for r in 0..rows {
            table.data_sets[c].cells[r][0] = Cell::number((r + 1) as f64 + (c as f64) * 10.0);
        }
    }
    let ds = DataSheet::new(SheetId::new("data-1"), "Data 1", TableArchetype::Column(table))?;
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(SheetId::new("data-1"));
    Ok(UiApp::from_session(Session::from_document(doc)))
}

fn minimal_xy_with_graph_app() -> Result<UiApp, Box<dyn Error>> {
    let mut doc = ProjectDocument::new("GraphProj")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 4)?;
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(((r + 1) * 2) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(((r + 1) * 3) as f64);
    }
    let ds = DataSheet::new(SheetId::new("data-xy"), "XY Data", TableArchetype::Xy(xy))?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &ds)?;
    gs.initialized = true;
    doc.data_sheets.push(ds);
    doc.graph_sheets.push(gs);
    doc.active_sheet_id = Some(SheetId::new("graph-1"));
    Ok(UiApp::from_session(Session::from_document(doc)))
}

// ============================================================================
// AUD-07-01 & AUD-13-02: ExternalPathPolicy enforcement in load_xlsx_from_path_str
// ============================================================================
#[test]
fn aud_07_01_and_13_02_xlsx_wizard_enforces_external_path_policy() -> TestResult {
    let root = temp_test_dir("aud07-01")?;
    let workdir = root.join("workdir");
    let outside = root.join("outside");
    std::fs::create_dir_all(&workdir)?;
    std::fs::create_dir_all(&outside)?;

    let xlsx_bytes = build_minimal_xlsx_bytes()?;
    let outside_xlsx = outside.join("secret.xlsx");
    let inside_xlsx = workdir.join("allowed.xlsx");
    std::fs::write(&outside_xlsx, &xlsx_bytes)?;
    std::fs::write(&inside_xlsx, &xlsx_bytes)?;

    // RejectExternal must block reading any path.
    let mut sess = Session::new();
    sess.set_path_policy(ExternalPathPolicy::RejectExternal);
    let mut app = UiApp::from_session(sess);
    app.grid.import_wizard.external_file_path = inside_xlsx.to_string_lossy().into_owned();
    let err = app.grid.import_wizard.load_xlsx_from_path_str();
    assert!(err.is_err(), "RejectExternal must block load_xlsx_from_path_str");

    // Workdir policy blocks outside path and permits inside path, emitting `path` (not bytes_base64).
    let mut sess2 = Session::new();
    sess2.set_path_policy(ExternalPathPolicy::workdir(&workdir)?);
    let mut app2 = UiApp::from_session(sess2);
    app2.grid.import_wizard.external_file_path = outside_xlsx.to_string_lossy().into_owned();
    assert!(app2.grid.import_wizard.load_xlsx_from_path_str().is_err(), "Workdir policy must block outside .xlsx");

    app2.grid.import_wizard.external_file_path = inside_xlsx.to_string_lossy().into_owned();
    let sheets_loaded = app2.grid.import_wizard.load_xlsx_from_path_str()?;
    assert_eq!(sheets_loaded, 1);
    let params = app2.grid.import_wizard.build_xlsx_import_params().ok_or("expected xlsx import params")?;
    assert!(params.get("path").is_some(), "path-loaded xlsx must emit `path` for engine policy validation");
    assert!(params.get("bytes_base64").is_none(), "path-loaded xlsx must not smuggle bytes via `bytes_base64`");

    // Direct load_xlsx_bytes still emits bytes_base64.
    let mut wiz_bytes = ImportPasteWizardState::default();
    let _ = wiz_bytes.load_xlsx_bytes(&xlsx_bytes)?;
    let bytes_params = wiz_bytes.build_xlsx_import_params().ok_or("expected bytes params")?;
    assert!(bytes_params.get("bytes_base64").is_some());

    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

// ============================================================================
// AUD-07-02: Internal DataSheet/ResultSheet extraction (>200 rows, all archetypes, active tab, full CI precision)
// ============================================================================
#[test]
fn aud_07_02_internal_sheet_extraction_no_200_cap_and_all_archetypes() -> TestResult {
    let mut doc = ProjectDocument::new("ExtractAll")?;

    // 1. Column table with 240 rows
    let mut big_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 240)?;
    for r in 0..240 {
        big_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    doc.data_sheets.push(DataSheet::new(SheetId::new("big-col"), "Big", TableArchetype::Column(big_col))?);

    // 2. Survival table
    let mut surv = SurvivalTable::new(2, 3)?;
    surv.time_title = String::from("Days");
    surv.cohorts[0].title = String::from("Ctrl");
    surv.cohorts[1].title = String::from("Drug");
    for r in 0..3 {
        surv.times[r] = Cell::number(((r + 1) * 10) as f64);
        surv.cohorts[0].cells[r][0] = Cell::number(1.0);
        surv.cohorts[1].cells[r][0] = Cell::number(0.0);
    }
    doc.data_sheets.push(DataSheet::new(SheetId::new("surv-1"), "Surv", TableArchetype::Survival(surv))?);

    // 3. Nested table
    let mut nst = NestedTable::new(2, 2, 2)?;
    nst.groups[0].cells[0][0] = Cell::number(11.0);
    nst.groups[0].cells[0][1] = Cell::number(12.0);
    nst.groups[1].cells[0][0] = Cell::number(21.0);
    nst.groups[1].cells[0][1] = Cell::number(22.0);
    doc.data_sheets.push(DataSheet::new(SheetId::new("nest-1"), "Nested", TableArchetype::Nested(nst))?);

    // 4. Multi-tab ResultSheet with high-precision CI on tab 1
    let mut rs = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("res-1"),
        SheetId::new("big-col"),
        AnalysisKind::DescriptiveStats(DescriptiveSpec::default()),
    )?)?;
    rs.status = ResultStatus::UpToDate;
    rs.tabs.push(ResultTab {
        id: String::from("tab-0"),
        title: String::from("Tab 0"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("First")],
        row_labels: vec![String::from("R0")],
        rows: vec![vec![ResultCell::Number { value: 1.0, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    rs.tabs.push(ResultTab {
        id: String::from("tab-1"),
        title: String::from("Tab 1"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("CI")],
        row_labels: vec![String::from("Precise")],
        rows: vec![vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(0.00012345), upper: CiBound::Finite(0.00098765) }]],
        footnotes: Vec::new(),
    });
    doc.result_sheets.push(rs);
    doc.active_sheet_id = Some(SheetId::new("big-col"));

    let mut app = UiApp::from_session(Session::from_document(doc));

    // Commit >200 rows from big-col into new sheet
    app.grid.import_paste.source_origin = String::from("internal_sheet");
    app.grid.import_paste.internal_source_sheet_id = Some(String::from("big-col"));
    app.grid.import_paste.target_mode = String::from("new_sheet");
    app.grid.import_reshape.target_kind = TableKind::Column;
    let payload = commit_import_paste_dialog(&mut app)?;
    let new_id = SheetId::new(payload.get("sheet_id").and_then(Value::as_str).ok_or("missing sheet_id")?);
    let imported = app.document().data_sheet(&new_id).ok_or("missing imported sheet")?;
    assert_eq!(imported.archetype.row_count(), 240, "must preserve all 240 rows without 200-row cap");

    // Commit Survival internal sheet -> preserves Elapsed Time + 2 cohorts = 3 columns
    app.grid.import_paste.internal_source_sheet_id = Some(String::from("surv-1"));
    app.grid.import_reshape.target_kind = TableKind::Survival;
    let surv_payload = commit_import_paste_dialog(&mut app)?;
    let surv_id = SheetId::new(surv_payload.get("sheet_id").and_then(Value::as_str).ok_or("missing surv sheet_id")?);
    let surv_ds = app.document().data_sheet(&surv_id).ok_or("missing surv sheet")?;
    let TableArchetype::Survival(ref imported_surv) = surv_ds.archetype else {
        return Err("expected Survival archetype (AUD-07-04)".into());
    };
    assert_eq!(imported_surv.times[0].active_number(), Some(10.0));
    assert_eq!(imported_surv.cohorts.len(), 2);

    // Commit Nested internal sheet -> preserves all 4 biological subcolumns
    app.grid.import_paste.internal_source_sheet_id = Some(String::from("nest-1"));
    app.grid.import_reshape.target_kind = TableKind::Column;
    let nest_payload = commit_import_paste_dialog(&mut app)?;
    let nest_id = SheetId::new(nest_payload.get("sheet_id").and_then(Value::as_str).ok_or("missing nest sheet_id")?);
    let nest_ds = app.document().data_sheet(&nest_id).ok_or("missing nest sheet")?;
    assert_eq!(nest_ds.archetype.column_count(), 4, "must extract all 4 nested biological-unit columns");

    // Commit active tab 1 of ResultSheet -> preserves full CI precision
    app.canvas_dialogs.result_view.result_sheet_id = Some(SheetId::new("res-1"));
    app.canvas_dialogs.result_view.active_tab_index = 1;
    app.grid.import_paste.internal_source_sheet_id = Some(String::from("res-1"));
    app.grid.import_reshape.target_kind = TableKind::Column;
    let rs_payload = commit_import_paste_dialog(&mut app)?;
    let rs_id = SheetId::new(rs_payload.get("sheet_id").and_then(Value::as_str).ok_or("missing rs sheet_id")?);
    let rs_ds = app.document().data_sheet(&rs_id).ok_or("missing rs sheet")?;
    let cell_text = match rs_ds.cell(CellCoord::new(0, 0, 0)).map(|c| &c.value) {
        Some(curvebook_doc::CellValue::Text(s)) => s.clone(),
        other => return Err(format!("expected CI text cell, got {other:?}").into()),
    };
    assert!(cell_text.contains("0.00012345") && cell_text.contains("0.00098765"), "CI must keep full precision, got `{cell_text}`");
    Ok(())
}

// ============================================================================
// AUD-07-03 & AUD-07-04: Asterisk restoration across subcolumns + atomic undo + exhaustive wire names
// ============================================================================
#[test]
fn aud_07_03_and_04_asterisk_subcols_atomic_undo_and_archetypes() -> TestResult {
    let mut doc = ProjectDocument::new("GroupedPaste")?;
    let grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    doc.data_sheets.push(DataSheet::new(SheetId::new("grp-1"), "Grouped", TableArchetype::Grouped(grp))?);
    doc.active_sheet_id = Some(SheetId::new("grp-1"));
    let mut app = UiApp::from_session(Session::from_document(doc));

    let undo_before = app.session().undo_depth();
    app.grid.import_paste.source_origin = String::from("clipboard");
    app.grid.import_paste.raw_source_text = String::from("A1\tA2\tB1\tB2\n1\t*litA2\t3\t*litB2");
    app.grid.import_paste.has_header_row = true;
    app.grid.import_paste.parse_asterisk_exclusions = false;
    app.grid.import_paste.target_mode = String::from("existing_sheet");
    app.grid.import_paste.target_sheet_id = Some(String::from("grp-1"));
    app.grid.import_reshape.tidy_delimiter = '\t';
    let _ = commit_import_paste_dialog(&mut app)?;

    let ds = app.document().data_sheet(&SheetId::new("grp-1")).ok_or("missing grp-1")?;
    let c01 = ds.cell(CellCoord::new(0, 0, 1)).ok_or("missing (0,0,1)")?;
    let c11 = ds.cell(CellCoord::new(0, 1, 1)).ok_or("missing (0,1,1)")?;
    assert!(!c01.excluded, "unexcluded asterisk in col 0 subcol 1 must not be excluded");
    assert_eq!(c01.value, curvebook_doc::CellValue::Text(String::from("*litA2")));
    assert!(!c11.excluded, "unexcluded asterisk in col 1 subcol 1 must not be excluded");
    assert_eq!(c11.value, curvebook_doc::CellValue::Text(String::from("*litB2")));
    assert_eq!(app.session().undo_depth(), undo_before + 1, "paste + asterisk restore must collapse into a single undo entry");

    // AUD-07-04: PartsOfWhole and Nested wire names
    for kind in [TableKind::PartsOfWhole, TableKind::Nested] {
        app.grid.import_paste.target_mode = String::from("new_sheet");
        app.grid.import_paste.parse_asterisk_exclusions = true;
        app.grid.import_paste.raw_source_text = String::from("G1\tG2\n10\t20\n30\t40");
        app.grid.import_reshape.target_kind = kind;
        let out = commit_import_paste_dialog(&mut app)?;
        let sid = SheetId::new(out.get("sheet_id").and_then(Value::as_str).ok_or("missing sheet_id")?);
        let created = app.document().data_sheet(&sid).ok_or("missing created sheet")?;
        assert_eq!(created.kind(), kind, "imported sheet must preserve requested TableKind {kind:?}");
    }
    Ok(())
}

// ============================================================================
// AUD-07-05: Cluster append duplicate row labels + single atomic undo
// ============================================================================
#[test]
fn aud_07_05_cluster_append_duplicate_labels_and_atomic_undo() -> TestResult {
    let mut doc = ProjectDocument::new("ClusterDup")?;
    let mut mv = MultipleVariablesTable::new(1, 3)?;
    mv.row_titles = vec![String::from("Ctrl"), String::from("Ctrl"), String::from("Treated")];
    mv.variables[0].cells = vec![Cell::number(1.0), Cell::number(2.0), Cell::number(10.0)];
    doc.data_sheets.push(DataSheet::new(SheetId::new("mv-1"), "MV", TableArchetype::MultipleVariables(mv))?);

    let mut rs = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("res-cl"),
        SheetId::new("mv-1"),
        AnalysisKind::Multivariate(curvebook_doc::MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: curvebook_doc::ClusterDistance::Euclidean,
            linkage: curvebook_doc::ClusterLinkage::Ward,
            k_sweep_max: None,
        }),
    )?)?;
    rs.tabs.push(ResultTab {
        id: String::from("kmeans-assignments"),
        title: String::from("K-Means Assignments"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Cluster")],
        row_labels: vec![String::from("Ctrl"), String::from("Ctrl"), String::from("Treated")],
        rows: vec![
            vec![ResultCell::Number { value: 1.0, prefix: None, units: None }],
            vec![ResultCell::Number { value: 2.0, prefix: None, units: None }],
            vec![ResultCell::Number { value: 2.0, prefix: None, units: None }],
        ],
        footnotes: Vec::new(),
    });
    doc.result_sheets.push(rs);
    doc.active_sheet_id = Some(SheetId::new("res-cl"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    let undo_before = app.session().undo_depth();
    let resp = control_call(
        &mut app,
        &json!({ "id": "cl-1", "method": "ui.result.appendClusterColumn", "params": { "sheet_id": "res-cl", "column_title": "Cluster" } }),
    )?;
    assert!(resp.ok, "appendClusterColumn failed: {:?}", resp.error);
    assert_eq!(resp.result.get("assigned_rows").and_then(Value::as_u64), Some(3));
    assert_eq!(app.session().undo_depth(), undo_before + 1, "appendClusterColumn must be a single undo step");

    let ds = app.document().data_sheet(&SheetId::new("mv-1")).ok_or("missing mv-1")?;
    let TableArchetype::MultipleVariables(ref mv_after) = ds.archetype else {
        return Err("expected MV".into());
    };
    let cl_col = mv_after.variables.last().ok_or("missing cluster col")?;
    assert_eq!(cl_col.cells[0].value, curvebook_doc::CellValue::Text(String::from("Cluster 1")));
    assert_eq!(cl_col.cells[1].value, curvebook_doc::CellValue::Text(String::from("Cluster 2")));
    assert_eq!(cl_col.cells[2].value, curvebook_doc::CellValue::Text(String::from("Cluster 2")));
    Ok(())
}

// ============================================================================
// AUD-07-06, AUD-07-07, AUD-07-08: Unstack/wrap bounds, skip_contiguous_blanks, exclude_predicate
// ============================================================================
#[test]
fn aud_07_06_07_08_import_paste_bounds_blanks_and_predicates() -> TestResult {
    let mut app = minimal_column_app(2, 2)?;
    app.grid.import_paste.source_origin = String::from("clipboard");
    app.grid.import_paste.target_mode = String::from("new_sheet");
    app.grid.import_reshape.tidy_delimiter = '\t';

    // AUD-07-06: row_wrap_width > MAX_DATA_SETS rejected cleanly
    app.grid.import_paste.raw_source_text = String::from("A\tB\n1\t2");
    app.grid.import_paste.row_wrap_width = curvebook_doc::MAX_DATA_SETS + 1;
    assert!(commit_import_paste_dialog(&mut app).is_err(), "oversized row_wrap_width must fail validation");
    app.grid.import_paste.row_wrap_width = 0;

    // AUD-07-06: unstack exceeding MAX_DATA_SETS rejected cleanly without OOM
    let mut huge_groups = String::from("Group\tVal\n");
    for i in 0..=(curvebook_doc::MAX_DATA_SETS + 2) {
        huge_groups.push_str(&format!("G{i}\t{i}\n"));
    }
    app.grid.import_paste.raw_source_text = huge_groups;
    app.grid.import_paste.unstack_enabled = true;
    app.grid.import_paste.unstack_group_col = 0;
    assert!(commit_import_paste_dialog(&mut app).is_err(), "unstack exceeding MAX_DATA_SETS must return Err");
    app.grid.import_paste.unstack_enabled = false;

    // AUD-07-07: skip_contiguous_blanks collapses double blanks to single blank (preserving 1 blank)
    app.grid.import_paste.raw_source_text = String::from("Val\n10\n\n\n20\n\n30");
    app.grid.import_paste.skip_contiguous_blanks = true;
    app.grid.import_paste.target_mode = String::from("existing_sheet");
    app.grid.import_paste.target_sheet_id = Some(String::from("data-1"));
    let _ = commit_import_paste_dialog(&mut app)?;
    let ds = app.document().data_sheet(&SheetId::new("data-1")).ok_or("missing data-1")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(10.0));
    assert!(ds.cell(CellCoord::new(1, 0, 0)).is_some_and(Cell::is_empty), "single separator blank must be preserved");
    assert_eq!(ds.cell(CellCoord::new(2, 0, 0)).and_then(Cell::active_number), Some(20.0));
    assert!(ds.cell(CellCoord::new(3, 0, 0)).is_some_and(Cell::is_empty));
    assert_eq!(ds.cell(CellCoord::new(4, 0, 0)).and_then(Cell::active_number), Some(30.0));
    app.grid.import_paste.skip_contiguous_blanks = false;

    // AUD-07-08: predicate resolves header names, rejects unknown names & malformed syntax
    app.grid.import_paste.target_mode = String::from("new_sheet");
    app.grid.import_paste.raw_source_text = String::from("Dose\tResp\n1\t100\n10\t200");
    app.grid.import_paste.exclude_predicate = String::from("Dose < 5");
    let out = commit_import_paste_dialog(&mut app)?;
    let sid = SheetId::new(out.get("sheet_id").and_then(Value::as_str).ok_or("missing sid")?);
    let ds_pred = app.document().data_sheet(&sid).ok_or("missing ds_pred")?;
    assert!(ds_pred.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded), "row 0 (Dose=1 < 5) must be excluded");
    assert!(ds_pred.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.excluded), "row 1 (Dose=10) must remain included");

    app.grid.import_paste.exclude_predicate = String::from("NonExistentCol > 0");
    assert!(commit_import_paste_dialog(&mut app).is_err(), "unknown column name in exclude_predicate must return Err");
    app.grid.import_paste.exclude_predicate = String::from("not a predicate");
    assert!(commit_import_paste_dialog(&mut app).is_err(), "malformed exclude_predicate must return Err");
    Ok(())
}

// ============================================================================
// AUD-07-09: FormatSelectedPoints cross-sheet isolation & brushed commit
// ============================================================================
#[test]
fn aud_07_09_format_selected_points_filters_brushed_by_source_sheet() -> TestResult {
    let mut doc = ProjectDocument::new("BrushScope")?;
    let mut xy1 = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    let mut xy2 = XyTable::new(SubcolumnMode::Single, 1, 3)?;
    for r in 0..3 {
        xy1.x_cells[r][0] = Cell::number(r as f64);
        xy1.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        xy2.x_cells[r][0] = Cell::number(r as f64);
        xy2.data_sets[0].cells[r][0] = Cell::number((r + 10) as f64);
    }
    let ds1 = DataSheet::new(SheetId::new("ds-a"), "A", TableArchetype::Xy(xy1))?;
    let ds2 = DataSheet::new(SheetId::new("ds-b"), "B", TableArchetype::Xy(xy2))?;
    let mut g1 = GraphSheet::new_uninitialized_for_data(SheetId::new("g-a"), &ds1)?;
    g1.initialized = true;
    doc.data_sheets.push(ds1);
    doc.data_sheets.push(ds2);
    doc.graph_sheets.push(g1);
    doc.active_sheet_id = Some(SheetId::new("g-a"));

    let mut app = UiApp::from_session(Session::from_document(doc));
    // Brush a point on ds-b (NOT graph g-a's source sheet ds-a)
    app.brush_from_grid_cell(&SheetId::new("ds-b"), GridCellCoord::new(2, 0, 0));
    let resp = control_call(&mut app, &json!({ "id": 1, "method": "ui.formatPoints.open", "params": { "sheet_id": "g-a" } }))?;
    assert!(resp.ok);
    assert_eq!(resp.result.get("selected_count").and_then(Value::as_u64), Some(0), "brushed point from ds-b must NOT leak into g-a");

    // Brush a point on ds-a and verify commit_active_modal_dialog commits it even if selected_points was empty
    app.brush_from_grid_cell(&SheetId::new("ds-a"), GridCellCoord::new(1, 0, 0));
    let doc_snap = app.document().clone();
    app.canvas_dialogs.open_format_selected_points_with_subcols(&doc_snap, &SheetId::new("g-a"), Vec::new());
    app.canvas_dialogs.format_points.hidden = Some(true);
    let outcome = commit_active_modal_dialog(&mut app, |_, _| Ok(Default::default()))?;
    assert!(outcome.handled);
    let g = app.document().graph_sheet(&SheetId::new("g-a")).ok_or("missing g-a")?;
    assert!(!g.spec.point_overrides.is_empty(), "brushed point on ds-a must commit override");
    Ok(())
}

// ============================================================================
// AUD-07-10 & AUD-07-11: Discontiguous selection bounds, stats clearing, excludeWithReason
// ============================================================================
#[test]
fn aud_07_10_and_11_discontiguous_selection_bounds_and_exclude_reason() -> TestResult {
    let mut app = minimal_column_app(3, 2)?;
    let ds = app.document().data_sheet(&SheetId::new("data-1")).ok_or("missing data-1")?.clone();

    // Populate quick_stats first
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let doc_snap = app.document().clone();
    app.grid.refresh_selection_stats(&doc_snap);
    assert!(app.grid.quick_stats.is_some());

    // Set only out-of-bounds col/subcol in selected_coords -> must filter out & clear stale quick_stats
    app.grid.selected_coords = vec![GridCellCoord::new(0, 99, 0), GridCellCoord::new(0, 0, 5)];
    assert!(app.grid.all_selected_coords(&ds).is_empty());
    assert!(app.grid.selected_graph_point_targets(&ds, 2).is_empty());
    app.grid.refresh_selection_stats(&doc_snap);
    assert!(app.grid.quick_stats.is_none(), "stale quick_stats must be cleared when filtered coords are empty");
    assert!(app.grid.extended_stats.is_none());

    // AUD-07-11: ui.grid.excludeWithReason uses all_selected_coords (discontiguous selection)
    app.grid.selected_coords = vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0)];
    let resp = control_call(
        &mut app,
        &json!({
            "id": "excl-1",
            "method": "ui.grid.excludeWithReason",
            "params": { "sheet_id": "data-1", "reason_tag": "outlier", "commit": true }
        }),
    )?;
    assert!(resp.ok, "excludeWithReason failed: {:?}", resp.error);
    let ds_after = app.document().data_sheet(&SheetId::new("data-1")).ok_or("missing data-1")?;
    assert!(ds_after.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded));
    assert!(ds_after.cell(CellCoord::new(2, 1, 0)).is_some_and(|c| c.excluded));
    assert!(ds_after.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| !c.excluded));
    Ok(())
}

// ============================================================================
// AUD-07-14, AUD-07-15, AUD-07-17, AUD-07-19, AUD-07-26, AUD-13-23
// ============================================================================
#[test]
fn aud_07_14_15_17_19_26_and_13_23_control_and_routing_invariants() -> TestResult {
    let mut app = minimal_xy_with_graph_app()?;

    // AUD-07-14: ui.theme.set preserves ui_scale
    app.set_ui_scale(1.5);
    let resp = control_call(&mut app, &json!({ "id": "t1", "method": "ui.theme.set", "params": { "mode": "dark" } }))?;
    assert!(resp.ok);
    assert!((app.ui_scale() - 1.5).abs() < 1e-4, "ui.theme.set must preserve ui_scale");

    // AUD-07-15: command_replaces_project & is_command_read_only
    assert!(command_replaces_project("table.importXlsx", &json!({})));
    assert!(!command_replaces_project("table.importXlsx", &json!({ "merge": true })));
    assert!(command_replaces_project("table.importPrism", &json!({ "merge": false })));
    let mut deep = json!({ "command": "ping" });
    for _ in 0..=MAX_READ_ONLY_NESTING_DEPTH + 2 {
        deep = json!({ "command": "command.run", "params": deep });
    }
    assert!(command_replaces_project("command.run", &deep), "depth overflow must fail-closed (true)");

    assert!(is_read_only_ui_command("ui.editParams.open"));
    assert!(!is_read_only_ui_command("ui.export.allTablesCsv"));
    assert!(is_command_read_only("ui.print.open", None));
    assert!(is_command_read_only("ui.print.open", Some(&json!({ "submit_to_os": false }))));
    assert!(!is_command_read_only("ui.print.open", Some(&json!({ "submit_to_os": true }))));

    // AUD-07-17: render.pdf with path writes file & omits inline data_base64
    let dir = temp_test_dir("aud07-17")?;
    app.session_mut().set_path_policy(ExternalPathPolicy::workdir(&dir)?);
    let pdf_resp = control_call(&mut app, &json!({ "id": "pdf1", "method": "render.pdf", "params": { "sheet_id": "graph-1", "path": "out.pdf" } }))?;
    assert!(pdf_resp.ok, "render.pdf failed: {:?}", pdf_resp.error);
    assert!(pdf_resp.result.get("data_base64").is_none(), "path-based render.pdf must omit data_base64");
    assert!(dir.join("out.pdf").exists());
    let _ = std::fs::remove_dir_all(&dir);

    // AUD-07-19: nonexistent explicit sheet_id rejected
    let bad_fig = control_call(&mut app, &json!({ "id": "f1", "method": "ui.export.figure.open", "params": { "sheet_id": "nope" } }))?;
    assert!(!bad_fig.ok, "ui.export.figure.open must reject nonexistent sheet_id");
    let bad_res = control_call(&mut app, &json!({ "id": "r1", "method": "ui.export.resultsCsv.open", "params": { "sheet_id": "data-xy" } }))?;
    assert!(!bad_res.ok, "ui.export.resultsCsv.open must reject wrong-section sheet_id");
    let bad_brush = control_call(&mut app, &json!({ "id": "b1", "method": "ui.brush.point", "params": { "sheet_id": "graph-1" } }))?;
    assert!(!bad_brush.ok, "ui.brush.point must reject non-data sheet_id");

    // AUD-07-26: locale cycling covers all 16 locales + ar/he Preferences labels
    let mut visited = Vec::new();
    app.set_locale("en");
    for _ in 0..LOCALES.len() {
        let next_loc = contextual_command_params(&app, "ui.locale.set").get("locale").and_then(Value::as_str).ok_or("missing locale")?.to_owned();
        visited.push(next_loc.clone());
        app.set_locale(&next_loc);
    }
    assert_eq!(visited.len(), 16);
    assert!(visited.iter().any(|l| l == "ar") && visited.iter().any(|l| l == "he"), "locale cycle must reach all 16 locales including ar/he");

    app.set_locale("ar");
    let ar_menus = curated_menu_bar(&app);
    let ar_has_pref = format!("{ar_menus:?}").contains("التفضيلات...");
    assert!(ar_has_pref, "Arabic Preferences label must be localized");
    app.set_locale("he");
    let he_menus = curated_menu_bar(&app);
    let he_has_pref = format!("{he_menus:?}").contains("העדפות...");
    assert!(he_has_pref, "Hebrew Preferences label must be localized");

    // AUD-13-23: aligned path keys
    for key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path", "file_path", "path"] {
        assert!(has_non_null_path_param(&json!({ key: "x" })), "expected path key `{key}` detected");
        assert!(has_non_null_path_param(&json!({ "nested": { key: "x" } })), "expected nested path key `{key}` detected");
    }
    Ok(())
}

// ============================================================================
// AUD-07-16 & AUD-07-18: Atomic undo for Delete/Cmd+E & subcol-aware LinkedDataRegion
// ============================================================================
#[test]
fn aud_07_16_and_18_atomic_clear_exclude_and_subcol_linked_region() -> TestResult {
    let mut doc = ProjectDocument::new("LinkSubcol")?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    for c in 0..2 {
        for r in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number((r + 1) as f64);
            grp.data_sets[c].cells[r][1] = Cell::number((r + 10) as f64);
        }
    }
    let mut ds = DataSheet::new(SheetId::new("grp-1"), "Grouped", TableArchetype::Grouped(grp))?;
    // Link ONLY col 0, subcol 0 (rows 0..=1); col 0 subcol 1 is NOT linked!
    ds.linked_regions.push(LinkedDataRegion {
        id: String::from("link-sub0"),
        spec: ImportLinkSpec { source: LinkSource::Clipboard {}, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("Linked subcol 0"),
    });
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(SheetId::new("grp-1"));
    let mut app = UiApp::from_session(Session::from_document(doc));

    let ds_ref = app.document().data_sheet(&SheetId::new("grp-1")).ok_or("missing grp-1")?;
    assert!(linked_region_for_cell_subcol(ds_ref, 0, 0, 0).is_some());
    assert!(linked_region_for_cell_subcol(ds_ref, 0, 0, 1).is_none());

    // Direct typing blocked on linked subcol 0, allowed on unlinked subcol 1
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    assert_eq!(handle_grid_text_input(&app, "5").action, "grid_core.link_read_only_prompt");
    assert!(prepare_grid_type_text_target(&mut app, Some("grid.cell.0.0.0"), None, false, false).is_err());

    app.grid.select_single_sync(GridCellCoord::new(0, 0, 1));
    assert!(!handle_grid_text_input(&app, "5").handled, "unlinked subcol 1 must allow typing");
    assert!(prepare_grid_type_text_target(&mut app, Some("grid.cell.0.0.1"), None, false, false).is_ok());

    // AUD-07-16: discontiguous Delete across 2 cells -> single atomic undo entry
    app.grid.selected_coords = vec![GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 0)];
    let undo_before = app.session().undo_depth();
    let del_out = handle_grid_key_down(&mut app, "Delete", KeyModifiers::default())?;
    assert_eq!(del_out.action, "grid_core.clear_selection");
    assert_eq!(app.session().undo_depth(), undo_before + 1, "discontiguous Delete must be 1 undo step");

    // AUD-07-16: Cmd+E across discontiguous cells -> single atomic undo entry
    let _ = app.session_mut().undo()?;
    app.grid.selected_coords = vec![GridCellCoord::new(0, 0, 1), GridCellCoord::new(2, 1, 0)];
    let undo_before_e = app.session().undo_depth();
    let e_out = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("e"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )?;
    assert_eq!(e_out.action, "shortcut:grid.toggle_exclude");
    assert_eq!(app.session().undo_depth(), undo_before_e + 1, "discontiguous Cmd+E must be 1 undo step");
    Ok(())
}

// ============================================================================
// AUD-07-20, 21, 22, 23, 24, 25: Workspace load reset, locale paste, snapshot staging, recents, power/r9
// ============================================================================
#[test]
fn aud_07_20_through_25_workspace_interaction_search_and_dialogs() -> TestResult {
    // AUD-07-20: load_from_virtual_workspace resets stale shell state including import_warnings
    let mut app = minimal_column_app(2, 2)?;
    app.save_to_virtual_workspace("proj.curvebook")?;
    app.import_warnings.warnings.push(ImportWarningItem { kind: String::from("warn"), source: String::from("test"), message: String::from("stale") });
    app.canvas_dialogs.active_dialog = ActiveDialog::About;
    app.brush_from_grid_cell(&SheetId::new("data-1"), GridCellCoord::new(0, 0, 0));
    app.load_from_virtual_workspace("proj.curvebook")?;
    assert!(app.import_warnings.warnings.is_empty(), "load_from_virtual_workspace must clear import_warnings");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(app.brushed_points.is_empty());

    // AUD-07-21: German locale paste parses "1,5\n2,25" as 1 column of decimals
    app.set_locale("de");
    let stub = Arc::new(StubPlatformClipboardAdapter::new());
    stub.seed_text("1,5\n2,25");
    app.set_clipboard_adapter(stub);
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    let paste_out = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("v"), modifiers: KeyModifiers { ctrl: true, ..Default::default() } },
        ViewportMetrics::default(),
    )?;
    assert_eq!(paste_out.action, "shortcut:table.paste");
    let ds = app.document().data_sheet(&SheetId::new("data-1")).ok_or("missing data-1")?;
    assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.5));
    assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(2.25));

    // AUD-07-21 part 2: handle_key_down propagates shortcut errors instead of swallowing them
    app.shortcuts.set_shortcut("ui.export.figure.open", "F9")?;
    let err_res = dispatch_ui_event(
        &mut app,
        UiInputEvent::KeyDown { key: String::from("F9"), modifiers: KeyModifiers::default() },
        ViewportMetrics::default(),
    );
    assert!(err_res.is_err(), "failing bound shortcut must propagate Err");

    // AUD-07-22: colliding sanitized SheetIds get distinct CSV paths and failed save preserves previous files
    let mut doc_col = ProjectDocument::new("Collide")?;
    doc_col.data_sheets.push(DataSheet::new(
        SheetId::new("exp.1"),
        "Exp Dot",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?),
    )?);
    doc_col.data_sheets.push(DataSheet::new(
        SheetId::new("exp+1"),
        "Exp Plus",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?),
    )?);
    let mut vfolder = VirtualWorkspaceFolder::default();
    let _ = vfolder.save_unpacked_snapshot(&doc_col)?;
    let csv_keys: Vec<&String> = vfolder.files.keys().filter(|k| k.starts_with("sheets/")).collect();
    assert_eq!(csv_keys.len(), 2, "colliding sanitized SheetIds must produce 2 distinct CSV files: {csv_keys:?}");
    let reloaded = vfolder.open_unpacked_snapshot()?;
    assert_eq!(reloaded.data_sheets.len(), 2);

    // AUD-07-23: search_engine evaluates live enabled state for Recent RunCommand entries
    let fresh_app = UiApp::new();
    let mut recents = SearchRecents::default();
    recents.record(&UnifiedSearchItem {
        kind: UnifiedSearchKind::Command,
        title: String::from("Undo"),
        subtitle: String::from("Edit"),
        category_badge: String::from("Command"),
        shortcut: String::from("Cmd+Z"),
        match_spans: Vec::new(),
        enabled: true,
        score: 100,
        action: UnifiedSearchAction::RunCommand { command_id: String::from("edit.undo"), is_ui_command: false },
        completion_text: String::from("Undo"),
    });
    let items = query_unified(&fresh_app, "", 20, &recents);
    let undo_item = items.iter().find(|it| matches!(&it.action, UnifiedSearchAction::RunCommand { command_id, .. } if command_id == "edit.undo"));
    assert!(undo_item.is_some_and(|it| !it.enabled), "edit.undo recent must be disabled on empty undo stack");

    // AUD-07-24: PowerWizardState resets RegressionSlope when switching to TwoSamplePairedT
    let mut pw = PowerWizardState::default();
    pw.set_family(PowerTestFamily::SimpleLinearRegression);
    pw.effect_mode = PowerEffectInputMode::RegressionSlope;
    pw.set_family(PowerTestFamily::TwoSamplePairedT);
    assert_eq!(pw.effect_mode, PowerEffectInputMode::Standardized);

    // AUD-07-25: RandomSampleRowsDialogState clears stale stratify_column on sheet switch
    let mut doc_r9 = ProjectDocument::new("R9")?;
    doc_r9.data_sheets.push(DataSheet::new(
        SheetId::new("s-wide"),
        "Wide",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 5)?),
    )?);
    doc_r9.data_sheets.push(DataSheet::new(
        SheetId::new("s-narrow"),
        "Narrow",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5)?),
    )?);
    let mut dlg = RandomSampleRowsDialogState::default();
    assert!(dlg.open_for_sheet(&doc_r9, &SheetId::new("s-wide")));
    dlg.stratify_column = Some(3);
    assert!(dlg.open_for_sheet(&doc_r9, &SheetId::new("s-narrow")));
    assert_eq!(dlg.stratify_column, None, "switching sheets must reset stale stratify_column");
    Ok(())
}

// ============================================================================
// AUD-08-05, 06, 07, 08, 11, 12, 13, 14, 15, 16, 17, 18: Shell invariants
// ============================================================================
#[test]
fn aud_08_shell_invariants_rulers_history_modals_raster_fit() -> TestResult {
    // AUD-08-05: ui.zoom.fitSheet resets sheet_viewports[*].pan_pt too
    let mut app = minimal_xy_with_graph_app()?;
    app.egui_views.canvas_view.pan_pt = [40.0, -25.0];
    app.egui_views.canvas_view.sheet_viewports.insert(
        SheetId::new("graph-1"),
        curvebook_ui_egui::egui_shell::canvas_view::CanvasPerSheetViewportState { pan_pt: [40.0, -25.0], ..Default::default() },
    );
    let fit_resp = control_call(&mut app, &json!({ "id": "fit1", "method": "ui.zoom.fitSheet" }))?;
    assert!(fit_resp.ok);
    assert_eq!(app.egui_views.canvas_view.pan_pt, [0.0, 0.0]);
    assert_eq!(app.egui_views.canvas_view.sheet_viewports.get(&SheetId::new("graph-1")).map(|v| v.pan_pt), Some([0.0, 0.0]));

    // AUD-08-06: compute_ruler_ticks scales labels with 1-2-5 physical increments & exceeds 32 ticks
    let low_zoom = compute_ruler_ticks(0.0, 0.0, 400.0, 0.25, PhysicalUnit::Inches);
    assert!(low_zoom.len() >= 2);
    assert_eq!(low_zoom[0].1, 0);
    assert_eq!(low_zoom[1].1, 2, "low zoom inches must step by 2 in, not mislabeled 1 in");
    assert!((low_zoom[1].0 - 36.0).abs() < 1e-3);
    let wide_pan = compute_ruler_ticks(-3000.0, 0.0, 600.0, 1.0, PhysicalUnit::Inches);
    assert!(!wide_pan.is_empty() && wide_pan[0].1 > 32, "must emit ticks past index 32 when panned");

    // AUD-08-12: CommitImportWizard rolls back new sheet creation on failed import; ResolveDuplicateAnalysis keeps modal open on error
    let mut wiz_app = minimal_column_app(2, 2)?;
    let sheets_before = wiz_app.document().data_sheets.len();
    wiz_app.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    wiz_app.grid.import_wizard.target_sheet_id = None;
    wiz_app.grid.import_wizard.raw_clipboard_text = String::new(); // empty clipboard fails table.paste
    let mut p_wiz = PendingUiIntents::default();
    p_wiz.push(UiIntent::CommitImportWizard { close_after: true });
    let _ = p_wiz.commit_if_settled(&mut wiz_app, false);
    assert_eq!(wiz_app.document().data_sheets.len(), sheets_before, "failed CommitImportWizard must roll back created sheet");

    let mut dup_err_app = minimal_column_app(2, 2)?;
    dup_err_app.canvas_dialogs.analyze.source_sheet_id = Some(SheetId::new("data-1"));
    dup_err_app.canvas_dialogs.analyze.selected_entry_id = Some(String::from("stats.descriptive"));
    dup_err_app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: SheetId::new("data-1"),
        existing_result_ids: vec![SheetId::new("nonexistent-result")],
        pending_entry_id: String::from("stats.descriptive"),
    };
    let mut p_dup = PendingUiIntents::default();
    p_dup.push(UiIntent::ResolveDuplicateAnalysis { modify_existing_id: Some(SheetId::new("nonexistent-result")) });
    assert!(p_dup.commit_if_settled(&mut dup_err_app, false).is_err());
    assert!(
        matches!(dup_err_app.canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }),
        "failed ResolveDuplicateAnalysis must keep dialog open"
    );

    // AUD-08-13: collect_visible_sheet_ids follows NestedResultsUnderData & hides collapsed children
    let mut doc_nav = ProjectDocument::new("Nav")?;
    doc_nav.data_sheets.push(DataSheet::new(
        SheetId::new("d1"),
        "D1",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?),
    )?);
    doc_nav.data_sheets.push(DataSheet::new(
        SheetId::new("d2"),
        "D2",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?),
    )?);
    let rs1 =
        ResultSheet::new(AnalysisSpec::new(SheetId::new("r1"), SheetId::new("d1"), AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    doc_nav.result_sheets.push(rs1);
    let mut nav_app = UiApp::from_session(Session::from_document(doc_nav));
    nav_app.nav.nesting_mode = NavigatorNestingMode::NestedResultsUnderData;
    nav_app.sync_views();
    let mut nav_state = LeadingDockEguiState::default();
    let expanded = collect_visible_sheet_ids(&nav_app, &nav_state);
    assert_eq!(expanded, vec![SheetId::new("d1"), SheetId::new("r1"), SheetId::new("d2")]);
    nav_state.nested_collapsed_parents.insert(SheetId::new("d1"));
    let collapsed = collect_visible_sheet_ids(&nav_app, &nav_state);
    assert_eq!(collapsed, vec![SheetId::new("d1"), SheetId::new("d2")], "collapsed parent must hide nested child");

    // AUD-08-14: sync_history_with_document adjusts cursor on predecessor deletion & deduplicates adjacent
    let mut doc_h = ProjectDocument::new("Hist")?;
    for id in ["s1", "s2", "s3", "s4"] {
        doc_h.data_sheets.push(DataSheet::new(
            SheetId::new(id),
            id,
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1)?),
        )?);
    }
    let mut views = EguiShellViewState::default();
    for id in ["s1", "s2", "s3", "s4"] {
        views.record_active_sheet_visit(&SheetId::new(id));
    }
    views.history_cursor = 2; // at s3, s4 is forward!
    doc_h.data_sheets.retain(|ds| ds.id.as_str() != "s1");
    doc_h.active_sheet_id = Some(SheetId::new("s3"));
    views.sync_history_with_document(&doc_h);
    assert_eq!(views.active_sheet_history, vec![SheetId::new("s2"), SheetId::new("s3"), SheetId::new("s4")]);
    assert_eq!(views.history_cursor, 1, "cursor must shift left to stay on s3");
    assert!(views.can_go_forward(), "forward history [s4] must be preserved");
    Ok(())
}

#[test]
fn aud_08_shell_invariants_modals_raster_fit_and_headless() -> TestResult {
    let mut app = minimal_xy_with_graph_app()?;

    // AUD-08-15: progress_modal triggers EguiProgressModal backdrop; huge coords & bad hover_n rejected
    let views = EguiShellViewState {
        progress_modal: Some(ProgressModalState {
            operation_label: String::from("Fitting"),
            detail: String::from("Step 1"),
            progress_0_to_1: Some(0.5),
            cancellable: true,
            cancel_requested: false,
        }),
        ..EguiShellViewState::default()
    };
    let dec = HybridCoexistenceArbiter::evaluate_with_views(&app, &views, false);
    assert_eq!(dec.modal_surface, HybridModalSurface::EguiProgressModal);
    assert!(dec.modal_backdrop_active);

    assert!(parse_pointer_gesture_params(&json!({ "kind": "click", "x": 1e300, "y": 10.0 }), 0.0).is_err());
    let mut dummy_meta = FrameInspectMeta::default();
    dummy_meta.record(FrameInspectNodeMeta {
        egui_id: egui::Id::unique("power.curve_hover"),
        accesskit_id: egui::Id::unique("power.curve_hover").accesskit_id(),
        widget_id: String::from("power.curve_hover"),
        role: String::from("canvas"),
        label: String::from("Power Curve"),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        unclipped_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0)),
        clipped_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 50.0)),
        z_layer: ZLayer::ModalDialog,
        ellipsized: false,
        full_label: None,
        action_rank: None,
    });
    assert!(resolve_power_curve_hover_target(&mut app, &dummy_meta, &json!({ "hover_n": "not-a-number" })).is_err());

    // AUD-08-16: crop_raster_rgba validates buffer length before allocation
    let truncated = RasterImage { width_px: 1000, height_px: 1000, dpi: 72, rgba: vec![0u8; 16] };
    assert!(crop_raster_rgba(&truncated, 0, 0, 1000, 1000).is_err());

    // AUD-08-17: StatParamsIntent::ToggleSchematic on FactorialAnova checks r1_paired_or_matched_allowed
    let mut doc_f = ProjectDocument::new("Fact")?;
    let grp_summary = GroupedTable::new(SubcolumnMode::MeanSdN, 2, 2)?;
    doc_f.data_sheets.push(DataSheet::new(SheetId::new("g-sum"), "Summary", TableArchetype::Grouped(grp_summary))?);
    let mut app_f = UiApp::from_session(Session::from_document(doc_f));
    app_f.canvas_dialogs.edit_params.draft = Some(AnalysisSpec::new(
        SheetId::new("r-fact"),
        SheetId::new("g-sum"),
        AnalysisKind::FactorialAnova(curvebook_doc::FactorialAnovaSpec::default()),
    )?);
    let mut pending = PendingUiIntents::default();
    pending.push(UiIntent::StatParams(StatParamsIntent::ToggleSchematic));
    let _ = pending.commit_if_settled(&mut app_f, false)?;
    assert!(!app_f.canvas_dialogs.edit_params.paired_or_matched, "MeanSdN summary table must not enable RM via ToggleSchematic");

    // AUD-08-18: fit_right_group with switcher == 0.0 reserves zero phantom switcher width
    let tight = fit_right_group(&RightWidths {
        room: 180.0,
        has_pinned_badge: false,
        switcher: 0.0,
        field: 176.0,
        icon: 48.0,
        gap: 6.0,
        trailing_extra: 56.0,
    });
    assert!(!tight.switcher);
    assert_eq!(tight.switcher_w, 0.0);
    assert_eq!(tight.search, SearchVisibility::Field, "180pt room with switcher=0 must fit 176pt search field");

    // AUD-08-07, AUD-08-08 & AUD-08-11: headless render honors views.launcher_open, disables status.summary under modal,
    // and respects rebound edit.copy without double-copying on Event::Copy
    let mut ctx_app = minimal_column_app(2, 2)?;
    let ctx = egui::Context::default();
    let mut live_views = EguiShellViewState { launcher_open: true, ..EguiShellViewState::default() };
    let mut meta_out = FrameInspectMeta::default();
    let mut intents_out = PendingUiIntents::default();
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &ctx_app, &mut live_views, &mut meta_out, &mut intents_out);
        },
    )
    .drop_without_applying_deltas();
    let summary = meta_out.find_by_widget_id("status.summary").ok_or("missing status.summary")?;
    assert_eq!(summary.label, "Welcome Hub", "footer must show Welcome Hub when views.launcher_open is true");

    // AUD-08-08: When Cmd+C is bound to another command or table.paste is rebound away from Cmd+V,
    // synthetic Event::Copy / Event::Paste must NOT bypass the remapped shortcut bindings.
    live_views.launcher_open = false;
    ctx_app.shortcuts.set_shortcut("ui.gallery.toggle", "Cmd+C")?;
    ctx_app.shortcuts.set_shortcut("table.paste", "F8")?;
    let mut meta_copy = FrameInspectMeta::default();
    let mut intents_copy = PendingUiIntents::default();
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
            events: vec![egui::Event::Copy, egui::Event::Paste(String::from("99"))],
            ..Default::default()
        },
        |ui| {
            let _ = render_egui_shell_frame(ui, &ctx_app, &mut live_views, &mut meta_copy, &mut intents_copy);
        },
    )
    .drop_without_applying_deltas();
    assert!(
        !intents_copy.intents().iter().any(|i| matches!(i, UiIntent::GridCopySelection | UiIntent::GridPasteFromClipboard { .. })),
        "Event::Copy/Paste must not bypass rebound shortcuts: {:?}",
        intents_copy.intents()
    );

    ctx_app.canvas_dialogs.active_dialog = ActiveDialog::About;
    let mut meta_modal = FrameInspectMeta::default();
    let mut intents_modal = PendingUiIntents::default();
    ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))), ..Default::default() },
        |ui| {
            let _ = render_egui_shell_frame(ui, &ctx_app, &mut live_views, &mut meta_modal, &mut intents_modal);
        },
    )
    .drop_without_applying_deltas();
    let summary_modal = meta_modal.find_by_widget_id("status.summary").ok_or("missing status.summary under modal")?;
    assert!(!summary_modal.enabled, "status.summary must be disabled when a modal is active");
    Ok(())
}

// ============================================================================
// AUD-13-10, 11, 12: Control/MCP invoke_menu, switch_tab/answer_dialog, screenshot budget
// ============================================================================
#[test]
fn aud_13_10_11_12_invoke_menu_tabs_and_screenshot_budget() -> TestResult {
    let mut app = minimal_xy_with_graph_app()?;

    // AUD-13-10: empty {} params in ui.invoke_menu treated as omitted in --human-path-only
    app.set_human_path_only(true);
    let menu_res = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.rulers.toggle", "params": {} }))?;
    assert_eq!(menu_res.get("invoked").and_then(Value::as_bool), Some(true));
    let non_empty_err = dispatch_gui_action(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.rulers.toggle", "params": { "visible": true } }));
    assert!(non_empty_err.is_err(), "non-empty params must still be blocked in human-path-only mode");
    app.set_human_path_only(false);

    // AUD-13-11: integer tab index in ui.switch_tab & top-level `choice` in ui.answer_dialog
    let tab_val = json!({ "tab": 2 });
    let parsed_tab = parse_switch_tab_params(&tab_val)?;
    match parsed_tab.target {
        curvebook_ui_egui::egui_shell::inspect_bridge::ParsedSwitchTabTarget::Tab(t) => assert_eq!(t, "2"),
        curvebook_ui_egui::egui_shell::inspect_bridge::ParsedSwitchTabTarget::Sheet(_) => return Err("expected Tab".into()),
    }
    let dlg_val = json!({ "choice": "create_new", "action": "ok" });
    let parsed_dlg = parse_answer_dialog_params(&dlg_val)?;
    assert_eq!(parsed_dlg.choice, Some("create_new"));
    let mut dup_app = minimal_column_app(2, 2)?;
    dup_app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
        source_sheet_id: SheetId::new("data-1"),
        existing_result_ids: vec![SheetId::new("res-1")],
        pending_entry_id: String::from("stats.descriptive"),
    };
    let views = dup_app.egui_views.clone();
    let choice_only_val = json!({ "choice": "modify_existing" });
    let parsed_choice_only = parse_answer_dialog_params(&choice_only_val)?;
    let patch_res = prepare_answer_dialog_patch(&mut dup_app, &views, false, &parsed_choice_only)?;
    assert!(patch_res.is_some(), "top-level choice must forward into DuplicateAnalysisPrompt patch");

    // AUD-13-12: ui.screenshot emits data_base64 only (no duplicate png_base64) + over-budget branches
    let shot = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "width_px": 160, "height_px": 120 }))?;
    assert!(shot.get("data_base64").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert!(shot.get("png_base64").is_none(), "must not duplicate PNG base64 in png_base64");
    assert!(shot.get("data_base64_omitted").is_none());

    let fake_png = vec![0x89u8; 4096];
    let shot_target = RenderTargetSpec { width_px: 1280, height_px: 800, dpi: 144 };
    let shot_dir = temp_test_dir("aud13-12-budget")?;
    let shot_out = shot_dir.join("large.png");
    let shot_out_str = shot_out.to_str().ok_or("non-utf8 temp path")?;

    // Branch 1: over budget WITH path -> writes file atomically & returns path-only metadata without data_base64
    let mut path_wrote = false;
    let omitted_val = finalize_screenshot_payload_with_limit("full_window", shot_target, &fake_png, Some(shot_out_str), 1024, |dest, bytes| {
        path_wrote = true;
        std::fs::write(dest, bytes).map_err(|e| curvebook_engine::EngineError::with_code(ErrorCode::InternalError, e.to_string()))
    })?;
    assert!(path_wrote, "over-budget path screenshot must write file to disk");
    assert_eq!(std::fs::read(&shot_out)?, fake_png);
    assert!(omitted_val.get("data_base64").is_none(), "over-budget path screenshot must omit data_base64");
    assert!(omitted_val.get("png_base64").is_none(), "over-budget path screenshot must not emit png_base64");
    assert_eq!(omitted_val.get("data_base64_omitted").and_then(Value::as_bool), Some(true));
    assert!(omitted_val.get("omitted_reason").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert_eq!(omitted_val.get("path").and_then(Value::as_str), Some(shot_out_str));

    // Branch 2: over budget WITHOUT path -> fails with ResourceLimit and performs zero disk writes
    let mut no_path_wrote = false;
    let over_err = finalize_screenshot_payload_with_limit("full_window", shot_target, &fake_png, None, 1024, |_dest, _bytes| {
        no_path_wrote = true;
        Ok(())
    })
    .err()
    .ok_or("expected ResourceLimit error when path is absent and inline frame exceeds budget")?;
    assert_eq!(over_err.code, ErrorCode::ResourceLimit);
    assert!(!no_path_wrote, "over-budget screenshot without path must not write to disk");

    // Branch 3: even path-only metadata exceeds limit -> fails with ResourceLimit BEFORE touching disk
    let shot_never = shot_dir.join("never.png");
    let shot_never_str = shot_never.to_str().ok_or("non-utf8 temp path")?;
    let mut meta_over_wrote = false;
    let meta_over_err = finalize_screenshot_payload_with_limit("full_window", shot_target, &fake_png, Some(shot_never_str), 64, |_dest, _bytes| {
        meta_over_wrote = true;
        Ok(())
    })
    .err()
    .ok_or("expected ResourceLimit error when metadata itself exceeds frame limit")?;
    assert_eq!(meta_over_err.code, ErrorCode::ResourceLimit);
    assert!(!meta_over_wrote && !shot_never.exists(), "metadata-over-budget must fail before touching disk");
    let _ = std::fs::remove_dir_all(&shot_dir);
    Ok(())
}

// ============================================================================
// AUD-13-13: command.batch structured failure detail (top-level & nested `details`)
// ============================================================================
#[test]
fn aud_13_13_batch_structured_failure_details_and_cap() -> TestResult {
    let mut app = minimal_xy_with_graph_app()?;
    let dir = temp_test_dir("aud13-13")?;
    app.session_mut().set_path_policy(ExternalPathPolicy::workdir(&dir)?);
    let batch_fail = control_call(
        &mut app,
        &json!({
            "id": "b-fail",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    { "command": "render.pdf", "params": { "sheet_id": "graph-1", "path": "step0.pdf" } },
                    { "command": "sheet.select", "params": { "sheet_id": "missing-sheet" } }
                ]
            }
        }),
    )?;
    assert!(!batch_fail.ok);
    let detail = batch_fail.error_detail.ok_or("missing structured batch error_detail")?;
    assert!(detail.get("code").and_then(Value::as_str).is_some_and(|c| !c.is_empty()));
    let nested = detail.get("details").ok_or("missing nested `details` object for MCP parse_control_error")?;
    for obj in [&detail, nested] {
        assert_eq!(obj.get("failed_index").and_then(Value::as_u64), Some(1));
        assert_eq!(obj.get("failed_command_id").and_then(Value::as_str), Some("sheet.select"));
        assert_eq!(obj.get("completed_steps").and_then(Value::as_u64), Some(1));
        assert_eq!(obj.get("rolled_back").and_then(Value::as_bool), Some(true));
        assert_eq!(obj.get("filesystem_side_effects_persisted").and_then(Value::as_bool), Some(true));
    }

    let too_many: Vec<Value> = (0..1005).map(|_| json!({ "command": "ping" })).collect();
    let limit_fail_atomic = control_call(
        &mut app,
        &json!({ "id": "b-lim-atomic", "method": "command.batch", "params": { "atomic": true, "commands": too_many.clone() } }),
    )?;
    assert!(!limit_fail_atomic.ok);
    let lim_atomic_detail = limit_fail_atomic.error_detail.ok_or("missing atomic batch limit error_detail")?;
    assert_eq!(lim_atomic_detail.get("code").and_then(Value::as_str), Some("RESOURCE_LIMIT"));
    let lim_atomic_nested = lim_atomic_detail.get("details").ok_or("missing nested details on atomic batch limit")?;
    for obj in [&lim_atomic_detail, lim_atomic_nested] {
        assert!(obj.get("failed_index").is_none(), "pre-execution cap rejection must omit failed_index");
        assert_eq!(obj.get("completed_steps").and_then(Value::as_u64), Some(0));
        assert_eq!(obj.get("rolled_back").and_then(Value::as_bool), Some(true));
        assert_eq!(obj.get("filesystem_side_effects_persisted").and_then(Value::as_bool), Some(false));
    }

    let limit_fail = control_call(&mut app, &json!({ "id": "b-lim", "method": "command.batch", "params": { "commands": too_many } }))?;
    assert!(!limit_fail.ok);
    let lim_detail = limit_fail.error_detail.ok_or("missing batch limit error_detail")?;
    assert_eq!(lim_detail.get("code").and_then(Value::as_str), Some("RESOURCE_LIMIT"));
    let lim_nested = lim_detail.get("details").ok_or("missing nested details on non-atomic batch limit")?;
    for obj in [&lim_detail, lim_nested] {
        assert_eq!(obj.get("completed_steps").and_then(Value::as_u64), Some(0));
        assert_eq!(obj.get("rolled_back").and_then(Value::as_bool), Some(false));
        assert_eq!(obj.get("filesystem_side_effects_persisted").and_then(Value::as_bool), Some(false));
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

// ============================================================================
// AUD-13-16 & AUD-13-18: Live TeX equation parity & GLM/Spline/ROC suppression
// ============================================================================
#[test]
fn aud_13_16_18_live_equation_tex_and_glm_roc_suppression() -> TestResult {
    let mut eq_doc = ProjectDocument::new("EqDoc")?;
    let xy = XyTable::new(SubcolumnMode::Single, 2, 4)?;
    let ds = DataSheet::new(SheetId::new("d-xy"), "XY", TableArchetype::Xy(xy))?;
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-xy"), &ds)?;
    gs.initialized = true;
    eq_doc.data_sheets.push(ds);
    eq_doc.graph_sheets.push(gs.clone());

    // 1. SimpleLogisticRegression with dataset-scoped scalars + secondary "ROC" overlay -> compact logistic TeX (no 20-scalar dump, ROC ignored)
    let mut rs_log = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("r-log"),
        SheetId::new("d-xy"),
        AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()),
    )?)?;
    rs_log.status = ResultStatus::UpToDate;
    rs_log.scalars.push(ResultScalarEntry { key: String::from("beta0"), value: -2.5, units: None, data_set_index: Some(0) });
    rs_log.scalars.push(ResultScalarEntry { key: String::from("beta1"), value: 0.0001234, units: None, data_set_index: Some(0) });
    rs_log.scalars.push(ResultScalarEntry { key: String::from("AIC"), value: 42.0, units: None, data_set_index: Some(0) });
    rs_log.fitted_curves.push(FittedCurveOverlay {
        equation_id: String::from("ROC"),
        data_set_index: 0,
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 0.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 1.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    eq_doc.result_sheets.push(rs_log);

    // 2. LinearRegression with fixed_point (1, 5) + force_through_origin=true -> preserves intercept
    let lin_spec = LinearRegressionSpec { force_through_origin: true, fixed_point: Some((1.0, 5.0)), ..Default::default() };
    let mut rs_lin = ResultSheet::new(AnalysisSpec::new(SheetId::new("r-lin"), SheetId::new("d-xy"), AnalysisKind::LinearRegression(lin_spec))?)?;
    rs_lin.status = ResultStatus::UpToDate;
    rs_lin.scalars.push(ResultScalarEntry { key: String::from("Slope"), value: 2.0, units: None, data_set_index: Some(0) });
    rs_lin.scalars.push(ResultScalarEntry { key: String::from("YIntercept"), value: 3.0, units: None, data_set_index: Some(0) });
    eq_doc.result_sheets.push(rs_lin);

    // 3. NonlinearRegression custom DSL with <A> and <B> branches
    let mut nl_spec = NonlinearFitSpec::new("custom_dsl");
    nl_spec.custom_dsl = Some(String::from("<A>Y = a*X\n<B>Y = b*X + c"));
    let mut rs_nl = ResultSheet::new(AnalysisSpec::new(SheetId::new("r-nl"), SheetId::new("d-xy"), AnalysisKind::NonlinearRegression(nl_spec))?)?;
    rs_nl.status = ResultStatus::UpToDate;
    rs_nl.scalars.push(ResultScalarEntry { key: String::from("b"), value: 4.0, units: None, data_set_index: Some(1) });
    rs_nl.scalars.push(ResultScalarEntry { key: String::from("c"), value: 7.0, units: None, data_set_index: Some(1) });
    eq_doc.result_sheets.push(rs_nl);

    // 4. FitSplineLowess with fitted_curves -> suppressed in both dialog and live TeX
    let mut rs_spline = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("r-spline"),
        SheetId::new("d-xy"),
        AnalysisKind::FitSplineLowess(curvebook_doc::SplineLowessSpec::default()),
    )?)?;
    rs_spline.status = ResultStatus::UpToDate;
    rs_spline.fitted_curves.push(FittedCurveOverlay {
        equation_id: String::from("lowess"),
        data_set_index: 0,
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 0.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 1.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    eq_doc.result_sheets.push(rs_spline);

    // 5. GeneralizedLinearModel (Logistic) with "ROC" overlay -> suppressed in both dialog and live TeX (no unknown fallback dump)
    let mut rs_glm = ResultSheet::new(AnalysisSpec::new(
        SheetId::new("r-glm"),
        SheetId::new("d-xy"),
        AnalysisKind::GeneralizedLinearModel(GlmSpec {
            family: curvebook_doc::GlmFamily::Logistic,
            response_column: 0,
            predictor_columns: vec![1],
            reference_levels: Vec::new(),
            compute_vif: false,
        }),
    )?)?;
    rs_glm.status = ResultStatus::UpToDate;
    rs_glm.scalars.push(ResultScalarEntry { key: String::from("beta0"), value: -1.0, units: None, data_set_index: Some(0) });
    rs_glm.scalars.push(ResultScalarEntry { key: String::from("beta1"), value: 0.5, units: None, data_set_index: Some(0) });
    rs_glm.fitted_curves.push(FittedCurveOverlay {
        equation_id: String::from("ROC"),
        data_set_index: 0,
        converged: true,
        ambiguous: false,
        points: vec![
            CurveSamplePoint { x: 0.0, y: 0.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
            CurveSamplePoint { x: 1.0, y: 1.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None },
        ],
    });
    eq_doc.result_sheets.push(rs_glm);

    let log_tex = resolve_live_equation_tex(&eq_doc, &SheetId::new("r-log"), 0, EquationDisplayMode::Substituted).ok_or("expected logistic TeX")?;
    assert!(log_tex.contains("P(Y=1)") && log_tex.contains("0.0001234") && !log_tex.contains("AIC"), "unexpected logistic TeX: {log_tex}");

    let lin_tex = resolve_live_equation_tex(&eq_doc, &SheetId::new("r-lin"), 0, EquationDisplayMode::Substituted).ok_or("expected linear TeX")?;
    assert!(lin_tex.contains("+ 3"), "fixed_point linear fit must preserve non-zero Y-intercept: {lin_tex}");

    let nl_tex_b =
        resolve_live_equation_tex(&eq_doc, &SheetId::new("r-nl"), 1, EquationDisplayMode::Substituted).ok_or("expected DSL B-branch TeX")?;
    assert!(nl_tex_b.contains('4') && nl_tex_b.contains('7'), "dataset B must render <B> DSL branch: {nl_tex_b}");

    assert!(resolve_live_equation_tex(&eq_doc, &SheetId::new("r-spline"), 0, EquationDisplayMode::Substituted).is_none());
    assert!(
        resolve_live_equation_tex(&eq_doc, &SheetId::new("r-glm"), 0, EquationDisplayMode::Substituted).is_none(),
        "GeneralizedLinearModel with ROC overlay must not fall back to unknown model TeX"
    );

    let mut eq_dlg = InsertModelEquationDialogState::default();
    eq_dlg.open_for_graph(&gs, &eq_doc);
    assert!(eq_dlg.available_result_ids.contains(&SheetId::new("r-log")));
    assert!(eq_dlg.available_result_ids.contains(&SheetId::new("r-lin")));
    assert!(eq_dlg.available_result_ids.contains(&SheetId::new("r-nl")));
    assert!(!eq_dlg.available_result_ids.contains(&SheetId::new("r-spline")), "FitSplineLowess must not appear in InsertModelEquation dialog");
    assert!(!eq_dlg.available_result_ids.contains(&SheetId::new("r-glm")), "GeneralizedLinearModel must not appear in InsertModelEquation dialog");

    Ok(())
}
