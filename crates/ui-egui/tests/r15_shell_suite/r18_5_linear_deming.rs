#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Targeted `EguiHumanPathDriver` integration test for R18.5:
//! - Arbitrary fixed-point linear regression (`edit.lin.fixed_point`, `x0`, `y0`)
//! - Standard addition (`edit.lin.standard_addition`) & line extension (`edit.lin.extend_x_int`)
//! - Model II Deming error model (`edit.lin.deming.equal`, `.sds`, `.sd_x`, `.sd_y`, `.ratio`, `.lambda`)

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, DemingErrorSpec, LinearRegressionSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use serde_json::json;

struct LinearModalState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl LinearModalState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver<'a>(state: LinearModalState) -> EguiHumanPathDriver<'a, LinearModalState> {
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

fn open_linear_modal(driver: &mut EguiHumanPathDriver<'_, LinearModalState>, res_id: &SheetId) {
    let doc = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc, res_id));
    driver.run_settled();
}

fn seed_linear_project(xs: &[f64], ys: &[f64]) -> (UiApp, SheetId) {
    let mut table = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    table.data_sets[0].title = String::from("Assay");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        table.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let xy_id = SheetId::try_new("xy-sa").unwrap();
    let ds = DataSheet::new(xy_id.clone(), "Standard Addition", TableArchetype::Xy(table)).unwrap();
    let mut doc = ProjectDocument::new("R18.5 Linear Test").unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(xy_id);

    let mut session = Session::from_document(doc);
    let kind = serde_json::to_value(AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-lin", "source_sheet_id": "xy-sa", "kind": kind })).unwrap();
    let res_id = SheetId::try_new("res-lin").unwrap();

    let mut app = UiApp::new();
    *app.session_mut() = session;
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &res_id));
    (app, res_id)
}

fn replace_textbox(driver: &mut EguiHumanPathDriver<'_, LinearModalState>, wid: &str, text: &str) {
    driver.click_widget_id(wid).unwrap();
    driver.press_key("a", KeyModifiers { meta: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..KeyModifiers::default() }).unwrap();
    driver.press_key("Backspace", KeyModifiers::default()).unwrap();
    driver.type_text(text).unwrap();
    driver.run_settled();
}

#[test]
fn r18_5_standard_addition_fixed_point_and_deming_error_modal_flow() {
    let xs = [0.0, 5.0, 10.0, 15.0, 20.0, 25.0];
    let ys = [0.182, 0.348, 0.505, 0.678, 0.835, 1.008];
    let (app, res_id) = seed_linear_project(&xs, &ys);
    let mut driver = make_driver(LinearModalState::new(app));

    // 1. Enable Standard Addition + Extend Line to X-intercept and commit via OK.
    driver.click_widget_id("edit.lin.standard_addition").unwrap();
    driver.click_widget_id("edit.lin.extend_x_int").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
            panic!("expected LinearRegression");
        };
        assert!(lin.standard_addition);
        assert!(lin.extend_line_to_x_intercept);
        let conc = rs.scalars.iter().find(|s| s.key == "StandardAdditionConcentration").expect("std add scalar").value;
        assert!((conc - 5.493_869_997_686_791).abs() < 1e-9, "unexpected conc {conc}");
        let curve_x0 = rs.fitted_curves[0].points[0].x;
        assert!((curve_x0 - (-conc)).abs() < 1e-9, "expected overlay extended to -conc, got {curve_x0}");
    }

    // 2. Reopen modal, enable fixed point (2.0, 0.25), commit, and verify line passes through (2.0, 0.25).
    open_linear_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.fixed_point").unwrap();
    replace_textbox(&mut driver, "edit.lin.fixed_point_x0", "2.0");
    replace_textbox(&mut driver, "edit.lin.fixed_point_y0", "0.25");
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
            panic!("expected LinearRegression");
        };
        assert_eq!(lin.fixed_point, Some((2.0, 0.25)));
        let slope = rs.scalars.iter().find(|s| s.key == "Slope").unwrap().value;
        let intercept = rs.scalars.iter().find(|s| s.key == "YIntercept").unwrap().value;
        assert!((intercept + slope * 2.0 - 0.25).abs() < 1e-12, "line must pass through (2.0, 0.25)");
    }

    // 3. Reopen modal, switch to Deming Model II, verify OLS-only controls disabled, set Known SDs (1.5, 0.75).
    open_linear_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.deming").unwrap();
    assert!(!driver.frame_meta().find_by_widget_id("edit.lin.origin").unwrap().enabled);
    assert!(!driver.frame_meta().find_by_widget_id("edit.lin.fixed_point").unwrap().enabled);
    assert!(!driver.frame_meta().find_by_widget_id("edit.lin.standard_addition").unwrap().enabled);

    driver.click_widget_id("edit.lin.deming.sds").unwrap();
    replace_textbox(&mut driver, "edit.lin.deming.sd_x", "1.5");
    replace_textbox(&mut driver, "edit.lin.deming.sd_y", "0.75");
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
            panic!("expected LinearRegression");
        };
        assert!(lin.deming_model_ii);
        assert_eq!(lin.deming_error, DemingErrorSpec::StandardDeviations { sd_x: 1.5, sd_y: 0.75 });
    }

    // 4. Switch Deming error model to VarianceRatio (lambda = 0.25) and commit.
    open_linear_modal(&mut driver, &res_id);
    driver.click_widget_id("edit.lin.deming.ratio").unwrap();
    replace_textbox(&mut driver, "edit.lin.deming.lambda", "0.25");
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let doc = driver.state().app.document();
    let rs = doc.result_sheet(&res_id).unwrap();
    let AnalysisKind::LinearRegression(lin) = &rs.analysis.kind else {
        panic!("expected LinearRegression");
    };
    assert_eq!(lin.deming_error, DemingErrorSpec::VarianceRatio { lambda: 0.25 });
}
