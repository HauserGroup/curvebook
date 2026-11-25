#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Focused `EguiHumanPathDriver` tests for the R19.2 standalone Levene / Brown–Forsythe variance test:
//! - `Analyze...` picker: the `stats.levene` entry is found by search on a `Column` table, selected by
//!   click, and launched with `modal.ok`, creating a median-centered (Brown–Forsythe) result sheet;
//! - parameters modal: clicking `edit.levene.center.mean` and `modal.ok` recomputes with the mean centre;
//! - every GUI result is cross-checked against a fresh headless `Session` (`D1`) and the published
//!   `scipy.stats.levene` / R 4.5.2 values used by `crates/engine/tests/r19_2_levene_engine.rs`.
//!
//! These are focused human-path control tests, not a five-stage `WorkflowRun`; they make no
//! `docs/evidence` or §6.2 completion claim.

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, DataSheet, LeveneCenter, LeveneTestSpec, ProjectDocument, ResultSheet, SheetId,
    TableArchetype, TableKind,
};
use curvebook_engine::Session;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveDialog, default_analysis_kind_for_id};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::json;

/// Same four groups as the engine `D1` oracle (`scipy.stats.levene` / R 4.5.2 `anova(lm)`).
const HET: [&[f64]; 4] = [
    &[10.2, 10.8, 9.9, 11.1, 10.5, 10.3],
    &[12.1, 14.8, 11.2, 15.6, 13.4, 16.2, 12.9, 14.1],
    &[18.5, 22.0, 16.4, 24.1, 19.8],
    &[25.0, 25.4, 24.8, 25.7, 25.1, 25.5, 24.9],
];

struct LeveneUiState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

fn make_driver<'a>(mut app: UiApp) -> EguiHumanPathDriver<'a, LeveneUiState> {
    app.set_gui_only(true);
    let views = app.egui_views.clone();
    let state = LeveneUiState { app, views, intents: PendingUiIntents::new() };
    let tokens = Tokens::from_theme_spec(&state.app.theme, &state.app.locale);
    let mut driver = EguiHumanPathDriver::new(state, ViewportMetrics::new(1024.0, 700.0, 1.0), |ui, meta, st| {
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

fn het_document() -> (ProjectDocument, SheetId) {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, HET.len(), 8).unwrap();
    for (c, (ds, values)) in t.data_sets.iter_mut().zip(HET.iter()).enumerate() {
        ds.title = format!("Group {}", c + 1);
        for (r, &v) in values.iter().enumerate() {
            ds.cells[r][0] = Cell::number(v);
        }
    }
    let id = SheetId::try_new("col-het").unwrap();
    let ds = DataSheet::new(id.clone(), "Variance groups", TableArchetype::Column(t)).unwrap();
    let mut doc = ProjectDocument::new("R19.2 Levene UI").unwrap();
    doc.data_sheets.push(ds);
    doc.active_sheet_id = Some(id.clone());
    (doc, id)
}

/// Headless oracle: run the same analysis through a fresh `Session` on the same document.
fn headless_scalars(center: &str) -> (f64, f64) {
    let (doc, _) = het_document();
    let mut session = Session::from_document(doc);
    session.execute("stats.run", json!({ "sheet_id": "oracle", "source_sheet_id": "col-het", "kind": "levene", "center": center })).unwrap();
    let rs = session.document().result_sheet(&SheetId::try_new("oracle").unwrap()).unwrap();
    (scalar(rs, "F"), scalar(rs, "P value"))
}

fn scalar(rs: &ResultSheet, key: &str) -> f64 {
    rs.scalars.iter().find(|s| s.key == key && s.data_set_index.is_none()).unwrap_or_else(|| panic!("missing scalar `{key}`")).value
}

#[test]
fn r19_2_levene_catalog_entry_is_column_and_mv_only() {
    let item = ANALYSIS_CATALOG.iter().find(|i| i.id == "stats.levene").expect("stats.levene catalog entry");
    assert!(item.label.contains("Levene"));
    let kind = default_analysis_kind_for_id("stats.levene").expect("default kind");
    assert_eq!(kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }));
    assert!(kind.is_compatible_with(TableKind::Column));
    assert!(kind.is_compatible_with(TableKind::MultipleVariables));
    for incompatible in [TableKind::Xy, TableKind::Grouped, TableKind::Contingency, TableKind::Survival, TableKind::Nested, TableKind::PartsOfWhole] {
        assert!(!kind.is_compatible_with(incompatible), "Levene must not be offered for {incompatible:?}");
    }
}

#[test]
fn r19_2_levene_analyze_picker_and_parameter_modal_human_path() {
    let (doc, col_id) = het_document();
    let mut app = UiApp::new();
    *app.session_mut() = Session::from_document(doc);
    app.sync_views();
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc_snap, &col_id));
    let mut driver = make_driver(app);

    // Search autofocuses on open; typing filters the catalog to the Levene entry.
    driver.type_text("levene").unwrap();
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.levene").is_some(), "search `levene` must list the entry");
    assert!(driver.frame_meta().find_by_widget_id("analyze.entry.stats.one_way_anova").is_none(), "search must filter other entries");
    driver.click_widget_id("analyze.entry.stats.levene").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let res_id = driver.state().app.document().active_sheet_id.clone().expect("created result sheet is active");
    let rs = driver.state().app.document().result_sheet(&res_id).expect("result sheet");
    assert_eq!(rs.analysis.kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Median }));
    let (f_oracle, p_oracle) = headless_scalars("median");
    assert!((scalar(rs, "F") - f_oracle).abs() <= 1e-12, "GUI F must equal headless Session F");
    assert!((scalar(rs, "P value") - p_oracle).abs() <= 1e-12, "GUI P must equal headless Session P");
    // Published scipy.stats.levene(center="median") value; tolerance as in the engine D1 oracle.
    assert!((f_oracle - 6.458_547_396_160_991).abs() <= 1e-10);

    // Parameters modal: switch the centre to the mean through the rendered radio and OK.
    let doc_now = driver.state().app.document().clone();
    assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_now, &res_id));
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("edit.levene.center.median").is_some(), "median radio rendered");
    driver.click_widget_id("edit.levene.center.mean").unwrap();
    driver.click_widget_id("modal.ok").unwrap();
    driver.run_settled();
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let rs = driver.state().app.document().result_sheet(&res_id).expect("result sheet after edit");
    assert_eq!(rs.analysis.kind, AnalysisKind::LeveneTest(LeveneTestSpec { center: LeveneCenter::Mean }));
    let (f_mean, p_mean) = headless_scalars("mean");
    assert!((scalar(rs, "F") - f_mean).abs() <= 1e-12);
    assert!((scalar(rs, "P value") - p_mean).abs() <= 1e-12);
    // Published scipy.stats.levene(center="mean") value.
    assert!((f_mean - 7.895_495_746_240_47).abs() <= 1e-10);
}
