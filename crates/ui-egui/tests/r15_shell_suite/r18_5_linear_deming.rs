#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Targeted `EguiHumanPathDriver` modal tests for `R18.1` (nonlinear fit params & 16-locale strings),
//! `R18.5` (linear standard addition, fixed point, Deming Model II), and `R18.6` (XY simple logistic).

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, DemingErrorSpec, LinearRegressionSpec, ProjectDocument, SheetId, SimpleLogisticSpec, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ANALYSIS_CATALOG;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use curvebook_ui_egui::i18n::{LOCALES, tr};
use serde_json::json;

use crate::common::{ShellTestState, make_default_driver, seed_xy_doc, set_textbox};

fn scalar_val(rs: &curvebook_doc::ResultSheet, key: &str) -> f64 {
    rs.scalars.iter().find(|s| s.key == key).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

fn open_edit_modal(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, res_id: &SheetId) {
    let doc = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc, res_id));
    driver.run_settled();
}

fn seed_analysis_edit_modal(doc: ProjectDocument, source_id: &str, res_id: &str, kind: AnalysisKind) -> (UiApp, SheetId) {
    let mut session = Session::from_document(doc);
    let kind_val = serde_json::to_value(kind).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": res_id, "source_sheet_id": source_id, "kind": kind_val })).unwrap();
    let rsid = SheetId::try_new(res_id).unwrap();
    let mut app = UiApp::from_session(session);
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &rsid));
    (app, rsid)
}

#[test]
fn r18_1_all_new_fit_modal_keys_localized_across_16_locales() {
    const KEYS: &[&str] = &[
        "fit.conv.default",
        "fit.conv.medium",
        "fit.conv.quick",
        "fit.conv.strict",
        "fit.conv.title",
        "fit.outlier.cleaned_tab",
        "fit.outlier.count_only",
        "fit.outlier.eliminate",
        "fit.outlier.none",
        "fit.outlier.title",
        "fit.rep.individual",
        "fit.rep.row_means",
        "fit.rep.title",
        "fit.wt.src.observed",
        "fit.wt.src.predicted",
        "fit.wt.src.title",
    ];
    assert_eq!(LOCALES.len(), 16);
    for locale in LOCALES {
        for key in KEYS {
            let val = tr(locale, key);
            assert!(!val.is_empty() && val != *key, "missing translation for key `{key}` in locale `{locale}`");
        }
    }
}

#[test]
fn r18_5_standard_addition_fixed_point_and_deming_error_modal_flow() {
    let (doc, _) = seed_xy_doc(
        "R18.5 Linear Test",
        "xy-sa",
        "Standard Addition",
        "X",
        &[0.0, 5.0, 10.0, 15.0, 20.0, 25.0],
        &[("Assay", &[0.182, 0.348, 0.505, 0.678, 0.835, 1.008])],
    )
    .unwrap();
    let (app, res_id) = seed_analysis_edit_modal(doc, "xy-sa", "res-lin", AnalysisKind::LinearRegression(LinearRegressionSpec::default()));
    let mut driver = make_default_driver(app);

    // 1. Standard Addition + Extend Line to X-intercept.
    driver.click_widget_id("edit.lin.standard_addition").unwrap();
    driver.click_widget_id("edit.lin.extend_x_int").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
            panic!("expected LinearRegression");
        };
        assert!(lin.standard_addition && lin.extend_line_to_x_intercept);
        let conc = scalar_val(rs, "StandardAdditionConcentration");
        assert!((conc - 5.493_869_997_686_791).abs() < 1e-9);
        assert!((rs.fitted_curves[0].points[0].x - (-conc)).abs() < 1e-9);
    }

    // 2. Fixed point (2.0, 0.25).
    open_edit_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.fixed_point").unwrap();
    set_textbox(&mut driver, "edit.lin.fixed_point_x0", "2.0").unwrap();
    set_textbox(&mut driver, "edit.lin.fixed_point_y0", "0.25").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
            panic!("expected LinearRegression");
        };
        assert_eq!(lin.fixed_point, Some((2.0, 0.25)));
        let slope = scalar_val(rs, "Slope");
        let intercept = scalar_val(rs, "YIntercept");
        assert!((intercept + slope * 2.0 - 0.25).abs() < 1e-12);
    }

    // 3. Deming Model II: OLS-only controls disabled, Known SDs (1.5, 0.75).
    open_edit_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.deming").unwrap();
    for wid in ["edit.lin.origin", "edit.lin.fixed_point", "edit.lin.standard_addition"] {
        assert!(!driver.frame_meta().find_by_widget_id(wid).unwrap().enabled, "{wid} must disable under Deming");
    }
    driver.click_widget_id("edit.lin.deming.sds").unwrap();
    set_textbox(&mut driver, "edit.lin.deming.sd_x", "1.5").unwrap();
    set_textbox(&mut driver, "edit.lin.deming.sd_y", "0.75").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    {
        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        assert!(matches!(
            &rs.analysis.kind,
            AnalysisKind::LinearRegression(lin)
                if lin.deming_model_ii && lin.deming_error == DemingErrorSpec::StandardDeviations { sd_x: 1.5, sd_y: 0.75 }
        ));
    }

    // 4. Deming VarianceRatio (lambda = 0.25).
    open_edit_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.deming.ratio").unwrap();
    set_textbox(&mut driver, "edit.lin.deming.lambda", "0.25").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
    assert!(matches!(&rs.analysis.kind, AnalysisKind::LinearRegression(lin) if lin.deming_error == DemingErrorSpec::VarianceRatio { lambda: 0.25 }));
}

