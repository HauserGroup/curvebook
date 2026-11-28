#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Focused `EguiHumanPathDriver` tests for standalone Frequency Distribution and Histogram
//! graphing (Analyze catalog, parameter modal, auto-created histogram, and Change Graph Type).

use super::common;

use curvebook_doc::{
    AnalysisKind, ColumnDesignHint, FrequencyBinningRule, FrequencyDistributionSpec, FrequencyOutputMode, GraphKind, SheetId, TableKind,
};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveDialog, GRAPH_TYPE_OPTIONS, default_analysis_kind_for_id};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::json;

use common::{make_shell_driver, seed_column_doc};

const DIST_GROUPS: [(&str, &[f64]); 2] = [
    ("Control", &[1.2, 2.5, 2.8, 3.1, 3.9, 4.0, 4.4, 5.2, 6.1, 7.8, 8.3, 9.5]),
    ("Treated", &[4.0, 5.1, 5.8, 6.2, 7.0, 7.5, 8.1, 8.9, 9.4, 10.2, 11.0, 12.1]),
];

#[test]
fn frequency_distribution_catalog_and_histogram_graph_type_entries_registered() {
    let item = ANALYSIS_CATALOG.iter().find(|i| i.id == "stats.frequency_distribution").expect("catalog entry");
    assert!(item.label.contains("Frequency"));
    let kind = default_analysis_kind_for_id("stats.frequency_distribution").expect("default kind");
    assert_eq!(kind, AnalysisKind::FrequencyDistribution(FrequencyDistributionSpec::default()));
    for ok in [TableKind::Column, TableKind::Xy, TableKind::Grouped, TableKind::MultipleVariables] {
        assert!(kind.is_compatible_with(ok), "expected compatibility with {ok:?}");
    }
    for bad in [TableKind::Contingency, TableKind::Survival, TableKind::Nested, TableKind::PartsOfWhole] {
        assert!(!kind.is_compatible_with(bad), "expected incompatibility with {bad:?}");
    }

    assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "hist.bars" && o.graph_kind == GraphKind::Histogram));
    assert!(GRAPH_TYPE_OPTIONS.iter().any(|o| o.id == "hist.cumulative" && o.graph_kind == GraphKind::Histogram));
}

#[test]
fn frequency_distribution_analyze_modal_auto_histogram_and_change_graph_type_human_path() {
    let (doc, col_id) = seed_column_doc("Frequency UI", "col-dist", "Assay values", ColumnDesignHint::UnpairedReplicates, &DIST_GROUPS).unwrap();
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    let snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&snap, &col_id));
    let mut driver = make_shell_driver(app, ViewportMetrics::new(1120.0, 760.0, 1.0));

    driver.type_text("frequency").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.frequency_distribution").is_some());
    driver.click_widget_id("analyze.entry.stats.frequency_distribution").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // Verify result sheet and auto-created Histogram graph sheet exist.
    let res_id: SheetId = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|r| matches!(r.analysis.kind, AnalysisKind::FrequencyDistribution(_)))
        .map(|r| r.id.clone())
        .expect("frequency distribution result sheet created");
    let hist_graph = driver
        .state()
        .app
        .document()
        .graph_sheets
        .iter()
        .find(|g| g.spec.kind == GraphKind::Histogram && g.spec.primary_source_sheet_id == res_id)
        .expect("auto-created linked histogram graph");
    let hist_gid = hist_graph.id.clone();

    // Reopen Edit Parameters modal and switch to Scott's rule + Percentages + Gaussian overlay.
    let doc_now = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_now, &res_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.freq.binning.scott").is_some());
    driver.click_widget_id("edit.freq.binning.scott").unwrap();
    driver.click_widget_id("edit.freq.output.pct").unwrap();
    driver.click_widget_id("edit.freq.overlay_gaussian").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let updated = driver.state().app.document().result_sheet(&res_id).expect("updated result");
    match &updated.analysis.kind {
        AnalysisKind::FrequencyDistribution(fd) => {
            assert_eq!(fd.binning_rule, FrequencyBinningRule::AutoScott);
            assert_eq!(fd.output_mode, FrequencyOutputMode::Percentages);
            assert!(fd.overlay_gaussian);
        }
        other => panic!("expected FrequencyDistribution, got {other:?}"),
    }

    // Open Change Graph Type on the histogram graph and switch to Cumulative Frequency Curve.
    let _ = dispatch_gui_action(
        &mut driver.state_mut().app,
        "ui.invoke_menu",
        json!({ "command_id": "ui.changeGraphType.open", "params": { "sheet_id": hist_gid.as_str() } }),
    )
    .unwrap();
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("graph_type.card.hist.cumulative").is_some());
    driver.click_widget_id("graph_type.card.hist.cumulative").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();

    let updated_g = driver.state().app.document().graph_sheet(&hist_gid).expect("updated histogram graph");
    assert_eq!(updated_g.spec.kind, GraphKind::Histogram);
    assert!(updated_g.spec.histogram.cumulative);
}
