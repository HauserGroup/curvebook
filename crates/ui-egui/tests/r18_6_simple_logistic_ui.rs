#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Targeted `EguiHumanPathDriver` integration test for R18.6 Standalone XY Simple Logistic Regression:
//! - Catalog registration (`stats.simple_logistic`) on XY tables
//! - Parameter modal controls (`edit.sl.row_class`, `edit.sl.conf_bands`, `edit.sl.cutoff`, `edit.sl.plot_x_min`, `edit.sl.plot_x_max`)
//! - Committing parameter edits through `modal.ok` and verifying recomputed `ResultSheet` tabs, scalars, and sigmoid overlay.

use curvebook_doc::{AnalysisKind, Cell, DataSheet, ProjectDocument, SheetId, SimpleLogisticSpec, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ANALYSIS_CATALOG;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use serde_json::json;

struct LogisticModalState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl LogisticModalState {
    fn new(mut app: UiApp) -> Self {
        app.set_gui_only(true);
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver<'a>(state: LogisticModalState) -> EguiHumanPathDriver<'a, LogisticModalState> {
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

fn seed_logistic_project() -> (UiApp, SheetId) {
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
        table.data_sets[0].cells[r][0] = match y {
            Some(v) => Cell::number(v),
            None => Cell::empty(),
        };
    }
    let xy_id = SheetId::try_new("xy-log").unwrap();
    let ds = DataSheet::new(xy_id.clone(), "Binary Assay", TableArchetype::Xy(table)).unwrap();
    let mut doc = ProjectDocument::new("R18.6 Simple Logistic Test").unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(xy_id);

    let mut session = Session::from_document(doc);
    let kind = serde_json::to_value(AnalysisKind::SimpleLogisticRegression(SimpleLogisticSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-sl", "source_sheet_id": "xy-log", "kind": kind })).unwrap();
    let res_id = SheetId::try_new("res-sl").unwrap();

    let mut app = UiApp::new();
    app.set_gui_only(true);
    *app.session_mut() = session;
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &res_id));
    (app, res_id)
}

fn replace_textbox(driver: &mut EguiHumanPathDriver<'_, LogisticModalState>, wid: &str, text: &str) {
    driver.click_widget_id(wid).unwrap();
    driver.press_key("a", KeyModifiers { meta: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..KeyModifiers::default() }).unwrap();
    driver.press_key("Backspace", KeyModifiers::default()).unwrap();
    driver.type_text(text).unwrap();
    driver.run_settled();
}

#[test]
fn r18_6_simple_logistic_catalog_and_parameter_modal_flow() {
    // 1. Verify catalog entry exists
    assert!(ANALYSIS_CATALOG.iter().any(|item| item.id == "stats.simple_logistic"));

    // 2. Drive parameter modal through human-path clicks and text entry
    let (app, res_id) = seed_logistic_project();
    let mut driver = make_driver(LogisticModalState::new(app));

    // Toggle off Row Classification, enable custom plot X bounds, update cutoff to 0.65
    driver.click_widget_id("edit.sl.row_class").unwrap();
    replace_textbox(&mut driver, "edit.sl.cutoff", "0.65");
    driver.click_widget_id("edit.sl.plot_x_min_toggle").unwrap();
    replace_textbox(&mut driver, "edit.sl.plot_x_min", "0.0");
    driver.click_widget_id("edit.sl.plot_x_max_toggle").unwrap();
    replace_textbox(&mut driver, "edit.sl.plot_x_max", "7.0");
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let doc = driver.state().app.document();
    let rs = doc.result_sheet(&res_id).unwrap();
    let AnalysisKind::SimpleLogisticRegression(sl) = &rs.analysis.kind else {
        panic!("expected SimpleLogisticRegression");
    };
    assert!(!sl.emit_row_classification);
    assert!((sl.classification_cutoff - 0.65).abs() < 1e-12);
    assert_eq!(sl.plot_x_min, Some(0.0));
    assert_eq!(sl.plot_x_max, Some(7.0));
    assert_eq!(rs.tabs.len(), 3);
    assert!(rs.tabs.iter().all(|t| t.title != "Row Classification"));
    assert!((rs.fitted_curves[0].points.first().unwrap().x - 0.0).abs() < 1e-12);
    assert!((rs.fitted_curves[0].points.last().unwrap().x - 7.0).abs() < 1e-12);
}