#[test]
fn r18_6_simple_logistic_catalog_and_parameter_modal_flow() {
    assert!(ANALYSIS_CATALOG.iter().any(|item| item.id == "stats.simple_logistic"));

    let pts = [
        (0.5, Some(0.0)),
        (1.0, Some(0.0)),
        (1.5, Some(0.0)),
        (2.0, Some(1.0)),
        (2.5, Some(0.0)),
        (3.0, Some(1.0)),
        (3.5, Some(0.0)),
        (4.0, Some(1.0)),
        (4.5, Some(1.0)),
        (5.0, Some(1.0)),
        (5.5, Some(1.0)),
        (6.0, Some(1.0)),
        (3.2, None),
    ];
    let mut table = XyTable::new(SubcolumnMode::Single, 1, pts.len()).unwrap();
    table.data_sets[0].title = String::from("Response");
    for (r, (x, y)) in pts.into_iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = y.map_or_else(Cell::empty, Cell::number);
    }
    let xy_id = SheetId::try_new("xy-log").unwrap();
    let mut doc = ProjectDocument::new("R18.6 Simple Logistic Test").unwrap();
    doc.data_sheets.push(DataSheet::new(xy_id.clone(), "Binary Assay", TableArchetype::Xy(table)).unwrap());
    doc.active_sheet_id = Some(xy_id);

    let (app, res_id) = seed_analysis_edit_modal(doc, "xy-log", "res-sl", AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default()));
    let mut driver = make_default_driver(app);

    driver.click_widget_id("edit.sl.row_class").unwrap();
    set_textbox(&mut driver, "edit.sl.cutoff", "0.65").unwrap();
    driver.click_widget_id("edit.sl.plot_x_min_toggle").unwrap();
    set_textbox(&mut driver, "edit.sl.plot_x_min", "0.0").unwrap();
    driver.click_widget_id("edit.sl.plot_x_max_toggle").unwrap();
    set_textbox(&mut driver, "edit.sl.plot_x_max", "7.0").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
    let AnalysisKind::SimpleLogisticRegression(sl) = &rs.analysis.kind else {
        panic!("expected SimpleLogisticRegression");
    };
    assert!(!sl.emit_row_classification && (sl.classification_cutoff - 0.65).abs() < 1e-12);
    assert_eq!((sl.plot_x_min, sl.plot_x_max), (Some(0.0), Some(7.0)));
    assert_eq!(rs.tabs.len(), 3);
    assert!(rs.tabs.iter().all(|t| t.title != "Row Classification"));
    assert!((rs.fitted_curves[0].points.first().unwrap().x - 0.0).abs() < 1e-12);
    assert!((rs.fitted_curves[0].points.last().unwrap().x - 7.0).abs() < 1e-12);
}
