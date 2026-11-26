#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Targeted Wave 2 modal remediation integration tests covering `AUD-10-01`..`56`,
//! modal halves of `AUD-13` (`02`, `15`, `16`, `17`), and `EXT-01`.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec, DataSheet, DemingErrorSpec,
    DiagnosticPlotKind, ErrorPropagationMode, FactorialAnovaSpec, FramePreset, GraphKind, InfoSheet, KappaWeighting, LinearRegressionSpec,
    MathTransformPreset, ModelComparisonMethod, ModelComparisonQuestion, ModelComparisonSpec, MultipleRowTestsSpec, MultipleVariablesTable,
    MultivariateSpec, OneWayMethod, ParameterRule, ParameterScope, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SheetId, SimpleLogisticSpec, SplineLowessMethod, SplineLowessSpec, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec,
    VariableRole, XyTable,
};
use curvebook_engine::{ExternalPathPolicy, Session};
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AnalysisCategory, AxisTarget};
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::egui_shell::app_modals::PreferencesCategory;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::grid_view::ImportOriginKind;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{StubPlatformClipboardAdapter, UiApp, normalize_shortcut_combo};
use egui::{Key, Modifiers, Vec2};
use serde_json::json;

use common::{ShellTestState, make_shell_driver_with_state, set_textbox, sync_app_to_driver, temp_dir};

fn seed_xy_app(xs: &[f64], ys: &[f64]) -> (UiApp, SheetId) {
    let mut tbl = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    tbl.x_title = String::from("Conc");
    tbl.data_sets[0].title = String::from("Response");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        tbl.x_cells[r][0] = Cell::number(x);
        tbl.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sid = SheetId::try_new("xy-1").unwrap();
    let mut doc = ProjectDocument::new("XY Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "Dose Response", TableArchetype::Xy(tbl)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

fn seed_column_app(cols: &[(&str, &[f64])], hint: ColumnDesignHint) -> (UiApp, SheetId) {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut tbl = ColumnTable::new(hint, cols.len(), rows).unwrap();
    for (ci, (title, vals)) in cols.iter().enumerate() {
        tbl.data_sets[ci].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tbl.data_sets[ci].cells[r][0] = Cell::number(v);
        }
    }
    let sid = SheetId::try_new("col-1").unwrap();
    let mut doc = ProjectDocument::new("Column Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "Groups", TableArchetype::Column(tbl)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

fn seed_mv_app(col_count: usize, rows: usize) -> (UiApp, SheetId) {
    let mut mv = MultipleVariablesTable::new(col_count, rows).unwrap();
    for (c, vc) in mv.variables.iter_mut().enumerate() {
        vc.title = format!("V{}", c + 1);
        if c == 0 {
            vc.role = VariableRole::Categorical;
            for r in 0..rows {
                vc.cells[r] = Cell::text(if r % 2 == 0 { "Ctrl" } else { "Tx" });
            }
        } else {
            vc.role = VariableRole::Continuous;
            for r in 0..rows {
                vc.cells[r] = Cell::number(((r + 1) * (c + 1)) as f64);
            }
        }
    }
    let sid = SheetId::try_new("mv-1").unwrap();
    let mut doc = ProjectDocument::new("MV Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "MV Sheet", TableArchetype::MultipleVariables(mv)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

fn make_driver(app: UiApp) -> curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::new(1280.0, 800.0, 1.0), tokens)
}

#[test]
fn ext_01_default_dose_response_and_fit_modal_log_x_warning_banner() {
    // 1. Raw non-log10 concentrations -> AnalyzeSheet nonlinear default switches to `agonist_vs_response_4pl`.
    let raw_xs = [0.01, 0.1, 1.0, 10.0, 100.0, 1000.0];
    let ys = [5.0, 12.0, 48.0, 88.0, 97.0, 99.0];
    let (mut app_raw, sid_raw) = seed_xy_app(&raw_xs, &ys);
    let doc = app_raw.document().clone();
    assert!(app_raw.canvas_dialogs.open_analyze_dialog(&doc, &sid_raw));
    app_raw.canvas_dialogs.analyze.select_catalog_entry("fit.nonlinear");
    let mut driver_raw = make_driver(app_raw);
    driver_raw.click_widget_id("modal.ok").unwrap();
    driver_raw.run_settled();

    let rs_raw = driver_raw.state().app.document().result_sheets.last().expect("created fit sheet");
    let AnalysisKind::NonlinearRegression(nl_raw) = &rs_raw.analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(nl_raw.equation_id, "agonist_vs_response_4pl");
    assert!(
        driver_raw.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_none(),
        "auto-switched non-log equation must not show log-X warning"
    );

    // 2. Log10 X values -> AnalyzeSheet keeps `log_agonist_vs_response_4pl`.
    let log_xs = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let (mut app_log, sid_log) = seed_xy_app(&log_xs, &ys);
    let doc_log = app_log.document().clone();
    assert!(app_log.canvas_dialogs.open_analyze_dialog(&doc_log, &sid_log));
    app_log.canvas_dialogs.analyze.select_catalog_entry("fit.nonlinear");
    let mut driver_log = make_driver(app_log);
    driver_log.click_widget_id("modal.ok").unwrap();
    driver_log.run_settled();

    let rs_log = driver_log.state().app.document().result_sheets.last().expect("created log fit sheet");
    let AnalysisKind::NonlinearRegression(nl_log) = &rs_log.analysis.kind else {
        panic!("expected NonlinearRegression");
    };
    assert_eq!(nl_log.equation_id, "log_agonist_vs_response_4pl");

    // 3. Explicit log-X equation on raw X data shows inline warning banner + one-click switch in `fit_modal`.
    let (mut app_warn, sid_warn) = seed_xy_app(&raw_xs, &ys);
    let out = app_warn
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": sid_warn.as_str(),
                "kind": "nonlinear_regression",
                "equation_id": "log_agonist_vs_response_4pl",
            }),
        )
        .unwrap();
    let rs_id = SheetId::try_new(out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc_w = app_warn.document().clone();
    assert!(app_warn.canvas_dialogs.open_edit_parameters(&doc_w, &rs_id));
    let mut driver = make_driver(app_warn);

    assert!(driver.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_some(), "expected EXT-01 log-X warning banner");
    assert!(driver.frame_meta().find_by_widget_id("fit.warning.switch_non_log").is_some(), "expected one-click switch button");
    driver.click_widget_id("fit.warning.switch_non_log").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("fit.warning.log_x_scale").is_none(), "warning must disappear after switching to non-log model");
    let draft = driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap();
    let AnalysisKind::NonlinearRegression(nl_after) = &draft.kind else {
        panic!("expected NonlinearRegression draft");
    };
    assert_eq!(nl_after.equation_id, "agonist_vs_response_4pl");
}

#[test]
fn aud_10_02_factorial_formula_capped_for_25_factors() {
    let (mut app, sid) = seed_mv_app(26, 6);
    let spec = AnalysisSpec::new(
        "res-fac25",
        sid,
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            response_column: Some(25),
            multifactor_columns: (0..25).collect(),
            max_interaction_order: 25,
            ..FactorialAnovaSpec::default()
        }),
    )
    .unwrap();
    app.canvas_dialogs.edit_params.result_sheet_id = Some(spec.id.clone());
    app.canvas_dialogs.edit_params.draft = Some(spec.clone());
    app.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: spec.id };
    let driver = make_driver(app);
    let bar = driver.frame_meta().find_by_widget_id("edit.formula_bar").expect("formula bar should render instantaneously");
    let text = bar.value.as_ref().and_then(|v| v.as_str()).unwrap_or(&bar.label);
    assert!(text.contains("^25") || text.contains("25 factors"), "expected bounded shorthand formula, got: {text}");
}

