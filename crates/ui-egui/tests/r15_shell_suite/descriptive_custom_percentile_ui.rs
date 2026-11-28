#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! UI tests for `DescriptiveSpec::custom_upper_percentile` in the `egui_shell` Descriptive
//! Statistics modal and `AnalyzeDialogState` wiring.

use super::common;

use common::{ShellTestState, make_shell_driver_with_state, seed_column_doc, set_textbox};
use curvebook_doc::{AnalysisKind, ColumnDesignHint, DescriptiveSpec, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::AnalyzeDialogState;
use curvebook_ui_egui::egui_shell::Tokens;
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

#[test]
fn descriptive_modal_custom_upper_percentile_round_trip() {
    let (doc, _) = seed_column_doc(
        "Descriptive Custom Percentile UI Test",
        "col-desc",
        "Descriptive Data",
        ColumnDesignHint::UnpairedReplicates,
        &[("Assay", &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0])],
    )
    .unwrap();
    let mut session = Session::from_document(doc);
    let kind_val = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-desc", "kind": kind_val })).unwrap();
    let res_id = SheetId::try_new("res-desc").unwrap();

    let mut app = UiApp::from_session(session);
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::default(), tokens);

    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct_toggle").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());

    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());

    for invalid in ["40", "100"] {
        set_textbox(&mut driver, "edit.desc.custom_pct", invalid).unwrap();
    }
    assert!(matches!(
        &driver.state().app.canvas_dialogs.edit_params.draft.as_ref().unwrap().kind,
        AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile == Some(90.0)
    ));

    set_textbox(&mut driver, "edit.desc.custom_pct", "95").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        assert!(matches!(&rs.analysis.kind, AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile == Some(95.0)));
        let tab = rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
        for lbl in [
            "5% Percentile (Type 6)",
            "95% Percentile (Type 6)",
            "95% CI of Geometric mean",
            "95% CI of Harmonic mean",
            "95% CI of Quadratic mean (RMS)",
        ] {
            assert!(tab.row_labels.iter().any(|l| l == lbl), "missing `{lbl}`");
        }
    }

    let doc2 = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc2, &res_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());
    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
    assert!(matches!(&rs.analysis.kind, AnalysisKind::DescriptiveStats(d) if d.custom_upper_percentile.is_none()));
    assert!(!rs.tabs.iter().find(|t| t.id == "desc-summary").unwrap().row_labels.iter().any(|l| l.contains("Type 6")));
}

#[test]
fn analyze_dialog_wires_custom_upper_percentile_into_analysis_run() {
    let (doc, sid) =
        seed_column_doc("Run", "col-run", "Run Data", ColumnDesignHint::UnpairedReplicates, &[("A", &[1.0, 2.0, 3.0, 4.0, 5.0])]).unwrap();
    let mut dlg = AnalyzeDialogState::default();
    dlg.open_for_data_sheet(doc.data_sheet(&sid).unwrap());
    dlg.select_catalog_entry("stats.descriptive");
    dlg.descriptive_custom_upper_percentile = Some(97.5);

    let intent = dlg.build_create_new_intent().expect("analysis.run intent");
    assert_eq!(intent.command_id, "analysis.run");
    assert_eq!(intent.params.pointer("/kind/DescriptiveStats/custom_upper_percentile").and_then(serde_json::Value::as_f64), Some(97.5));
}

#[test]
fn descriptive_modal_visibility_precision_and_save_as_default_round_trip() {
    use curvebook_doc::ResultCell;
    use curvebook_ui_egui::egui_shell::results_view::format_result_cell_display_text;
    use curvebook_ui_egui::{extract_descriptive_default_from_preferences_extra, write_descriptive_default_to_preferences_extra};

    let (doc, sid) = seed_column_doc(
        "Descriptive Parity UI Test",
        "col-desc-vis",
        "Descriptive Data",
        ColumnDesignHint::UnpairedReplicates,
        &[("Assay", &[2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0])],
    )
    .unwrap();
    let mut session = Session::from_document(doc);
    let kind_val = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-desc-vis", "source_sheet_id": "col-desc-vis", "kind": kind_val })).unwrap();
    let res_id = SheetId::try_new("res-desc-vis").unwrap();

    let mut app = UiApp::from_session(session);
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::default(), tokens);

    // Toggle off Harmonic mean & Sum on Parameters tab, then switch to Options tab for precision & Save as Default.
    driver.click_widget_id("edit.desc.metric.harmonic_mean").unwrap();
    driver.click_widget_id("edit.desc.metric.sum").unwrap();
    driver.click_widget_id("edit.tab.options").unwrap();
    driver.run_settled();
    driver.click_widget_id("edit.desc.sig_figs.inc").unwrap();
    driver.click_widget_id("edit.desc.save_as_default").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.sig_figs.auto").is_some());

    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        let AnalysisKind::DescriptiveStats(d) = &rs.analysis.kind else {
            panic!("expected DescriptiveStats");
        };
        assert!(!d.visible_metrics.harmonic_mean && !d.visible_metrics.sum && d.visible_metrics.mean_sd_sem);
        assert_eq!(d.significant_digits, Some(5));
        let tab = rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
        assert!(!tab.row_labels.iter().any(|l| l == "Harmonic mean" || l == "95% CI of Harmonic mean" || l == "Sum"));
        assert!(tab.row_labels.iter().any(|l| l == "Mean"));
        let formatted = format_result_cell_display_text(&ResultCell::Number { value: 12.34567, prefix: None, units: None }, rs, tab);
        assert_eq!(formatted, "12.346");

        // Saved default is persisted on UiApp (with save_as_default cleared) and seeds the next Analyze dialog.
        let saved = driver.state().app.saved_descriptive_default().expect("saved descriptive default").clone();
        assert!(!saved.save_as_default);
        assert!(!saved.visible_metrics.harmonic_mean && !saved.visible_metrics.sum);
        assert_eq!(saved.significant_digits, Some(5));

        let mut extra = serde_json::Map::new();
        write_descriptive_default_to_preferences_extra(&mut extra, Some(&saved));
        assert_eq!(extract_descriptive_default_from_preferences_extra(&extra), Ok(Some(saved.clone())));

        // Invalid preferences payloads are rejected safely.
        let mut bad_extra = serde_json::Map::new();
        bad_extra.insert(String::from("descriptive_stats_default"), json!({ "significant_digits": 0 }));
        assert!(extract_descriptive_default_from_preferences_extra(&bad_extra).is_err());
    }

    // Re-opening Analyze Data... inherits the saved DescriptiveStats defaults automatically.
    let doc_after = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_analyze_dialog(&doc_after, &sid));
    driver.state_mut().app.canvas_dialogs.analyze.select_catalog_entry("stats.descriptive");
    let intent = driver.state().app.canvas_dialogs.analyze.build_create_new_intent().expect("seeded intent");
    assert_eq!(intent.params.pointer("/kind/DescriptiveStats/significant_digits").and_then(serde_json::Value::as_u64), Some(5));
    assert_eq!(intent.params.pointer("/kind/DescriptiveStats/visible_metrics/harmonic_mean").and_then(serde_json::Value::as_bool), Some(false));
    assert_eq!(intent.params.pointer("/kind/DescriptiveStats/visible_metrics/sum").and_then(serde_json::Value::as_bool), Some(false));
}

