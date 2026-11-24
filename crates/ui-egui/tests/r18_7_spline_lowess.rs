#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Targeted `EguiHumanPathDriver` integration test for R18.7 `FitSplineLowess`:
//! - Selecting methods (`AkimaSubspline`, `SmoothingSpline`, `Lowess`) via rendered modal radios
//! - Toggling `edit.spline.emit_coords` and `edit.spline.interpolate_unknowns`
//! - Committing via `modal.ok` and verifying live recomputation, tabs, and LOWESS warnings.

use curvebook_doc::{
    AnalysisKind, Cell, ChecklistStatus, DataSheet, LowessPreset, ProjectDocument, ResultStatus, ResultTabKind, SheetId, SplineLowessMethod,
    SplineLowessSpec, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiBackend};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

struct SplineModalState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl SplineModalState {
    fn new(mut app: UiApp) -> Self {
        app.set_ui_backend(UiBackend::Egui);
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver<'a>(state: SplineModalState) -> EguiHumanPathDriver<'a, SplineModalState> {
    let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::default(), |ui, meta, st| {
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_tokens(tokens);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.app.egui_views = st.views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn open_spline_modal(driver: &mut EguiHumanPathDriver<'_, SplineModalState>, res_id: &SheetId) {
    let doc = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc, res_id));
    driver.run_settled();
}

fn seed_spline_project() -> (UiApp, SheetId) {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
    table.data_sets[0].title = String::from("Signal");
    let pts = [(0.0, 0.0), (1.0, 1.0), (2.0, 4.0), (3.0, 9.0), (4.0, 16.0), (5.0, 25.0)];
    for (r, &(x, y)) in pts.iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    // Forward unknown (X = 2.5, blank Y) and inverse unknown (blank X, Y = 9.0)
    table.x_cells[6][0] = Cell::number(2.5);
    table.data_sets[0].cells[7][0] = Cell::number(9.0);

    let xy_id = SheetId::try_new("xy-sp").unwrap();
    let ds = DataSheet::new(xy_id.clone(), "Spline Standard", TableArchetype::Xy(table)).unwrap();
    let mut doc = ProjectDocument::new("R18.7 Spline Test").unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(xy_id);

    let mut session = Session::from_document(doc);
    let kind = serde_json::to_value(AnalysisKind::FitSplineLowess(SplineLowessSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-sp", "source_sheet_id": "xy-sp", "kind": kind })).unwrap();
    let res_id = SheetId::try_new("res-sp").unwrap();

    let mut app = UiApp::new();
    app.set_ui_backend(UiBackend::Egui);
    app.set_gui_only(true);
    *app.session_mut() = session;
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &res_id));
    (app, res_id)
}

#[test]
fn r18_7_spline_lowess_parameter_modal_human_path_flow() {
    let (app, res_id) = seed_spline_project();
    let mut driver = make_driver(SplineModalState::new(app));

    // 1. Switch to Akima subspline, enable Curve Coordinates tab & Interpolate Unknowns, commit via OK.
    driver.click_widget_id("edit.spline.method.AkimaSubspline").unwrap();
    driver.click_widget_id("edit.spline.emit_coords").unwrap();
    driver.click_widget_id("edit.spline.interpolate_unknowns").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        let AnalysisKind::FitSplineLowess(sp) = &rs.analysis.kind else {
            panic!("expected FitSplineLowess");
        };
        assert_eq!(sp.method, SplineLowessMethod::AkimaSubspline);
        assert!(sp.emit_curve_coordinates_tab);
        assert!(sp.interpolate_unknowns);
        assert_eq!(rs.fitted_curves[0].equation_id, "akima_subspline");
        assert!(rs.tabs.iter().any(|t| t.kind == ResultTabKind::CurveCoordinates));
        assert!(rs.tabs.iter().any(|t| t.kind == ResultTabKind::InterpolatedUnknowns));
    }

    // 2. Reopen modal, switch to Smoothing Spline, commit, and verify effective_df scalar.
    open_spline_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.spline.method.SmoothingSpline").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.fitted_curves[0].equation_id, "smoothing_spline");
        assert!(rs.scalars.iter().any(|s| s.key == "effective_df"));
    }

    // 3. Reopen modal, switch to LOWESS with Fine preset, commit, and verify span + N < 20 warning.
    open_spline_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.spline.method.Lowess").unwrap();
    driver.click_widget_id("edit.spline.lowess_preset.Fine").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        assert_eq!(rs.status, ResultStatus::UpToDate);
        let AnalysisKind::FitSplineLowess(sp) = &rs.analysis.kind else {
            panic!("expected FitSplineLowess");
        };
        assert_eq!(sp.method, SplineLowessMethod::Lowess);
        assert_eq!(sp.lowess_preset, LowessPreset::Fine);
        assert_eq!(rs.fitted_curves[0].equation_id, "lowess");
        assert!(rs.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("LOWESS")));
    }
}