#[test]
fn aud_10_03_05_20_21_and_aud_13_02_import_wizard_hardening() {
    // 1. AUD-10-20 / AUD-13-02: ExternalPathPolicy::RejectExternal disables path & Load Sheets; Workdir blocks escape.
    let mut app_reject = UiApp::from_session(Session::new().with_path_policy(ExternalPathPolicy::RejectExternal));
    app_reject.grid.import_wizard.open = true;
    app_reject.grid.import_wizard.origin_kind = ImportOriginKind::XlsxWorkbook;
    app_reject.grid.import_wizard.external_file_path = String::from("/etc/passwd");
    let drv_rej = make_driver(app_reject);
    assert!(!drv_rej.frame_meta().find_by_widget_id("import_paste.input.path").unwrap().enabled);
    assert!(!drv_rej.frame_meta().find_by_widget_id("import_wizard.xlsx.load_path").unwrap().enabled);
    assert!(!drv_rej.frame_meta().find_by_widget_id("import_wizard.xlsx.select_all").unwrap().enabled);
    assert!(!drv_rej.frame_meta().find_by_widget_id("import_wizard.xlsx.select_none").unwrap().enabled);

    let dir = temp_dir("aud10_20_wd").unwrap();
    let policy = ExternalPathPolicy::workdir(dir.path()).unwrap();
    let mut app_wd = UiApp::from_session(Session::new().with_path_policy(policy));
    app_wd.grid.import_wizard.open = true;
    app_wd.grid.import_wizard.origin_kind = ImportOriginKind::XlsxWorkbook;
    app_wd.grid.import_wizard.external_file_path = String::from("../outside.xlsx");
    let mut drv_wd = make_driver(app_wd);
    drv_wd.click_widget_id("import_wizard.xlsx.load_path").unwrap();
    drv_wd.run_settled();
    assert!(drv_wd.state().app.grid.import_wizard.xlsx_error.as_deref().is_some_and(|e| e.contains("workdir policy")));
    assert!(drv_wd.state().app.grid.import_wizard.xlsx_sheets.is_empty());

    // 2. AUD-10-21: Empty project atomic import preserves Info constants & summary note in a single undo step.
    let mut app_empty = UiApp::new();
    app_empty.grid.import_wizard.open = true;
    app_empty.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    app_empty.grid.import_wizard.delimiter = ',';
    app_empty.grid.import_wizard.raw_clipboard_text = String::from("Instrument: PlateReader\nA,B\n1,2\n3,4");
    let mut drv_empty = make_driver(app_empty);
    drv_empty.click_widget_id("import_paste.tab.notes").unwrap();
    drv_empty.click_widget_id("import_paste.opt.extract_info_metadata").unwrap();
    drv_empty.click_widget_id("import_paste.opt.add_floating_note").unwrap();
    drv_empty.click_widget_id("modal.ok").unwrap();
    drv_empty.run_settled();
    assert!(!drv_empty.state().app.grid.import_wizard.open, "wizard should close after atomic import");
    assert_eq!(drv_empty.state().app.document().data_sheets.len(), 1);
    assert_eq!(drv_empty.state().app.session().undo_depth(), 1, "import + info writes must be a single undo step");
    let info = drv_empty.state().app.document().info_sheets.first().expect("paired InfoSheet created");
    assert!(info.constants.iter().any(|c| c.name == "Instrument"), "extracted header constant must persist on empty project");
    assert!(info.notes_markdown.contains("[Import Summary]"), "summary note must persist on empty project");

    // 3. AUD-10-21 failure atomicity: frozen target sheet fails import AND leaves InfoSheet completely untouched!
    let mut doc_f = ProjectDocument::new("Frozen Info").unwrap();
    let sid_f = SheetId::try_new("col-f").unwrap();
    let info_f_id = SheetId::try_new("info-f").unwrap();
    let mut info_sheet_f = InfoSheet::new_preseeded(info_f_id.clone(), "Info").unwrap();
    info_sheet_f.constants.clear();
    info_sheet_f.notes_markdown.clear();
    doc_f.info_sheets.push(info_sheet_f);
    let mut ds_f =
        DataSheet::new(sid_f.clone(), "Frozen Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap()))
            .unwrap();
    ds_f.info_sheet_id = Some(info_f_id.clone());
    doc_f.data_sheets.push(ds_f);
    doc_f.active_sheet_id = Some(sid_f.clone());
    let mut app_frozen = UiApp::from_session(Session::from_document(doc_f));
    app_frozen.execute_command("sheet.freeze", json!({ "sheet_id": sid_f.as_str() })).unwrap();
    app_frozen.grid.import_wizard.open = true;
    app_frozen.grid.import_wizard.target_sheet_id = Some(sid_f);
    app_frozen.grid.import_wizard.raw_clipboard_text = String::from("Batch: B99\n10\t20");
    let mut drv_frozen = make_driver(app_frozen);
    drv_frozen.click_widget_id("import_paste.tab.notes").unwrap();
    drv_frozen.click_widget_id("import_paste.opt.extract_info_metadata").unwrap();
    drv_frozen.click_widget_id("import_paste.opt.add_floating_note").unwrap();
    drv_frozen.click_widget_id("modal.ok").unwrap();
    drv_frozen.run_settled();
    assert!(drv_frozen.state().app.grid.import_wizard.open, "wizard must stay open on commit failure");
    let info_f = drv_frozen.state().app.document().info_sheet(&info_f_id).unwrap();
    assert!(info_f.constants.is_empty(), "failed import must not leak constants into InfoSheet");
    assert!(info_f.notes_markdown.is_empty(), "failed import must not leak notes into InfoSheet");

    // 4. AUD-10-03: Unstack skips header row & rejects oversized Cartesian blowup without OOM.
    let mut app_unstack = UiApp::new();
    app_unstack.grid.import_wizard.open = true;
    app_unstack.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    app_unstack.grid.import_wizard.delimiter = ',';
    app_unstack.grid.import_wizard.header_rows = 1;
    app_unstack.grid.import_wizard.filter.unstack_group_column = Some(0);
    // 250 distinct groups x 250 rows in same group > 50,000 preview cell cap.
    let mut big = String::from("Grp,Val\n");
    for i in 0..250 {
        big.push_str(&format!("G{i},{i}\n"));
    }
    for j in 0..250 {
        big.push_str(&format!("G0,{j}\n"));
    }
    app_unstack.grid.import_wizard.raw_clipboard_text = big;
    let mut drv_un = make_driver(app_unstack);
    drv_un.click_widget_id("import_paste.tab.filter").unwrap();
    assert!(drv_un.state().views.import_wizard.unstack_error.is_some(), "oversized unstack must surface an inline error");
    assert!(!drv_un.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "modal.ok must disable when unstack fails budget");

    // AUD-10-03 edge case: single header-only row with `skip_header_row` must NOT treat header row as data.
    let mut app_un1 = UiApp::new();
    app_un1.grid.import_wizard.open = true;
    app_un1.grid.import_wizard.origin_kind = ImportOriginKind::Clipboard;
    app_un1.grid.import_wizard.delimiter = ',';
    app_un1.grid.import_wizard.header_rows = 1;
    app_un1.grid.import_wizard.filter.unstack_group_column = Some(0);
    app_un1.grid.import_wizard.raw_clipboard_text = String::from("Grp,Val");
    let mut drv_un1 = make_driver(app_un1);
    drv_un1.click_widget_id("import_paste.tab.filter").unwrap();
    assert!(drv_un1.state().views.import_wizard.unstack_error.is_none());
    drv_un1.click_widget_id("modal.ok").unwrap();
    drv_un1.run_settled();
    let ds_un1 = drv_un1.state().app.document().data_sheets.first().expect("created empty data sheet");
    for r in 0..ds_un1.archetype.row_count() {
        for c in 0..ds_un1.archetype.column_count() {
            let cell = ds_un1.archetype.data_cell(CellCoord::new(r, c, 0));
            assert!(cell.is_none_or(Cell::is_empty), "header-only 1-row unstack must not import header row as data: {cell:?}");
        }
    }

    // 5. AUD-10-05: InternalDataSheet import preserves XY X column + all replicate subcolumns.
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[0].cells[0][1] = Cell::number(11.0);
    xy.data_sets[0].cells[1][0] = Cell::number(20.0);
    xy.data_sets[0].cells[1][1] = Cell::number(21.0);
    let mut doc_xy = ProjectDocument::new("XY Rep").unwrap();
    let src_id = SheetId::try_new("xy-src").unwrap();
    let dst_id = SheetId::try_new("col-dst").unwrap();
    doc_xy.data_sheets.push(DataSheet::new(src_id.clone(), "Src XY", TableArchetype::Xy(xy)).unwrap());
    doc_xy.data_sheets.push(
        DataSheet::new(dst_id.clone(), "Dst Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).unwrap()))
            .unwrap(),
    );
    doc_xy.active_sheet_id = Some(dst_id.clone());
    let mut app_xy = UiApp::from_session(Session::from_document(doc_xy));
    app_xy.grid.import_wizard.open = true;
    app_xy.grid.import_wizard.origin_kind = ImportOriginKind::InternalDataSheet;
    app_xy.grid.import_wizard.is_live_link = false;
    app_xy.grid.import_wizard.source_sheet_id = Some(src_id);
    app_xy.grid.import_wizard.target_sheet_id = Some(dst_id.clone());
    let mut drv_xy = make_driver(app_xy);
    drv_xy.click_widget_id("modal.ok").unwrap();
    drv_xy.run_settled();
    let dst = drv_xy.state().app.document().data_sheet(&dst_id).unwrap();
    assert_eq!(dst.archetype.data_cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(1.0), "X value must be preserved");
    assert_eq!(dst.archetype.data_cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(10.0), "Y subcol 0 must be preserved");
    assert_eq!(dst.archetype.data_cell(CellCoord::new(0, 2, 0)).and_then(Cell::active_number), Some(11.0), "Y subcol 1 must be preserved");
}

#[test]
fn aud_10_04_42_48_and_aud_13_15_fit_modal_fixes() {
    let (mut app, sid) = seed_xy_app(&[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0], &[5.0, 12.0, 48.0, 88.0, 97.0, 99.0]);
    let out = app
        .execute_command(
            "analysis.run",
            json!({
                "source_sheet_id": sid.as_str(),
                "kind": "nonlinear_regression",
                "equation_id": "log_agonist_vs_response_4pl",
            }),
        )
        .unwrap();
    let rs_id = SheetId::try_new(out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &rs_id));

    // Seed structured comparison + fixed built-in parameter rule to test AUD-10-04 and AUD-10-42.
    if let Some(d) = app.canvas_dialogs.edit_params.draft.as_mut()
        && let AnalysisKind::NonlinearRegression(nl) = &mut d.kind
    {
        nl.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("straight_line") });
        nl.comparison_question = Some(ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: String::from("straight_line") });
        nl.comparison_method = Some(ModelComparisonMethod::ExtraSumOfSquaresF);
        nl.parameter_rules.push(ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::Constant(0.0),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        });
    }
    let mut driver = make_driver(app);

    // AUD-10-04: Compare tab method switching & None clearing all 3 fields.
    driver.click_widget_id("fit.tab.compare").unwrap();
    driver.click_widget_id("fit.cmp.aicc").unwrap();
    if let AnalysisKind::NonlinearRegression(nl) = &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert_eq!(nl.comparison_method, Some(ModelComparisonMethod::Aicc));
    }
    driver.click_widget_id("fit.cmp.none").unwrap();
    if let AnalysisKind::NonlinearRegression(nl) = &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert!(nl.comparison.is_none() && nl.comparison_question.is_none() && nl.comparison_method.is_none());
    }

    // AUD-10-42: Switching to Custom DSL clears stale built-in `parameter_rules`.
    driver.click_widget_id("fit.tab.model").unwrap();
    driver.click_widget_id("fit.eq.custom_toggle").unwrap();
    if let AnalysisKind::NonlinearRegression(nl) = &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert!(nl.parameter_rules.is_empty(), "toggling custom DSL must clear built-in parameter_rules");
    }
    driver.click_widget_id("fit.eq.mode.builtin").unwrap();

    // AUD-13-15: Output tab exposes `summary_parameter` pills when `create_summary_graph` is enabled.
    driver.click_widget_id("fit.tab.output").unwrap();
    let scroll_pos = driver.state().dialog_rect_pt.expect("dialog rect").center();
    for _ in 0..4 {
        if driver.frame_meta().find_by_widget_id("fit.out.create_summary_graph").is_some() {
            break;
        }
        driver.wheel_at(scroll_pos, Vec2::new(0.0, -180.0)).unwrap();
    }
    driver.click_widget_id("fit.out.create_summary_graph").unwrap();
    for _ in 0..3 {
        if driver.frame_meta().find_by_widget_id("fit.out.summary_param.LogEC50").is_some() {
            break;
        }
        driver.wheel_at(scroll_pos, Vec2::new(0.0, -120.0)).unwrap();
    }
    assert!(driver.frame_meta().find_by_widget_id("fit.out.summary_param.LogEC50").is_some(), "expected summary_parameter selector");
    driver.click_widget_id("fit.out.summary_param.LogEC50").unwrap();
    if let AnalysisKind::NonlinearRegression(nl) = &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert_eq!(nl.summary_parameter.as_deref(), Some("LogEC50"));
    }
}