#[test]
fn analyze_modal_surfaces_log_x_scale_warning_and_very_wide_ci_formatting() {
    use curvebook_doc::{
        AnalysisSpec, Cell, CellCoord, DataSheet, NonlinearFitSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
        SubcolumnMode, TableArchetype, XyTable,
    };
    use curvebook_ui_egui::AssumptionDiagnosticBanner;
    use curvebook_ui_egui::egui_shell::results_view::format_result_cell_display_text;

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy.x_title = String::from("Concentration (M)");
    xy.data_sets[0].title = String::from("Drug");
    let mut sheet = DataSheet::new("xy-molar", "Raw Molar Dose", TableArchetype::Xy(xy)).unwrap();
    for (r, (&x, &y)) in [1e-9, 1e-8, 1e-7, 1e-6, 1e-5].iter().zip([5.0, 18.0, 52.0, 86.0, 96.0].iter()).enumerate() {
        sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
        sheet.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).unwrap();
    }
    let sid = sheet.id.clone();
    let mut doc = ProjectDocument::new("Raw Molar UI Test").unwrap();
    doc.data_sheets.push(sheet);
    doc.active_sheet_id = Some(sid.clone());

    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &sid));
    // Default equation auto-swaps to non-log on raw molar X.
    assert_eq!(app.canvas_dialogs.analyze.equation_id, "agonist_vs_response_4pl");
    // Explicitly selecting a log_* model triggers the X-scale mismatch banner & one-click switch in Analyze Data...
    app.canvas_dialogs.analyze.select_catalog_entry("fit.nonlinear");
    app.canvas_dialogs.analyze.equation_id = String::from("log_agonist_vs_response_4pl");

    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let mut driver = make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::default(), tokens);
    driver.run_settled();

    assert!(driver.frame_meta().find_by_widget_id("analyze.warning.log_x_scale").is_some(), "expected X-scale mismatch warning banner");
    assert!(driver.frame_meta().find_by_widget_id("analyze.warning.switch_non_log").is_some(), "expected one-click switch button");
    driver.click_widget_id("analyze.warning.switch_non_log").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.analyze.equation_id, "agonist_vs_response_4pl");
    assert!(driver.frame_meta().find_by_widget_id("analyze.warning.log_x_scale").is_none());

    // Verify `"Very wide"` CI cells format cleanly and trigger the nonlinear diagnostic banner.
    let mut nl_rs =
        ResultSheet::new(AnalysisSpec::new("res-vw", "xy-molar", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap())
            .unwrap();
    nl_rs.status = ResultStatus::UpToDate;
    let tab = ResultTab {
        id: String::from("nonlinear_parameters"),
        title: String::from("Best-fit values & confidence intervals"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![String::from("Best-fit value"), String::from("Confidence Interval")],
        row_labels: vec![String::from("C")],
        rows: vec![vec![
            ResultCell::Number { value: 1.25, prefix: Some(String::from("~")), units: None },
            ResultCell::Text(String::from("Very wide")),
        ]],
        footnotes: Vec::new(),
    };
    nl_rs.tabs.push(tab.clone());
    assert_eq!(format_result_cell_display_text(&ResultCell::Text(String::from("Very wide")), &nl_rs, &tab), "Very wide");
    assert!(AssumptionDiagnosticBanner::from_result(&nl_rs).is_some(), "`Very wide` cell must surface ambiguity banner");
}
