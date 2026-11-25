#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Targeted `EguiHumanPathDriver` integration test for R18.7 `FitSplineLowess`.

#[path = "common/mod.rs"]
mod common;

use curvebook_doc::{
    AnalysisKind, Cell, ChecklistStatus, DataSheet, LowessPreset, ProjectDocument, ResultStatus, ResultTabKind, SheetId, SplineLowessMethod,
    SplineLowessSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use serde_json::json;

use common::make_default_driver;

#[test]
fn r18_7_spline_lowess_parameter_modal_human_path_flow() {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
    table.data_sets[0].title = String::from("Signal");
    for (r, &(x, y)) in [(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0), (4.0, 16.0), (5.0, 25.0)].iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    table.x_cells[6][0] = Cell::number(2.5);
    table.data_sets[0].cells[7][0] = Cell::number(9.0);

    let xy_id = SheetId::try_new("xy-sp").unwrap();
    let mut doc = ProjectDocument::new("R18.7 Spline Test").unwrap();
    doc.data_sheets.push(DataSheet::new(xy_id.clone(), "Spline Standard", TableArchetype::Xy(table)).unwrap());
    doc.active_sheet_id = Some(xy_id);

    let mut session = Session::from_document(doc);
    let kind = serde_json::to_value(AnalysisKind::FitSplineLowess(SplineLowessSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-sp", "source_sheet_id": "xy-sp", "kind": kind })).unwrap();
    let res_id = SheetId::try_new("res-sp").unwrap();

    let mut app = UiApp::from_session(session);
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&snap, &res_id));
    let mut driver = make_default_driver(app);

    let cases: &[(&[&str], SplineLowessMethod, &str)] = &[
        (
            &["edit.spline.method.AkimaSubspline", "edit.spline.emit_coords", "edit.spline.interpolate_unknowns"],
            SplineLowessMethod::AkimaSubspline,
            "akima_subspline",
        ),
        (&["edit.spline.method.SmoothingSpline"], SplineLowessMethod::SmoothingSpline, "smoothing_spline"),
        (&["edit.spline.method.Lowess", "edit.spline.lowess_preset.Fine"], SplineLowessMethod::Lowess, "lowess"),
    ];

    for (idx, &(clicks, method, eq_id)) in cases.iter().enumerate() {
        if idx > 0 {
            let doc_now = driver.state().app.document().clone();
            assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_now, &res_id));
            driver.run_settled();
        }
        for wid in clicks {
            driver.click_widget_id(wid).unwrap();
        }
        driver.click_widget_id("modal.ok").unwrap();
        driver.run_settled();

        let rs = driver.state().app.document().result_sheet(&res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.fitted_curves[0].equation_id, eq_id);
        let AnalysisKind::FitSplineLowess(sp) = &rs.analysis.kind else {
            panic!("expected FitSplineLowess");
        };
        assert_eq!(sp.method, method);
        match method {
            SplineLowessMethod::AkimaSubspline => {
                assert!(sp.emit_curve_coordinates_tab && sp.interpolate_unknowns);
                assert!(rs.tabs.iter().any(|t| t.kind == ResultTabKind::CurveCoordinates));
                assert!(rs.tabs.iter().any(|t| t.kind == ResultTabKind::InterpolatedUnknowns));
            }
            SplineLowessMethod::SmoothingSpline => assert!(rs.scalars.iter().any(|s| s.key == "effective_df")),
            SplineLowessMethod::Lowess => {
                assert_eq!(sp.lowess_preset, LowessPreset::Fine);
                assert!(rs.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("LOWESS")));
            }
            _ => {}
        }
    }
}