#[test]
fn aud_10_06_07_23_24_51_analyze_picker_and_change_data_fixes() {
    // 1. AUD-10-07 + AUD-10-23 + AUD-10-24 on Column table.
    let (mut app_col, sid_col) =
        seed_column_app(&[("A", &[0.01, 0.04, 0.2]), ("B", &[0.02, 0.05, 0.3]), ("C", &[0.03, 0.06, 0.4])], ColumnDesignHint::UnpairedReplicates);
    let doc = app_col.document().clone();
    assert!(app_col.canvas_dialogs.open_analyze_dialog(&doc, &sid_col));
    let mut drv = make_driver(app_col);

    // AUD-10-07 (live path): Unchecking variable 0 when All (`selected_data_sets.is_empty()`) yields `[1, 2]`.
    drv.click_widget_id("analyze.mv.var.0").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1, 2]);

    // AUD-10-07 (fallback `analyze.dataset.*` branch when `variable_candidates` is empty): also yields `[1, 2]`, NOT `[0]`.
    let (mut app_fb, sid_fb) =
        seed_column_app(&[("A", &[0.01, 0.04, 0.2]), ("B", &[0.02, 0.05, 0.3]), ("C", &[0.03, 0.06, 0.4])], ColumnDesignHint::UnpairedReplicates);
    let doc_fb = app_fb.document().clone();
    assert!(app_fb.canvas_dialogs.open_analyze_dialog(&doc_fb, &sid_fb));
    app_fb.canvas_dialogs.analyze.variable_candidates.clear();
    let mut drv_fb = make_driver(app_fb);
    drv_fb.click_widget_id("analyze.dataset.0").unwrap();
    assert_eq!(drv_fb.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1, 2]);

    // AUD-10-23: Category filter chip filters entries and does NOT clobber `search_query`.
    set_textbox(&mut drv, "analyze.search", "t-test").unwrap();
    drv.click_widget_id("analyze.cat.column").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.analyze.category_filter, Some(AnalysisCategory::Column));
    assert_eq!(drv.state().app.canvas_dialogs.analyze.search_query, "t-test");
    drv.click_widget_id("analyze.cat.all").unwrap();
    drv.click_widget_id("analyze.search.clear").unwrap();

    // AUD-10-24: P-Value Stack dropdown trigger click does NOT cycle method; column cycles 0..3 -> None (All).
    set_textbox(&mut drv, "analyze.search", "stack").unwrap();
    drv.click_widget_id("analyze.entry.stats.pvalue_stack").unwrap();
    let method_before = drv.state().app.canvas_dialogs.analyze.pvalue_stack_method;
    drv.click_widget_id("analyze.pvalue_stack.method").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.analyze.pvalue_stack_method, method_before, "opening dropdown must not mutate method");
    drv.click_widget_id("analyze.pvalue_stack.method").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.analyze.pvalue_stack_method, method_before, "closing dropdown must not mutate method");
    for expected in [Some(0), Some(1), Some(2), None] {
        drv.click_widget_id("analyze.pvalue_stack.column").unwrap();
        assert_eq!(drv.state().app.canvas_dialogs.analyze.pvalue_stack_column, expected);
    }

    // 2. AUD-10-06: PCR predictor toggle syncs `pca_predictors` + `selected_data_sets` + disables matching outcome.
    let (mut app_mv, sid_mv) = seed_mv_app(4, 6);
    let doc_mv = app_mv.document().clone();
    assert!(app_mv.canvas_dialogs.open_analyze_dialog(&doc_mv, &sid_mv));
    let mut drv_mv = make_driver(app_mv);
    set_textbox(&mut drv_mv, "analyze.search", "PCR").unwrap();
    drv_mv.click_widget_id("analyze.entry.stats.pcr").unwrap();
    assert!(drv_mv.state().app.canvas_dialogs.analyze.pca_predictors.is_empty());
    drv_mv.click_widget_id("analyze.pca.pred.1").unwrap();
    assert_eq!(drv_mv.state().app.canvas_dialogs.analyze.pca_predictors, vec![1]);
    assert_eq!(drv_mv.state().app.canvas_dialogs.analyze.selected_data_sets, vec![1]);
    assert!(!drv_mv.frame_meta().find_by_widget_id("analyze.pcr.outcome.1").unwrap().enabled);
    assert!(drv_mv.frame_meta().find_by_widget_id("analyze.pcr.outcome.2").unwrap().enabled);
}

