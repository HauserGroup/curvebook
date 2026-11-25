#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Targeted `EguiHumanPathDriver` regression test for Descriptive Statistics
//! `custom_upper_percentile` modal controls (`edit.desc.custom_pct_toggle`,
//! `edit.desc.custom_pct`) and `AnalyzeDialogState` (`analysis.run`) wiring.

use curvebook_doc::{AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ProjectDocument, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::AnalyzeDialogState;
use curvebook_ui_egui::dialogs_canvas::dialog_patch::patch_analyze;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::i18n::LocalePreset;
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use serde_json::{Map, json};

struct DescModalState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl DescModalState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver<'a>(state: DescModalState) -> EguiHumanPathDriver<'a, DescModalState> {
    let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::default(), |ui, meta, st| {
        let mut pass = PendingUiIntents::new();
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut pass);
        st.intents.merge_pass(pass);
    });
    driver.set_tokens(tokens);
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_all_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn seed_descriptive_project() -> (UiApp, SheetId) {
    let vals = [2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, vals.len()).unwrap();
    table.data_sets[0].title = String::from("Assay");
    for (r, v) in vals.into_iter().enumerate() {
        table.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let col_id = SheetId::try_new("col-desc").unwrap();
    let ds = DataSheet::new(col_id.clone(), "Descriptive Data", TableArchetype::Column(table)).unwrap();
    let mut doc = ProjectDocument::new("Descriptive Custom Percentile UI Test").unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(col_id);

    let mut session = Session::from_document(doc);
    let kind = serde_json::to_value(AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
    let _ = session.execute("analysis.run", json!({ "sheet_id": "res-desc", "source_sheet_id": "col-desc", "kind": kind })).unwrap();
    let res_id = SheetId::try_new("res-desc").unwrap();

    let mut app = UiApp::new();
    *app.session_mut() = session;
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_edit_parameters(&doc_snap, &res_id));
    (app, res_id)
}

fn replace_textbox(driver: &mut EguiHumanPathDriver<'_, DescModalState>, wid: &str, text: &str) {
    driver.click_widget_id(wid).unwrap();
    driver.press_key("a", KeyModifiers { meta: cfg!(target_os = "macos"), ctrl: !cfg!(target_os = "macos"), ..KeyModifiers::default() }).unwrap();
    driver.press_key("Backspace", KeyModifiers::default()).unwrap();
    driver.type_text(text).unwrap();
    driver.run_settled();
}

#[test]
fn descriptive_modal_custom_upper_percentile_round_trip() {
    let (app, res_id) = seed_descriptive_project();
    let mut driver = make_driver(DescModalState::new(app));

    // Initially unchecked -> numeric input is not shown.
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct_toggle").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());

    // Toggle ON -> defaults to 90.0 and reveals numeric input.
    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());

    // Out-of-range values (<= 50 or >= 100) are ignored and do not overwrite valid draft.
    replace_textbox(&mut driver, "edit.desc.custom_pct", "40");
    replace_textbox(&mut driver, "edit.desc.custom_pct", "100");
    {
        let draft = driver.state().app.canvas_dialogs.edit_params.draft.as_ref().expect("draft");
        let AnalysisKind::DescriptiveStats(d) = &draft.kind else {
            panic!("expected DescriptiveStats draft");
        };
        assert_eq!(d.custom_upper_percentile, Some(90.0));
    }

    // Enter valid P = 95 and commit via modal.ok.
    replace_textbox(&mut driver, "edit.desc.custom_pct", "95");
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    {
        let doc = driver.state().app.document();
        let rs = doc.result_sheet(&res_id).unwrap();
        let AnalysisKind::DescriptiveStats(d) = &rs.analysis.kind else {
            panic!("expected DescriptiveStats");
        };
        assert_eq!(d.custom_upper_percentile, Some(95.0));
        let tab = rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
        assert!(tab.row_labels.iter().any(|l| l == "5% Percentile (Type 7)"));
        assert!(tab.row_labels.iter().any(|l| l == "95% Percentile (Type 7)"));
        assert!(tab.row_labels.iter().any(|l| l == "95% CI of Geometric mean"));
        assert!(tab.row_labels.iter().any(|l| l == "95% CI of Harmonic mean"));
        assert!(tab.row_labels.iter().any(|l| l == "95% CI of Quadratic mean (RMS)"));
    }

    // Reopen modal, toggle OFF, and commit -> clears custom_upper_percentile back to None.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &res_id));
    }
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_some());
    driver.click_widget_id("edit.desc.custom_pct_toggle").unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.desc.custom_pct").is_none());
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let doc = driver.state().app.document();
    let rs = doc.result_sheet(&res_id).unwrap();
    let AnalysisKind::DescriptiveStats(d) = &rs.analysis.kind else {
        panic!("expected DescriptiveStats");
    };
    assert_eq!(d.custom_upper_percentile, None);
    let tab = rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
    assert!(!tab.row_labels.iter().any(|l| l.contains("Type 7")));
}

#[test]
fn analyze_dialog_wires_custom_upper_percentile_into_analysis_run() {
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5).unwrap();
    for r in 0..5 {
        table.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let sheet = DataSheet::new(SheetId::try_new("col-run").unwrap(), "Run Data", TableArchetype::Column(table)).unwrap();
    let mut dlg = AnalyzeDialogState::default();
    dlg.open_for_data_sheet(&sheet);
    dlg.select_catalog_entry("stats.descriptive");
    let preset = LocalePreset::default();

    // Reject out-of-range percentile in (50, 100).
    let mut bad = Map::new();
    bad.insert(String::from("custom_upper_percentile"), json!(50.0));
    assert!(patch_analyze(&mut dlg, &bad, &preset).is_err());

    let mut patch = Map::new();
    patch.insert(String::from("custom_upper_percentile"), json!(97.5));
    patch_analyze(&mut dlg, &patch, &preset).unwrap();

    let intent = dlg.build_create_new_intent().expect("analysis.run intent");
    assert_eq!(intent.command_id, "analysis.run");
    let pct = intent.params.pointer("/kind/DescriptiveStats/custom_upper_percentile").and_then(serde_json::Value::as_f64);
    assert_eq!(pct, Some(97.5));
}