#[test]
fn aud_10_09_10_26_27_30_46_47_51_stats_params_fixes() {
    // 1. AUD-10-09 + AUD-10-30 + AUD-10-51: German locale comma decimal & invalid input disables OK + dynamic confidence label.
    let (mut app_lin, sid_lin) = seed_xy_app(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 6.0, 8.0, 10.0]);
    app_lin.set_locale("de");
    let out_lin = app_lin.execute_command("analysis.run", json!({ "source_sheet_id": sid_lin.as_str(), "kind": "linear_regression" })).unwrap();
    let res_lin_id = SheetId::try_new(out_lin.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc_lin = app_lin.document().clone();
    assert!(app_lin.canvas_dialogs.open_edit_parameters(&doc_lin, &res_lin_id));
    let mut drv_lin = make_driver(app_lin);
    drv_lin.click_widget_id("edit.lin.fixed_point").unwrap();
    set_textbox(&mut drv_lin, "edit.lin.fixed_point_x0", "1,5").unwrap();
    assert!(drv_lin.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    if let AnalysisKind::LinearRegression(l) = &drv_lin.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert_eq!(l.fixed_point, Some((1.5, 0.0)));
    }
    set_textbox(&mut drv_lin, "edit.lin.fixed_point_x0", "not_a_number").unwrap();
    assert!(!drv_lin.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "invalid numeric input must disable modal.ok");
    set_textbox(&mut drv_lin, "edit.lin.fixed_point_x0", "2,5").unwrap();
    assert!(drv_lin.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    drv_lin.click_widget_id("edit.tab.options").unwrap();
    drv_lin.click_widget_id("edit.conf.99").unwrap();
    drv_lin.click_widget_id("edit.tab.diagnostics").unwrap();
    assert!(
        drv_lin.frame_meta().find_by_widget_id("edit.lin.conf_bands").unwrap().label.contains("99%"),
        "confidence band checkbox must reflect active 99% level"
    );

    // AUD-10-09 Cancel -> Reopen regression: stale invalid numeric draft (`"-5"`) must NOT survive Cancel + Reopen on same sheet.
    drv_lin.click_widget_id("edit.tab.experimental_design").unwrap();
    drv_lin.click_widget_id("edit.lin.deming").unwrap();
    drv_lin.click_widget_id("edit.lin.deming.sds").unwrap();
    set_textbox(&mut drv_lin, "edit.lin.deming.sd_x", "-5").unwrap();
    assert!(!drv_lin.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "negative SD_X must disable modal.ok");
    drv_lin.click_widget_id("modal.cancel").unwrap();
    assert_eq!(drv_lin.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let doc_reopen = drv_lin.state().app.document().clone();
    assert!(drv_lin.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_reopen, &res_lin_id));
    sync_app_to_driver(&mut drv_lin);
    drv_lin.click_widget_id("edit.lin.deming").unwrap();
    drv_lin.click_widget_id("edit.lin.deming.sds").unwrap();
    assert!(
        drv_lin.state().views.stats_modals.numeric_field_errors.is_empty(),
        "reopened modal must not inherit stale numeric_field_errors from canceled session"
    );
    let sdx_val = drv_lin.frame_meta().find_by_widget_id("edit.lin.deming.sd_x").and_then(|e| e.value.clone());
    assert_ne!(sdx_val.as_ref().and_then(|v| v.as_str()), Some("-5"), "reopened SD_X must not resurrect canceled invalid draft");
    assert!(drv_lin.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "reopened modal must have modal.ok enabled");

    // 2. AUD-10-10: Cox PH role conflict clearing.
    let (mut app_cox, sid_cox) = seed_mv_app(4, 6);
    let cox_spec = AnalysisSpec::new(
        "res-cox",
        sid_cox,
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 1,
            status_column: 2,
            predictor_columns: vec![3],
            reference_levels: Vec::new(),
            tie_method: curvebook_doc::CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    app_cox.canvas_dialogs.edit_params.result_sheet_id = Some(cox_spec.id.clone());
    app_cox.canvas_dialogs.edit_params.draft = Some(cox_spec.clone());
    app_cox.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: cox_spec.id };
    let mut drv_cox = make_driver(app_cox);
    drv_cox.click_widget_id("edit.tab.data").unwrap();
    drv_cox.click_widget_id("edit.cox.time.3").unwrap();
    if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. }) =
        &drv_cox.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind
    {
        assert_eq!(*time_column, 3);
        assert!(!predictor_columns.contains(&3), "assigning Time must remove col from predictor_columns");
        assert_ne!(*time_column, *status_column);
    }
    // Assign Time to col 2 (currently status_column) -> must swap so time != status.
    drv_cox.click_widget_id("edit.cox.time.2").unwrap();
    if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, .. }) =
        &drv_cox.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind
    {
        assert_eq!(*time_column, 2);
        assert_ne!(*time_column, *status_column);
    }

    // 3. AUD-10-26: MultipleRowTTests disables Tukey/Dunnett.
    let (mut app_rt, sid_rt) = seed_column_app(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0])], ColumnDesignHint::UnpairedReplicates);
    let rt_spec = AnalysisSpec::new("res-rt", sid_rt, AnalysisKind::MultipleRowTTests(MultipleRowTestsSpec::default())).unwrap();
    app_rt.canvas_dialogs.edit_params.result_sheet_id = Some(rt_spec.id.clone());
    app_rt.canvas_dialogs.edit_params.draft = Some(rt_spec.clone());
    app_rt.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: rt_spec.id };
    let mut drv_rt = make_driver(app_rt);
    drv_rt.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    assert!(!drv_rt.frame_meta().find_by_widget_id("edit.post_hoc.tukey").unwrap().enabled);
    assert!(!drv_rt.frame_meta().find_by_widget_id("edit.post_hoc.dunnett").unwrap().enabled);

    // 4. AUD-10-27: Multiple Imputation max_iters clamps at 100.
    let (mut app_mi, sid_mi) = seed_mv_app(3, 6);
    let mi_spec = AnalysisSpec::new(
        "res-mi",
        sid_mi,
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 100,
            seed: 42,
            response_column: None,
            predictor_columns: Vec::new(),
        }),
    )
    .unwrap();
    app_mi.canvas_dialogs.edit_params.result_sheet_id = Some(mi_spec.id.clone());
    app_mi.canvas_dialogs.edit_params.draft = Some(mi_spec.clone());
    app_mi.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: mi_spec.id };
    let mut drv_mi = make_driver(app_mi);
    drv_mi.click_widget_id("edit.tab.experimental_design").unwrap();
    drv_mi.click_widget_id("edit.mi.iters.inc").unwrap();
    if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters, .. }) =
        &drv_mi.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind
    {
        assert_eq!(*max_iters, 100, "max_iters must clamp at engine limit 100");
    }
}

#[test]
fn aud_10_28_29_48_49_50_55_r9_defaults_spline_viewer_contingency() {
    // 5. AUD-10-28: RandomizeGroups 3-arm / 1:2 preset snaps block_sizes to 6 and steps by 3.
    let mut app_rg = UiApp::new();
    app_rg.canvas_dialogs.active_dialog = ActiveDialog::RandomizeGroups;
    let mut drv_rg = make_driver(app_rg);
    drv_rg.click_widget_id("rand_groups.sch.block").unwrap();
    drv_rg.click_widget_id("rand_groups.p3").unwrap();
    assert_eq!(drv_rg.state().app.canvas_dialogs.randomize_groups.block_sizes, vec![6]);
    drv_rg.click_widget_id("rand_groups.block.inc").unwrap();
    assert_eq!(drv_rg.state().app.canvas_dialogs.randomize_groups.block_sizes, vec![9]);
    drv_rg.click_widget_id("rand_groups.block.dec").unwrap();
    drv_rg.click_widget_id("rand_groups.block.dec").unwrap();
    assert_eq!(drv_rg.state().app.canvas_dialogs.randomize_groups.block_sizes, vec![3]);

    // 6. AUD-10-29: RandomSampleRows source switch clears stratify_column & clamps sample_size.
    let mut doc_rs = ProjectDocument::new("RS").unwrap();
    let s1 = SheetId::try_new("s1").unwrap();
    let s2 = SheetId::try_new("s2").unwrap();
    doc_rs.data_sheets.push(
        DataSheet::new(s1.clone(), "Big", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 4, 20).unwrap())).unwrap(),
    );
    doc_rs.data_sheets.push(
        DataSheet::new(s2.clone(), "Small", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap())).unwrap(),
    );
    let mut app_rs = UiApp::from_session(Session::from_document(doc_rs));
    app_rs.canvas_dialogs.random_sample_rows.source_sheet_id = Some(s1.clone());
    app_rs.canvas_dialogs.random_sample_rows.stratify_column = Some(3);
    app_rs.canvas_dialogs.random_sample_rows.proportional_allocation = true;
    app_rs.canvas_dialogs.random_sample_rows.sample_size = 15;
    app_rs.canvas_dialogs.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(s1) };
    let mut drv_rs = make_driver(app_rs);
    drv_rs.click_widget_id("rand_sample.src.1").unwrap();
    let rs_state = &drv_rs.state().app.canvas_dialogs.random_sample_rows;
    assert_eq!(rs_state.source_sheet_id, Some(s2));
    assert_eq!(rs_state.stratify_column, None);
    assert!(!rs_state.proportional_allocation);
    assert!(rs_state.sample_size <= 3);

    // 7. AUD-10-48: `saved_defaults` keys by sub-variant slug (no PCA vs CorrelationMatrix collision).
    let (mut app_def, sid_def) = seed_mv_app(3, 6);
    let pca_id = SheetId::try_new("res-pca").unwrap();
    app_def.execute_command("analysis.run", json!({ "sheet_id": "res-pca", "source_sheet_id": sid_def.as_str(), "kind": "pca" })).unwrap();
    let corr_id = SheetId::try_new("res-corr").unwrap();
    app_def
        .execute_command("analysis.run", json!({ "sheet_id": "res-corr", "source_sheet_id": sid_def.as_str(), "kind": "correlation_matrix" }))
        .unwrap();
    let doc_def = app_def.document().clone();
    assert!(app_def.canvas_dialogs.open_edit_parameters(&doc_def, &pca_id));
    let mut drv_def = make_driver(app_def);
    drv_def.click_widget_id("dialog.button.save_default").unwrap();
    drv_def.click_widget_id("modal.cancel").unwrap();
    let doc_def2 = drv_def.state().app.document().clone();
    assert!(drv_def.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_def2, &corr_id));
    sync_app_to_driver(&mut drv_def);
    drv_def.click_widget_id("dialog.button.save_default").unwrap();
    assert_eq!(drv_def.state().app.egui_views.stats_modals.saved_defaults.len(), 2, "PCA and CorrelationMatrix must not overwrite each other");

    // 8. AUD-10-49 + AUD-10-50 + AUD-10-55: SplineLowess checklist & smoothing_df preservation; Contingency variants; ResultViewer.
    let (mut app_sp, sid_sp) = seed_xy_app(&[0.0, 1.0, 2.0, 3.0, 4.0], &[0.0, 1.0, 4.0, 9.0, 16.0]);
    let sp_spec = AnalysisSpec::new(
        "res-sp",
        sid_sp,
        AnalysisKind::FitSplineLowess(SplineLowessSpec {
            method: SplineLowessMethod::SmoothingSpline,
            smoothing_df: Some(6.0),
            smoothing_knots: None,
            ..SplineLowessSpec::default()
        }),
    )
    .unwrap();
    app_sp.canvas_dialogs.edit_params.result_sheet_id = Some(sp_spec.id.clone());
    app_sp.canvas_dialogs.edit_params.draft = Some(sp_spec.clone());
    app_sp.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: sp_spec.id };
    let mut drv_sp = make_driver(app_sp);
    assert!(drv_sp.frame_meta().find_by_widget_id("edit.dataset.0").is_some(), "SplineLowess must render dataset checklist");
    drv_sp.click_widget_id("edit.spline.knots.inc").unwrap();
    if let AnalysisKind::FitSplineLowess(s) = &drv_sp.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert_eq!(s.smoothing_df, Some(7.0), "stepping in smoothing_df mode must preserve smoothing_df");
    }

    // AUD-10-50: ResultViewer clamps OOB tab_idx consistently and shows row labels / truncation notice.
    let mut doc_rv = ProjectDocument::new("RV").unwrap();
    let rs_rv_id = SheetId::try_new("rv-1").unwrap();
    let tab1 = ResultTab {
        id: String::from("t1"),
        title: String::from("Tab 1"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: (0..130).map(|i| format!("Row {i}")).collect(),
        rows: (0..130).map(|i| vec![ResultCell::Number { value: i as f64, units: None, prefix: None }]).collect(),
        footnotes: Vec::new(),
    };
    let tab2 = ResultTab {
        id: String::from("t2"),
        title: String::from("Tab 2"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("LastCol")],
        row_labels: vec![String::from("OnlyRow")],
        rows: vec![vec![ResultCell::Number { value: 99.0, units: None, prefix: None }]],
        footnotes: Vec::new(),
    };
    let mut rs_rv = ResultSheet::new(AnalysisSpec::new("rv-1", "src-rv", AnalysisKind::LinearRegression(Default::default())).unwrap()).unwrap();
    rs_rv.status = ResultStatus::UpToDate;
    rs_rv.tabs = vec![tab1, tab2];
    doc_rv.result_sheets.push(rs_rv);
    let mut app_rv = UiApp::from_session(Session::from_document(doc_rv));
    app_rv.canvas_dialogs.result_view.active_tab_index = 99;
    app_rv.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: rs_rv_id };
    let drv_rv = make_driver(app_rv);
    let tab1_meta = drv_rv.frame_meta().find_by_widget_id("result.tab.1").expect("tab 1 button");
    assert_eq!(tab1_meta.value.as_ref().and_then(|v| v.as_bool()), Some(true), "OOB tab_index must clamp highlight to last tab");

    // AUD-10-55: Contingency exposes G2, Barnard, FleissKappa, ExactBinomial, and KappaWeighting.
    let (mut app_cg, sid_cg) = seed_column_app(&[("C1", &[10.0, 20.0]), ("C2", &[15.0, 25.0])], ColumnDesignHint::UnpairedReplicates);
    let cg_spec = AnalysisSpec::new("res-cg", sid_cg, AnalysisKind::Contingency(ContingencySpec::default())).unwrap();
    app_cg.canvas_dialogs.edit_params.result_sheet_id = Some(cg_spec.id.clone());
    app_cg.canvas_dialogs.edit_params.draft = Some(cg_spec.clone());
    app_cg.canvas_dialogs.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: cg_spec.id };
    let mut drv_cg = make_driver(app_cg);
    for wid in ["edit.cont.g2", "edit.cont.barnard", "edit.cont.fleiss_kappa", "edit.cont.exact_binomial"] {
        assert!(drv_cg.frame_meta().find_by_widget_id(wid).is_some(), "missing contingency variant {wid}");
    }
    drv_cg.click_widget_id("edit.cont.kappa").unwrap();
    drv_cg.click_widget_id("edit.cont.kappa.linear").unwrap();
    if let AnalysisKind::Contingency(c) = &drv_cg.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind {
        assert_eq!(c.method, ContingencyMethod::CohenKappa { weighting: KappaWeighting::Linear });
    }
}

#[test]
fn aud_10_14_16_45_transform_modals_fixes() {
    // 1. AUD-10-14: Affine LinearScale { slope: 2.5, intercept: 10.0 } is preserved when editing TransformValues.
    let (mut app, sid) = seed_xy_app(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]);
    let kind = AnalysisKind::Transform(TransformAnalysisSpec::Math {
        x_transform: None,
        y_transform: Some(MathTransformPreset::LinearScale { slope: 2.5, intercept: 10.0 }),
        error_propagation: ErrorPropagationMode::ReplicateEndpoints,
    });
    let out = app.execute_command("analysis.run", json!({ "source_sheet_id": sid.as_str(), "kind": serde_json::to_value(&kind).unwrap() })).unwrap();
    let rs_id = SheetId::try_new(out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc, &rs_id));
    let mut driver = make_driver(app);
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let rs = driver.state().app.document().result_sheet(&rs_id).unwrap();
    let AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform: Some(MathTransformPreset::LinearScale { slope, intercept }), .. }) =
        &rs.analysis.kind
    else {
        panic!("expected preserved LinearScale");
    };
    assert!((slope - 2.5).abs() < 1e-12, "slope must not be destroyed by ConstantKOp::infer_from_preset");
    assert!((intercept - 10.0).abs() < 1e-12);

    // 2. AUD-10-16: Comma-decimal locale formats & parses Simulate parameters cleanly without disabling OK.
    let (mut app_de, sid_de) = seed_xy_app(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]);
    app_de.set_locale("de");
    let sim_kind = AnalysisKind::Transform(TransformAnalysisSpec::Simulate {
        seed: 42,
        row_count: 10,
        replicates: 2,
        x_min: -9.0,
        x_max: -3.0,
        equation_id: String::from("log_agonist_vs_response_4pl"),
        parameters: vec![(String::from("Top"), 100.5), (String::from("HillSlope"), 1.5)],
        noise: curvebook_doc::SimulationNoise::None,
    });
    let out_sim = app_de
        .execute_command("analysis.run", json!({ "source_sheet_id": sid_de.as_str(), "kind": serde_json::to_value(&sim_kind).unwrap() }))
        .unwrap();
    let sim_id = SheetId::try_new(out_sim.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc_de = app_de.document().clone();
    assert!(app_de.canvas_dialogs.open_edit_parameters(&doc_de, &sim_id));
    let drv_de = make_driver(app_de);
    assert!(drv_de.state().views.transform_modals.sim_params_draft.contains(';'), "comma-decimal locale must join params with ';'");
    assert!(drv_de.state().views.transform_modals.validation_error.is_none(), "comma-decimal params must validate cleanly");
    assert!(drv_de.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);

    // 3. AUD-10-45: Standalone RemoveBaseline Cycle Operation cycles `baseline_op`.
    let (mut app_rb, sid_rb) = seed_column_app(&[("Ctrl", &[1.0, 2.0]), ("Tx", &[3.0, 4.0])], ColumnDesignHint::UnpairedReplicates);
    app_rb.egui_views.transform_modals.open_kind = Some(TransformModalKind::RemoveBaseline);
    app_rb.egui_views.transform_modals.target_sheet_id = Some(sid_rb);
    let mut drv_rb = make_driver(app_rb);
    let before_op = drv_rb.state().views.transform_modals.baseline_op;
    drv_rb.click_widget_id("edit.transform.baseline_operation").unwrap();
    assert_eq!(drv_rb.state().views.transform_modals.baseline_op, (before_op + 1) % 4);

    // 4. AUD-10-45: ExcludeWithReason preset radio updates `exclude_reason_draft` without 1-frame overwrite lag.
    let (mut app_ex, sid_ex) = seed_column_app(&[("Ctrl", &[1.0, 2.0])], ColumnDesignHint::UnpairedReplicates);
    app_ex.canvas_dialogs.exclude_with_reason.sheet_id = Some(sid_ex.clone());
    app_ex.canvas_dialogs.exclude_with_reason.coords = vec![CellCoord::new(0, 0, 0).into()];
    app_ex.canvas_dialogs.exclude_with_reason.reason_tag.clear();
    app_ex.canvas_dialogs.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id: sid_ex };
    let mut drv_ex = make_driver(app_ex);
    assert!(!drv_ex.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    drv_ex.click_widget_id("exclude.preset.0").unwrap();
    assert!(!drv_ex.state().views.transform_modals.exclude_reason_draft.is_empty(), "preset click must update draft immediately");
    assert!(drv_ex.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled, "preset click must enable modal.ok");
}

#[test]
fn aud_10_15_17_18_22_43_44_app_anova_manage_vars_and_atomic_commits() {
    // 1. AUD-10-15: Preferences shortcut capture produces `Cmd+K` / `Ctrl+K`, valid in `normalize_shortcut_combo`.
    let mut app_pref = UiApp::new();
    app_pref.egui_views.app_modals.open_preferences(&app_pref.shortcuts, Some(PreferencesCategory::KeyboardShortcuts));
    let first_cmd = app_pref.shortcuts.bindings().next().unwrap().command_id.clone();
    app_pref.egui_views.app_modals.preferences.selected_shortcut_command = Some(first_cmd);
    app_pref.egui_views.app_modals.preferences.shortcut_capture_armed = true;
    let mut drv_pref = make_driver(app_pref);
    drv_pref.press_egui_key(Key::K, Modifiers::COMMAND);
    drv_pref.run_settled();
    let captured = drv_pref.state().views.app_modals.preferences.shortcut_combo_draft.clone();
    assert!(!captured.contains("CmdOrCtrl"), "captured combo must not contain rejected CmdOrCtrl token: {captured}");
    assert!(normalize_shortcut_combo(&captured).is_ok(), "captured combo `{captured}` must normalize cleanly");

    // 2. AUD-10-44: About & Feedback copy buttons route through injected `StubPlatformClipboardAdapter`.
    let stub = Arc::new(StubPlatformClipboardAdapter::default());
    let mut app_clip = UiApp::new();
    app_clip.human_path_only = true;
    app_clip.set_clipboard_adapter(stub.clone());
    app_clip.canvas_dialogs.active_dialog = ActiveDialog::About;
    let mut drv_clip = make_driver(app_clip);
    drv_clip.click_widget_id("dialog.about.copy_version").unwrap();
    let copied_about = stub.last_text().expect("About copy must write through PlatformClipboardAdapter");
    assert!(copied_about.contains("CurveBook"), "expected diagnostic summary in stub clipboard, got: {copied_about}");
    stub.clear();
    drv_clip.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    sync_app_to_driver(&mut drv_clip);
    drv_clip.click_widget_id("dialog.feedback.copy_markdown").unwrap();
    let copied_fb = stub.last_text().expect("Feedback copy must write through PlatformClipboardAdapter");
    assert!(copied_fb.contains("CurveBook"), "expected feedback markdown in stub clipboard, got: {copied_fb}");

    // 3. AUD-10-17: One-Way ANOVA `edit.compare_against_control` is disabled in REML and nonparametric modes.
    let (mut app_av, sid_av) =
        seed_column_app(&[("A", &[1.0, 2.0, 3.0]), ("B", &[4.0, 5.0, 6.0]), ("C", &[7.0, 8.0, 9.0])], ColumnDesignHint::PairedRepeatedMeasures);
    let out_av = app_av.execute_command("analysis.run", json!({ "source_sheet_id": sid_av.as_str(), "kind": "one_way_anova" })).unwrap();
    let av_id = SheetId::try_new(out_av.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc_av = app_av.document().clone();
    assert!(app_av.canvas_dialogs.open_edit_parameters(&doc_av, &av_id));
    if let Some(d) = app_av.canvas_dialogs.edit_params.draft.as_mut()
        && let AnalysisKind::OneWayAnova(a) = &mut d.kind
    {
        a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: true };
        a.mixed_effects_reml = true;
    }
    app_av.canvas_dialogs.edit_params.paired_or_matched = true;
    let mut drv_av = make_driver(app_av);
    drv_av.click_widget_id("edit.tab.multiple_comparisons").unwrap();
    assert!(
        !drv_av.frame_meta().find_by_widget_id("edit.compare_against_control").unwrap().enabled,
        "REML must disable edit.compare_against_control"
    );

    // 4. AUD-10-18 + AUD-10-43: Manage Variables reverts live edits even when undo stack is saturated at 64,
    //    disables frozen controls, and stages multi-column title/unit edits across column switches.
    let (mut app_mv, sid_mv) = seed_mv_app(3, 4);
    for i in 0..70usize {
        app_mv.execute_command("table.setTitles", json!({ "sheet_id": sid_mv.as_str(), "col": 2, "title": format!("Seed{i}") })).unwrap();
    }
    assert_eq!(app_mv.session().undo_depth(), 64, "undo stack should be saturated at MAX_UNDO_ENTRIES = 64");
    app_mv.grid.variables_dialog.open = true;
    app_mv.grid.variables_dialog.selected_column = Some(1);
    let mut drv_mv = make_driver(app_mv);
    // Perform a live edit (role change) inside Manage Variables while undo_depth is saturated at 64.
    drv_mv.click_widget_id("mv_formula.vars.role.categorical").unwrap();
    drv_mv.click_widget_id("modal.cancel").unwrap();
    let mv_after_cancel = match &drv_mv.state().app.document().data_sheet(&sid_mv).unwrap().archetype {
        TableArchetype::MultipleVariables(m) => m.variables[1].role,
        _ => panic!("expected MV"),
    };
    assert_eq!(mv_after_cancel, VariableRole::Continuous, "Cancel must revert live edits even when undo_depth started at 64");

    // Stage edits across two columns and commit atomically on OK.
    drv_mv.state_mut().app.grid.variables_dialog.open = true;
    drv_mv.state_mut().app.grid.variables_dialog.selected_column = Some(0);
    sync_app_to_driver(&mut drv_mv);
    set_textbox(&mut drv_mv, "mv_formula.vars.title_input", "ArmRenamed").unwrap();
    drv_mv.click_widget_id("mv_formula.vars.col.1").unwrap();
    set_textbox(&mut drv_mv, "mv_formula.vars.title_input", "DoseRenamed").unwrap();
    drv_mv.click_widget_id("modal.ok").unwrap();
    let mv_after_ok = match &drv_mv.state().app.document().data_sheet(&sid_mv).unwrap().archetype {
        TableArchetype::MultipleVariables(m) => (m.variables[0].title.clone(), m.variables[1].title.clone()),
        _ => panic!("expected MV"),
    };
    assert_eq!(mv_after_ok.0, "ArmRenamed");
    assert_eq!(mv_after_ok.1, "DoseRenamed");

    // Frozen MV sheet disables reverse order & textboxes.
    drv_mv.state_mut().app.execute_command("sheet.freeze", json!({ "sheet_id": sid_mv.as_str() })).unwrap();
    drv_mv.state_mut().app.grid.variables_dialog.open = true;
    sync_app_to_driver(&mut drv_mv);
    assert!(!drv_mv.frame_meta().find_by_widget_id("mv_formula.vars.title_input").unwrap().enabled);
    assert!(!drv_mv.frame_meta().find_by_widget_id("mv_formula.vars.order.reversed").unwrap().enabled);

    // 5. AUD-10-22: Failed ImportProjectMode keeps modal open instead of closing prematurely.
    let mut app_ipm = UiApp::new();
    app_ipm.canvas_dialogs.active_dialog = ActiveDialog::ImportProjectMode {
        command_id: String::from("table.importPrism"),
        path: String::from("/nonexistent/definitely_missing_12345.prism"),
        has_unsaved_changes: false,
    };
    let mut drv_ipm = make_driver(app_ipm);
    drv_ipm.click_widget_id("import_project_mode.merge").unwrap();
    assert!(matches!(drv_ipm.state().app.canvas_dialogs.active_dialog, ActiveDialog::ImportProjectMode { .. }), "failed import must keep modal open");

    // 6. AUD-10-22: Failing ExportFigure via `DialogButtonAction` keeps modal open & preserves `export_emf_selected`;
    //    succeeding layout EMF export closes modal & clears `export_emf_selected`.
    let mut app_exp = UiApp::new();
    let ls_out = app_exp.execute_command("layout.create", json!({ "sheet_id": "ls-rem", "title": "Rem Layout" })).unwrap();
    let ls_id = SheetId::try_new(ls_out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let missing_id = SheetId::try_new("gs-missing").unwrap();
    app_exp.canvas_dialogs.open_export_figure(&missing_id);
    app_exp.egui_views.app_modals.export_emf_selected = true;
    let mut fail_intents = curvebook_ui_egui::egui_shell::PendingUiIntents::new();
    fail_intents.push(curvebook_ui_egui::egui_shell::UiIntent::DialogButtonAction { action: String::from("ok") });
    assert!(fail_intents.commit_if_settled(&mut app_exp, false).is_err(), "exporting nonexistent sheet must fail");
    assert!(matches!(app_exp.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }), "failed export must keep ExportFigure modal open");
    assert!(app_exp.egui_views.app_modals.export_emf_selected, "failed export must preserve export_emf_selected");

    app_exp.canvas_dialogs.open_export_figure(&ls_id);
    let mut ok_intents = curvebook_ui_egui::egui_shell::PendingUiIntents::new();
    ok_intents.push(curvebook_ui_egui::egui_shell::UiIntent::DialogButtonAction { action: String::from("ok") });
    assert!(ok_intents.commit_if_settled(&mut app_exp, false).is_ok(), "layout EMF export via DialogButtonAction must succeed");
    assert_eq!(app_exp.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(!app_exp.egui_views.app_modals.export_emf_selected, "successful export must reset export_emf_selected");
}

#[test]
fn aud_10_13_31_40_and_aud_13_16_17_graph_modals_end_to_end() {
    // 1. AUD-10-13: FormatAxes with enable_y2 + custom frame_preset preserves frame_preset atomically.
    let (mut app, sid) = seed_xy_app(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.0, 0.0, 1.0, 1.0, 1.0]);
    let g_out = app.execute_command("graph.create", json!({ "data_sheet_id": sid.as_str(), "kind": "xy" })).unwrap();
    let gid = SheetId::try_new(g_out.payload["sheet_id"].as_str().unwrap()).unwrap();

    // Run SimpleLogisticRegression & Deming Model II to test AUD-13-16 and AUD-13-17 overlays in FormatGraph!
    let sl_kind = serde_json::to_value(AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
    let _ = app
        .execute_command(
            "analysis.run",
            json!({ "sheet_id": "res-sl", "title": "Simple Logistic Fit", "source_sheet_id": sid.as_str(), "kind": sl_kind }),
        )
        .unwrap();
    let dem_kind = serde_json::to_value(AnalysisKind::LinearRegression(LinearRegressionSpec {
        deming_model_ii: true,
        deming_error: DemingErrorSpec::EqualError,
        ..LinearRegressionSpec::default()
    }))
    .unwrap();
    let _ = app
        .execute_command("analysis.run", json!({ "sheet_id": "res-dem", "title": "Deming Fit", "source_sheet_id": sid.as_str(), "kind": dem_kind }))
        .unwrap();

    let gs_init = app.document().graph_sheet(&gid).unwrap().clone();
    app.canvas_dialogs.format_axes.sync_from_graph(&gs_init, AxisTarget::Y2);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gid.clone() };
    app.canvas_dialogs.format_axes.enable_y2 = true;
    let mut drv = make_driver(app);
    drv.click_widget_id("edit.tab.axes.frame_origin").unwrap();
    drv.click_widget_id("axes.frame.plain_boxed").unwrap();
    let undo_before = drv.state().app.session().undo_depth();
    drv.click_widget_id("modal.ok").unwrap();
    drv.run_settled();
    assert_eq!(drv.state().app.session().undo_depth(), undo_before + 1, "FormatAxes commit must be a single undo step");
    let gs = drv.state().app.document().graph_sheet(&gid).unwrap();
    assert_eq!(gs.spec.frame_preset, FramePreset::PlainBoxed, "user-chosen PlainBoxed must not be overwritten when enable_y2 is true");

    // 2. AUD-13-16 & AUD-13-17: FormatGraph offers SimpleLogisticRegression AND Deming overlays.
    let doc2 = drv.state().app.document().clone();
    assert!(drv.state_mut().app.canvas_dialogs.open_format_graph(&doc2, &gid));
    sync_app_to_driver(&mut drv);
    let overlay_labels: Vec<String> =
        drv.frame_meta().entries.iter().filter(|n| n.widget_id.starts_with("format_graph.overlay.attach.")).map(|n| n.label.clone()).collect();
    assert!(
        overlay_labels.iter().any(|l| l.contains("Simple Logistic")),
        "AUD-13-16: FormatGraph must list SimpleLogisticRegression fitted overlay; got {overlay_labels:?}"
    );
    assert!(
        overlay_labels.iter().any(|l| l.contains("Deming")),
        "AUD-13-17: FormatGraph must list Deming Model II fitted overlay; got {overlay_labels:?}"
    );
    drv.click_widget_id("modal.cancel").unwrap();

    // 3. AUD-10-31: Heatmap FormatGraph OK without edits does not fail on empty series or emit spurious graph.setAxes.
    let (mut app_hm, sid_hm) = seed_mv_app(4, 6);
    let corr_out = app_hm.execute_command("analysis.run", json!({ "source_sheet_id": sid_hm.as_str(), "kind": "correlation_matrix" })).unwrap();
    let corr_id = SheetId::try_new(corr_out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let hm_out = app_hm
        .execute_command(
            "graph.create",
            json!({ "data_sheet_id": corr_id.as_str(), "kind": GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap), "initialized": true }),
        )
        .unwrap();
    let hm_gid = SheetId::try_new(hm_out.payload["sheet_id"].as_str().unwrap()).unwrap();
    let doc_hm = app_hm.document().clone();
    assert!(app_hm.canvas_dialogs.open_format_graph(&doc_hm, &hm_gid));
    let mut drv_hm = make_driver(app_hm);
    let undo_hm_before = drv_hm.state().app.session().undo_depth();
    drv_hm.click_widget_id("modal.ok").unwrap();
    drv_hm.run_settled();
    assert_eq!(drv_hm.state().last_intent_error, None, "Heatmap FormatGraph OK must succeed cleanly");
    assert_eq!(drv_hm.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(drv_hm.state().app.session().undo_depth() <= undo_hm_before + 1);
}

#[test]
fn wave3_aud10_22_and_aud09_12_export_failure_and_contingency_ci_alignment() {
    use curvebook_doc::ContingencyTable;
    use curvebook_ui_egui::egui_shell::{PendingUiIntents, UiIntent};

    // 1. AUD-10-22 (headless PendingUiIntents seam): `[EngineCommand(err), CloseAllModals]` keeps
    //    `ExportFigure` open and preserves `export_emf_selected` on failure.
    let mut app_exp = UiApp::new();
    app_exp.canvas_dialogs.active_dialog = ActiveDialog::ExportFigure { sheet_id: SheetId::new("missing-graph") };
    app_exp.egui_views.app_modals.export_emf_selected = true;
    let mut intents = PendingUiIntents::new();
    intents.push_engine("graph.exportEmf", json!({ "sheet_id": "missing-graph" }));
    intents.push(UiIntent::CloseAllModals);
    let batch = intents.commit_all_if_settled(&mut app_exp, false);
    assert!(batch.first_error().is_some(), "exporting missing graph must fail");
    assert!(
        matches!(app_exp.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }),
        "AUD-10-22: failed ExportFigure EngineCommand must skip trailing CloseAllModals"
    );
    assert!(app_exp.egui_views.app_modals.export_emf_selected, "AUD-10-22: failed export must preserve export_emf_selected");

    // 2. AUD-09-12 seam: cycling Contingency OR CI to engine-unsupported `Woolf` surfaces inline
    //    `edit.contingency.ci.error`, disables `modal.ok` / `modal.apply`, and rejects direct `ok`
    //    with the explicit unsupported CI message; cycling back re-enables commit cleanly.
    let mut doc = ProjectDocument::new("Contingency Seam").unwrap();
    let mut ct = ContingencyTable::new(2, 2).unwrap();
    for (r, (a, b)) in [(12.0, 4.0), (5.0, 14.0)].into_iter().enumerate() {
        ct.data_sets[0].cells[r][0] = Cell::number(a);
        ct.data_sets[1].cells[r][0] = Cell::number(b);
    }
    doc.add_data_sheet(DataSheet::new("cont-1", "2x2", TableArchetype::Contingency(ct)).unwrap()).unwrap();
    let cont_spec = AnalysisSpec::new(
        "res-cont",
        "cont-1",
        AnalysisKind::Contingency(ContingencySpec { compute_odds_and_risk_ratios: true, ..Default::default() }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(cont_spec).unwrap()).unwrap();

    let mut app = UiApp::from_session(Session::from_document(doc));
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &SheetId::new("res-cont")));
    let mut drv = make_driver(app);
    assert!(drv.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    assert!(drv.frame_meta().find_by_widget_id("edit.contingency.ci.error").is_none());

    // Toggle OR CI -> Woolf: modal.ok & modal.apply disable, inline error renders.
    drv.click_widget_id("edit.contingency.ci.baptista_pike").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.edit_params.contingency_or_ci_method, "Woolf");
    let ok_node = drv.frame_meta().find_by_widget_id("modal.ok").unwrap();
    assert!(!ok_node.enabled, "AUD-09-12: modal.ok must disable when unsupported Woolf CI is selected");
    assert!(
        ok_node.disabled_reason.as_deref().is_some_and(|r| r.contains("Woolf")),
        "modal.ok disabled_reason must explain unsupported Woolf CI: {:?}",
        ok_node.disabled_reason
    );
    assert!(
        drv.frame_meta().find_by_widget_id("edit.contingency.ci.error").is_some(),
        "AUD-09-12: inline edit.contingency.ci.error alert must render"
    );

    // Direct programmatic ok attempt must fail with explicit unsupported CI error (not generic missing draft).
    let mut ok_try = PendingUiIntents::new();
    ok_try.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    let err = ok_try.commit_if_settled(&mut drv.state_mut().app, false).expect_err("Woolf commit must fail");
    assert!(err.to_string().contains("Woolf"), "unexpected commit error: {err}");

    // Cycle back to Baptista-Pike -> modal.ok re-enables and inline alert disappears.
    drv.click_widget_id("edit.contingency.ci.baptista_pike").unwrap();
    assert_eq!(drv.state().app.canvas_dialogs.edit_params.contingency_or_ci_method, "Baptista-Pike");
    assert!(drv.frame_meta().find_by_widget_id("modal.ok").unwrap().enabled);
    assert!(drv.frame_meta().find_by_widget_id("edit.contingency.ci.error").is_none());
}
