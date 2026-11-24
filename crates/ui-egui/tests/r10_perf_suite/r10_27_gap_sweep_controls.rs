#![forbid(unsafe_code)]

//! Targeted `EguiHumanPathDriver` regression suite for `GWF-18` through `GWF-21`
//! (65-workflow non-Format-Graph GUI gap fixes + Heatmap persistence + `WF-60` XY copy/paste).

use std::sync::Arc;

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphKind, GraphSheet, MultipleVariablesTable,
    MultivariateSpec, OneWayAnovaSpec, OneWayMethod, ProjectDocument, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TableKind,
    TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_palette::Rgba8;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AxisTarget};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::stats_modals::render_stat_params_modal;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

struct TestState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
    last_error: Option<String>,
}

impl TestState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new(), last_error: None }
    }
}

fn make_driver<'a>(app: UiApp, metrics: ViewportMetrics) -> EguiHumanPathDriver<'a, TestState> {
    let mut driver = EguiHumanPathDriver::new(TestState::new(app), metrics, |ui, meta, st| {
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            if let Err(err) = st.intents.commit_if_settled(&mut st.app, false) {
                st.last_error = Some(err.to_string());
            }
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

fn commit_intents(driver: &mut EguiHumanPathDriver<'_, TestState>) -> Result<(), String> {
    let st = driver.state_mut();
    st.app.egui_views = st.views.clone();
    let mut intents = std::mem::take(&mut st.intents);
    let res = intents.commit_if_settled(&mut st.app, false).map(|_| ()).map_err(|e| e.to_string());
    st.views = st.app.egui_views.clone();
    driver.run_settled();
    res
}

fn replace_focused_text(driver: &mut EguiHumanPathDriver<'_, TestState>, text: &str) -> Result<(), String> {
    driver.press_egui_key(Key::A, Modifiers::COMMAND);
    driver.type_text(text)
}

fn assert_modal_parity(driver: &EguiHumanPathDriver<'_, TestState>, label: &str) -> Result<(), String> {
    let parity = driver.verify_pixel_to_hit_box_parity();
    let filtered: Vec<_> = parity.violations.iter().filter(|v| v.widget_id != "canvas.viewport" && v.widget_id != "canvas.split.viewport").collect();
    if !filtered.is_empty() {
        return Err(format!("{label}: parity violations: {filtered:?}"));
    }
    let sb = driver.check_scrollbar_non_overlap();
    if !sb.is_empty() {
        return Err(format!("{label}: scrollbar overlap: {sb:?}"));
    }
    Ok(())
}

fn make_xy_and_graph_doc() -> Result<(ProjectDocument, SheetId, SheetId), Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("GWF Gap Sweep Doc")?;
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 4)?;
    xy.x_title = String::from("Time (min)");
    xy.data_sets[0].title = String::from("Y1");
    xy.data_sets[1].title = String::from("Y2");
    for r in 0..4 {
        let x = (r as f64 + 1.0) * 10.0;
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(x * 2.0);
        xy.data_sets[1].cells[r][0] = Cell::number(x * 0.5);
    }
    let xy_id = SheetId::try_new("ds-xy")?;
    let ds = DataSheet::new(xy_id.clone(), "XY Table", TableArchetype::Xy(xy))?;
    let gid = SheetId::try_new("gs-xy")?;
    let mut gs = GraphSheet::new_uninitialized_for_data(gid.clone(), &ds)?;
    gs.initialized = true;
    gs.spec.kind = GraphKind::Xy;
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(gs)?;
    doc.select_in_navigator(&gid)?;
    Ok((doc, xy_id, gid))
}

fn new_egui_app(doc: ProjectDocument) -> UiApp {
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.set_gui_only(true);
    app.set_human_path_only(true);
    app.nav.welcome.open = false;
    app.egui_views.launcher_open = false;
    app
}

#[test]
fn gwf_18_format_axes_major_tick_interval_and_minor_ticks() -> Result<(), String> {
    let (doc, _xy_id, gid) = make_xy_and_graph_doc().map_err(|e| e.to_string())?;
    let mut app = new_egui_app(doc);
    {
        let gs = app.document().graph_sheet(&gid).ok_or("missing graph")?.clone();
        app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y);
        app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gid.clone() };
        app.sync_views();
    }

    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    driver.click_widget_id("edit.tab.axes.y")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.major_tick_interval").is_some());
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.minor_ticks_per_interval").is_some());
    assert_modal_parity(&driver, "FormatAxes initial")?;

    // Invalid interval (<= 0) displays error and blocks commit while keeping FormatAxes open.
    driver.click_widget_id("axes.seg.0.major_tick_interval")?;
    replace_focused_text(&mut driver, "0")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_some());
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    // Set valid major tick interval = 5 and minor ticks = 9 via text + stepper.
    driver.click_widget_id("axes.seg.0.major_tick_interval")?;
    replace_focused_text(&mut driver, "5")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_none());

    driver.click_widget_id("axes.seg.0.minor_ticks_per_interval")?;
    replace_focused_text(&mut driver, "8")?;
    driver.click_widget_id("axes.seg.0.minor_ticks.plus")?;

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing graph after commit")?;
    assert_eq!(gs.spec.y_axis.segments[0].major_tick_interval, Some(5.0));
    assert_eq!(gs.spec.y_axis.segments[0].minor_ticks_per_interval, 9);

    // Also verify FormatAxes localized field patch path directly handles major_tick_interval and minor_ticks_per_interval.
    let mut fa = driver.state().app.canvas_dialogs.format_axes.clone();
    fa.sync_from_graph(gs, AxisTarget::X);
    let preset = driver.state().app.locale_preset.clone();
    let patch_set = serde_json::Map::from_iter([
        (String::from("axes.major_tick_interval"), json!(10.0)),
        (String::from("axes.minor_ticks_per_interval"), json!(4)),
    ]);
    curvebook_ui_egui::dialogs_canvas::dialog_patch::patch_format_axes(&mut fa, &patch_set, None, &preset)?;
    assert_eq!(fa.axis_draft.segments[0].major_tick_interval, Some(10.0));
    assert_eq!(fa.axis_draft.segments[0].minor_ticks_per_interval, 4);
    let patch_clear = serde_json::Map::from_iter([(String::from("major_tick_interval"), json!(null))]);
    curvebook_ui_egui::dialogs_canvas::dialog_patch::patch_format_axes(&mut fa, &patch_clear, None, &preset)?;
    assert_eq!(fa.axis_draft.segments[0].major_tick_interval, None);
    Ok(())
}

#[test]
fn gwf_19_transform_modals_transpose_prune_and_select_and_transform() -> Result<(), String> {
    let (mut doc, xy_id, _gid) = make_xy_and_graph_doc().map_err(|e| e.to_string())?;
    let mv_id = SheetId::try_new("ds-mv").map_err(|e| e.to_string())?;
    let mut mv = MultipleVariablesTable::new(3, 4).map_err(|e| e.to_string())?;
    mv.variables[0].title = String::from("Dose");
    mv.variables[0].role = VariableRole::Continuous;
    mv.variables[1].title = String::from("Arm");
    mv.variables[1].role = VariableRole::Categorical;
    mv.variables[2].title = String::from("Response");
    mv.variables[2].role = VariableRole::Continuous;
    let mut mv_ds = DataSheet::new(mv_id.clone(), "MV Cohort", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    for r in 0..4 {
        mv_ds.set_cell(CellCoord::new(r, 0, 0), Cell::number(r as f64 + 1.0)).map_err(|e| e.to_string())?;
        mv_ds.set_cell(CellCoord::new(r, 1, 0), Cell::text(if r % 2 == 0 { "A" } else { "B" })).map_err(|e| e.to_string())?;
        mv_ds.set_cell(CellCoord::new(r, 2, 0), Cell::number(10.0 + (r as f64) * 5.0)).map_err(|e| e.to_string())?;
    }
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&xy_id).map_err(|e| e.to_string())?;

    let app = new_egui_app(doc);
    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

    // 1. Standalone Transpose (`WF-55`): uncheck swap_row_and_column_titles and commit.
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: xy_id.clone(), kind: TransformModalKind::Transpose });
    commit_intents(&mut driver)?;
    assert_modal_parity(&driver, "Transpose modal")?;
    driver.click_widget_id("edit.transform.transpose.swap_titles")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_error, None);

    let transpose_rs = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Transpose { .. })))
        .ok_or("missing Transpose result sheet")?
        .clone();
    assert_eq!(transpose_rs.status, ResultStatus::UpToDate);
    assert_eq!(transpose_rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }));

    // Re-enter via EditAnalysisParameters (`from_edit = true`) and toggle back to true.
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &transpose_rs.id));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
    }
    driver.run_settled();
    driver.click_widget_id("edit.transform.transpose.swap_titles")?;
    driver.click_widget_id("modal.ok")?;
    let updated_t = driver.state().app.document().result_sheet(&transpose_rs.id).ok_or("missing updated transpose")?;
    assert_eq!(updated_t.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true }));

    // 2. Standalone PruneRows (`WF-06`): step K=2 -> K=3, enable average_window, commit.
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: xy_id.clone(), kind: TransformModalKind::PruneRows });
    commit_intents(&mut driver)?;
    assert_modal_parity(&driver, "PruneRows modal")?;
    driver.click_widget_id("edit.transform.prune.every_k.plus")?;
    driver.click_widget_id("edit.transform.prune.average_window")?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_error, None);

    let prune_rs = driver
        .state()
        .app
        .document()
        .result_sheets
        .iter()
        .find(|rs| matches!(rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { .. })))
        .ok_or("missing PruneRows result sheet")?;
    assert_eq!(prune_rs.status, ResultStatus::UpToDate);
    assert_eq!(prune_rs.analysis.kind, AnalysisKind::Transform(TransformAnalysisSpec::PruneRows { every_k_rows: 3, average_window: true }));

    // 3. Standalone MV Filter & Transform: derived column + row filter + column subset + target Grouped (`WF-31`).
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: mv_id.clone(), kind: TransformModalKind::MvFilterAndTransform });
    commit_intents(&mut driver)?;
    assert_modal_parity(&driver, "MvFilterAndTransform modal")?;

    driver.click_widget_id("edit.transform.derived_title")?;
    replace_focused_text(&mut driver, "LogDose")?;
    driver.click_widget_id("edit.transform.formula")?;
    replace_focused_text(&mut driver, "ln([Dose] + 1)")?;
    driver.click_widget_id("edit.transform.add_derived")?;

    driver.click_widget_id("edit.tab.select_rows")?;
    driver.click_widget_id("edit.transform.filter_formula")?;
    replace_focused_text(&mut driver, "[Dose] > 1")?;

    driver.click_widget_id("edit.tab.select_cols")?;
    driver.click_widget_id("edit.transform.col.1")?;

    driver.click_widget_id("edit.tab.target_format")?;
    driver.click_widget_id("edit.transform.target_archetype.grouped")?;

    let rs_count_before = driver.state().app.document().result_sheets.len();
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_error, None);
    let doc_after_mv = driver.state().app.document();
    assert_eq!(doc_after_mv.result_sheets.len(), rs_count_before + 1);
    let mv_rs = doc_after_mv.result_sheets.last().ok_or("missing MV SelectAndTransform result")?;
    assert_eq!(mv_rs.status, ResultStatus::UpToDate);
    assert_eq!(mv_rs.analysis.selected_data_sets, vec![0, 2]);
    match &mv_rs.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns }) => {
            assert_eq!(filter_formula.as_deref(), Some("[Dose] > 1"));
            assert_eq!(*target_archetype, TableKind::Grouped);
            assert_eq!(derived_columns.len(), 1);
            assert_eq!(derived_columns[0].title, "LogDose");
            assert_eq!(derived_columns[0].formula, "ln([Dose] + 1)");
        }
        other => return Err(format!("expected SelectAndTransform, got {other:?}")),
    }

    // 4. Standalone Extract & Rearrange: pick Column target and inspect binding textboxes (`edit.transform.extract.binding_{x,y,grp}`).
    driver.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: mv_id.clone(), kind: TransformModalKind::ExtractAndRearrange });
    commit_intents(&mut driver)?;
    assert_modal_parity(&driver, "ExtractAndRearrange modal")?;
    driver.click_widget_id("edit.transform.target_archetype.column")?;
    driver.click_widget_id("edit.tab.data_arrangement")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.extract.binding_x").is_some());
    assert!(driver.frame_meta().find_by_widget_id("edit.transform.extract.binding_y").is_some());
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_error, None);
    let ext_rs = driver.state().app.document().result_sheets.last().ok_or("missing ExtractAndRearrange result")?;
    assert_eq!(ext_rs.status, ResultStatus::UpToDate);
    assert!(matches!(
        ext_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype: TableKind::Column, .. })
    ));

    // 5. Verify string shorthand `select_and_transform` parses `target_archetype` and `derived_columns` in engine.
    let engine_res = driver
        .state_mut()
        .app
        .execute_command(
            "transform.run",
            json!({
                "source_sheet_id": mv_id.as_str(),
                "kind": "select_and_transform",
                "target_archetype": "Grouped",
                "derived_columns": [{ "title": "Scaled", "formula": "[Response] * 2" }],
            }),
        )
        .map_err(|e| format!("engine select_and_transform failed: {e}"))?;
    assert!(engine_res.payload.get("sheet_id").is_some());
    let shorthand_rs = driver.state().app.document().result_sheets.last().ok_or("missing shorthand result")?;
    match &shorthand_rs.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, derived_columns, .. }) => {
            assert_eq!(*target_archetype, TableKind::Grouped);
            assert_eq!(derived_columns.len(), 1);
            assert_eq!(derived_columns[0].title, "Scaled");
        }
        other => return Err(format!("expected SelectAndTransform shorthand, got {other:?}")),
    }

    Ok(())
}

#[test]
fn gwf_20_stats_modal_r7_correlation_sphericity_and_two_group_diagnostics() -> Result<(), String> {
    let mut doc = ProjectDocument::new("GWF-20 Stats Modals").map_err(|e| e.to_string())?;
    let col_id = SheetId::try_new("col-rm").map_err(|e| e.to_string())?;
    let mut tbl = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).map_err(|e| e.to_string())?;
    tbl.data_sets[0].title = String::from("Ctrl");
    tbl.data_sets[1].title = String::from("Mid");
    tbl.data_sets[2].title = String::from("High");
    for r in 0..5 {
        let b = (r + 1) as f64;
        tbl.data_sets[0].cells[r][0] = Cell::number(10.0 + b);
        tbl.data_sets[1].cells[r][0] = Cell::number(12.0 + b * 1.4);
        tbl.data_sets[2].cells[r][0] = Cell::number(15.0 + b * 1.8);
    }
    doc.add_data_sheet(DataSheet::new(col_id.clone(), "Paired Table", TableArchetype::Column(tbl)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.select_in_navigator(&col_id).map_err(|e| e.to_string())?;

    let mut app = new_egui_app(doc);
    let doc_clone = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc_clone, &col_id));
    app.sync_views();

    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    driver.click_widget_id("analyze.search")?;
    replace_focused_text(&mut driver, "Correlation Matrix")?;
    driver.click_widget_id("analyze.entry.stats.corr_matrix")?;
    assert_modal_parity(&driver, "AnalyzeSheet Correlation")?;

    // R7 in AnalyzeSheet: heatmap checkbox enabled when all_pairs=true, disabled when unchecked.
    assert!(driver.frame_meta().find_by_widget_id("analyze.correlation_all_pairs").is_some_and(|e| e.enabled));
    assert!(driver.frame_meta().find_by_widget_id("analyze.correlation.heatmap").is_some_and(|e| e.enabled));
    let before_heat = driver.state().app.canvas_dialogs.analyze.correlation_heatmap;
    driver.click_widget_id("analyze.correlation.heatmap")?;
    assert_eq!(driver.state().app.canvas_dialogs.analyze.correlation_heatmap, !before_heat);
    driver.click_widget_id("analyze.correlation_all_pairs")?;
    assert!(!driver.state().app.canvas_dialogs.analyze.correlation_all_pairs);
    assert!(driver.frame_meta().find_by_widget_id("analyze.correlation.heatmap").is_some_and(|e| !e.enabled));
    driver.click_widget_id("analyze.correlation_all_pairs")?;
    assert!(driver.frame_meta().find_by_widget_id("analyze.correlation.heatmap").is_some_and(|e| e.enabled));
    driver.click_widget_id("modal.cancel")?;

    // Run CorrelationMatrix via engine and verify EditAnalysisParameters R7 checkboxes (`edit.corr.*`).
    let corr_res = driver
        .state_mut()
        .app
        .execute_command(
            "stats.run",
            json!({
                "source_sheet_id": col_id.as_str(),
                "kind": AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }),
            }),
        )
        .map_err(|e| e.to_string())?;
    let corr_rs_id = SheetId::try_new(corr_res.payload["sheet_id"].as_str().ok_or("missing corr sheet_id")?).map_err(|e| e.to_string())?;
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &corr_rs_id));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
    }
    driver.run_settled();
    assert_modal_parity(&driver, "EditParams CorrelationMatrix")?;
    assert!(driver.frame_meta().find_by_widget_id("edit.corr.all_pairs").is_some_and(|e| e.enabled));
    assert!(driver.frame_meta().find_by_widget_id("edit.corr.heatmap").is_some_and(|e| e.enabled));
    let ep_heat0 = driver.state().app.canvas_dialogs.edit_params.correlation_heatmap;
    driver.click_widget_id("edit.corr.heatmap")?;
    assert_eq!(driver.state().app.canvas_dialogs.edit_params.correlation_heatmap, !ep_heat0);
    driver.click_widget_id("edit.corr.all_pairs")?;
    assert!(!driver.state().app.canvas_dialogs.edit_params.correlation_all_pairs);
    assert!(driver.frame_meta().find_by_widget_id("edit.corr.heatmap").is_some_and(|e| !e.enabled));
    driver.click_widget_id("modal.cancel")?;

    // Two-Group comparison diagnostics (`edit.diag.qq`, `edit.diag.shapiro`) via stat_params modal.
    let t_res = driver
        .state_mut()
        .app
        .execute_command(
            "stats.run",
            json!({
                "source_sheet_id": col_id.as_str(),
                "selected_data_sets": [0, 1],
                "kind": AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
            }),
        )
        .map_err(|e| e.to_string())?;
    let t_rs_id = SheetId::try_new(t_res.payload["sheet_id"].as_str().ok_or("missing two-group sheet_id")?).map_err(|e| e.to_string())?;
    {
        let doc_snap = driver.state().app.document().clone();
        assert!(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc_snap, &t_rs_id));
        driver.state_mut().app.sync_views();
        driver.state_mut().views = driver.state().app.egui_views.clone();
    }
    driver.run_settled();
    driver.click_widget_id("edit.tab.diagnostics")?;
    assert_modal_parity(&driver, "EditParams TwoGroup Diagnostics")?;
    let qq0 = driver.state().app.canvas_dialogs.edit_params.diag_qq_plot;
    let chk0 = driver.state().app.canvas_dialogs.edit_params.diag_assumption_checklist;
    driver.click_widget_id("edit.diag.qq")?;
    driver.click_widget_id("edit.diag.shapiro")?;
    assert_eq!(driver.state().app.canvas_dialogs.edit_params.diag_qq_plot, !qq0);
    assert_eq!(driver.state().app.canvas_dialogs.edit_params.diag_assumption_checklist, !chk0);

    // Welch t-test disables Two-Group Diagnostics tab (`edit.tab.diagnostics`) per R4.
    driver.click_widget_id("edit.tab.experimental_design")?;
    if let Some(mut spec) = driver.state().app.canvas_dialogs.edit_params.draft.clone() {
        spec.kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() });
        driver.state_mut().intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
        commit_intents(&mut driver)?;
    }
    assert!(driver.frame_meta().find_by_widget_id("edit.tab.diagnostics").is_some_and(|e| !e.enabled));
    driver.click_widget_id("modal.cancel")?;

    // RM ANOVA Geisser-Greenhouse via `render_stat_params_modal` directly (ensuring stat_params.rs Options tab wiring is exercised).
    let anova_res = driver
        .state_mut()
        .app
        .execute_command(
            "stats.run",
            json!({
                "source_sheet_id": col_id.as_str(),
                "kind": AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                    method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
                    ..OneWayAnovaSpec::default()
                }),
            }),
        )
        .map_err(|e| e.to_string())?;
    let anova_rs_id = SheetId::try_new(anova_res.payload["sheet_id"].as_str().ok_or("missing anova sheet_id")?).map_err(|e| e.to_string())?;
    let mut rm_app = driver.state().app.clone();
    let rm_doc = rm_app.document().clone();
    assert!(rm_app.canvas_dialogs.open_edit_parameters(&rm_doc, &anova_rs_id));
    rm_app.sync_views();
    let mut sp_driver = EguiHumanPathDriver::new(TestState::new(rm_app), ViewportMetrics::new(1280.0, 800.0, 1.0), |ui, meta, st| {
        let tokens = Tokens::get(ui.ctx());
        let _ = render_stat_params_modal(ui.ctx(), &st.app, &mut st.views.stats_modals, &tokens, meta, &mut st.intents);
    });
    sp_driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    sp_driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    sp_driver.run_settled();
    sp_driver.click_widget_id("edit.tab.options")?;
    assert!(sp_driver.frame_meta().find_by_widget_id("edit.sphericity.geisser_greenhouse").is_some_and(|e| e.enabled));
    sp_driver.click_widget_id("edit.sphericity.geisser_greenhouse")?;
    let gg_after = sp_driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind).is_some_and(
        |k| matches!(k, AnalysisKind::OneWayAnova(a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })),
    );
    assert!(gg_after, "clicking edit.sphericity.geisser_greenhouse must toggle GG to false");

    // When unpaired (`paired_or_matched = false`), R1b must disable edit.sphericity.geisser_greenhouse.
    sp_driver.state_mut().app.canvas_dialogs.edit_params.paired_or_matched = false;
    sp_driver.run_settled();
    assert!(sp_driver.frame_meta().find_by_widget_id("edit.sphericity.geisser_greenhouse").is_some_and(|e| !e.enabled));
    Ok(())
}

struct HeatmapPrimitives {
    tile_sizes: Vec<(f64, f64)>,
    texts: Vec<(String, Rgba8)>,
}

fn collect_tile_rects_and_texts(scene: &ChartScene) -> HeatmapPrimitives {
    let mut tile_sizes = Vec::new();
    let mut texts = Vec::new();
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, fill: Some(_), stroke: Some(_), .. } => {
                if let [PathCmd::MoveTo(p0), PathCmd::LineTo(p1), PathCmd::LineTo(_), PathCmd::LineTo(p3), PathCmd::Close] = path.commands.as_slice()
                {
                    let w = (p1.x - p0.x).abs();
                    let h = (p3.y - p0.y).abs();
                    if w > 5.0 && h > 5.0 && w < 200.0 && h < 200.0 {
                        tile_sizes.push((w, h));
                    }
                }
            }
            ScenePrimitive::Text(pt) => {
                texts.push((pt.rich.plain_text(), pt.color));
            }
            _ => {}
        }
    }
    HeatmapPrimitives { tile_sizes, texts }
}

#[test]
fn gwf_21_heatmap_annotations_cell_gap_and_wf60_xy_column_copy_paste() -> Result<(), String> {
    // Part A (`WF-45`): Heatmap annotations + cell_gap_pt committed & rendered in ChartScene.
    let mut doc = ProjectDocument::new("GWF-21 Heatmap").map_err(|e| e.to_string())?;
    let mv_id = SheetId::try_new("mv-hm").map_err(|e| e.to_string())?;
    let mut mv = MultipleVariablesTable::new(3, 3).map_err(|e| e.to_string())?;
    for c in 0..3 {
        mv.variables[c].title = format!("V{}", c + 1);
        mv.variables[c].role = VariableRole::Continuous;
    }
    let mut mv_ds = DataSheet::new(mv_id, "Heatmap Data", TableArchetype::MultipleVariables(mv)).map_err(|e| e.to_string())?;
    for r in 0..3 {
        for c in 0..3 {
            mv_ds.set_cell(CellCoord::new(r, c, 0), Cell::number((r * 3 + c + 1) as f64 * 10.0)).map_err(|e| e.to_string())?;
        }
    }
    let hm_gid = SheetId::try_new("gs-hm").map_err(|e| e.to_string())?;
    let mut hm_graph = GraphSheet::new_uninitialized_for_data(hm_gid.clone(), &mv_ds).map_err(|e| e.to_string())?;
    hm_graph.initialized = true;
    hm_graph.spec.kind = GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap);
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(hm_graph).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&hm_gid).map_err(|e| e.to_string())?;

    let base_scene = compile_graph(doc.graph_sheet(&hm_gid).ok_or("missing hm graph")?, &doc).map_err(|e| e.to_string())?;
    let HeatmapPrimitives { tile_sizes: base_tiles, texts: base_texts } = collect_tile_rects_and_texts(&base_scene);
    assert!(!base_tiles.is_empty(), "heatmap must emit cell tile rectangles");

    let mut app = new_egui_app(doc);
    let doc_snap = app.document().clone();
    assert!(app.canvas_dialogs.open_format_graph(&doc_snap, &hm_gid));
    app.sync_views();

    let mut driver = make_driver(app, ViewportMetrics::new(1280.0, 800.0, 1.0));
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    assert_modal_parity(&driver, "FormatGraph Heatmap Labels")?;
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    assert!(driver.state().app.canvas_dialogs.format_graph.heatmap_labels.annotate_cells);

    driver.click_widget_id("edit.tab.format_graph.heatmap_borders")?;
    let gap_rect = driver.frame_meta().find_by_widget_id("format_graph.heatmap.cell_gap").ok_or("missing cell_gap")?.clipped_rect;
    let cy = gap_rect.center().y;
    driver.drag(Pos2::new(gap_rect.min.x + 8.0, cy), Pos2::new(gap_rect.min.x + 90.0, cy))?;
    assert!(driver.state().app.canvas_dialogs.format_graph.heatmap_labels.cell_gap_pt > 0.0, "dragging cell_gap must set gap > 0");

    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_error, None);
    let doc_hm = driver.state().app.document();
    let hm_after = doc_hm.graph_sheet(&hm_gid).ok_or("missing committed heatmap")?;
    assert!(hm_after.spec.heatmap_labels.annotate_cells);
    assert!(hm_after.spec.heatmap_labels.cell_gap_pt > 0.0);

    let after_scene = compile_graph(hm_after, doc_hm).map_err(|e| e.to_string())?;
    let HeatmapPrimitives { tile_sizes: after_tiles, texts: after_texts } = collect_tile_rects_and_texts(&after_scene);
    assert_eq!(after_tiles.len(), base_tiles.len());
    assert!(
        after_tiles[0].0 < base_tiles[0].0 - 0.1 && after_tiles[0].1 < base_tiles[0].1 - 0.1,
        "tile gap must inset rendered tiles: before={:?}, after={:?}",
        base_tiles[0],
        after_tiles[0]
    );
    assert!(after_texts.len() >= base_texts.len() + 9, "expected 9 numeric cell annotation text primitives");
    assert!(after_texts.iter().any(|(t, c)| t == "10" && (*c == Rgba8::WHITE || *c == Rgba8::BLACK)));
    assert!(after_texts.iter().any(|(t, c)| t == "90" && (*c == Rgba8::WHITE || *c == Rgba8::BLACK)));

    // Part B (`WF-60`): Verify multi-row Y column Cmd+C -> Cmd+V on an XY sheet (both 2-column and 1-column cases).
    let (xy_doc_2col, xy_id, _) = make_xy_and_graph_doc().map_err(|e| e.to_string())?;
    let mut xy_app = new_egui_app(xy_doc_2col);
    xy_app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    let _ = xy_app.execute_command("sheet.select", json!({ "sheet_id": xy_id.as_str() })).map_err(|e| e.to_string())?;
    let mut grid_driver = make_driver(xy_app, ViewportMetrics::new(1280.0, 800.0, 1.0));

    // Select multi-row Y1 (`grid.cell.0.0.0` .. `grid.cell.3.0.0`) with Shift+Click, Cmd+C, click Y2 (`grid.cell.0.1.0`), Cmd+V.
    grid_driver.click_widget_id("grid.cell.0.0.0")?;
    let y1_end = grid_driver.frame_meta().find_by_widget_id("grid.cell.3.0.0").ok_or("missing grid.cell.3.0.0")?.clipped_rect.center();
    grid_driver.click_at_button(y1_end, PointerButton::Primary, KeyModifiers { shift: true, ..KeyModifiers::default() })?;
    grid_driver.press_egui_key(Key::C, Modifiers::COMMAND);
    grid_driver.click_widget_id("grid.cell.0.1.0")?;
    grid_driver.press_egui_key(Key::V, Modifiers::COMMAND);

    let ds_after = grid_driver.state().app.document().data_sheet(&xy_id).ok_or("missing xy after copy/paste")?;
    let copied_all_4_rows = (0..4).all(|r| {
        ds_after.cell(CellCoord::new(r, 1, 0)).and_then(|c| c.active_number())
            == ds_after.cell(CellCoord::new(r, 0, 0)).and_then(|c| c.active_number())
    });
    assert!(copied_all_4_rows, "multi-row Y1 Cmd+C -> Y2 Cmd+V must copy all 4 rows cleanly");

    // Part C (`WF-60`): Duplicate Column context-menu action on a 1-Y-column XY sheet with linked GraphSheet.
    let mut single_doc = ProjectDocument::new("WF-60 Duplicate Column").map_err(|e| e.to_string())?;
    let mut xy1 = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 4)
        .map_err(|e| e.to_string())?;
    xy1.data_sets[0].title = String::from("RawY");
    for r in 0..4 {
        xy1.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy1.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
    }
    let xy1_id = SheetId::try_new("ds-xy1").map_err(|e| e.to_string())?;
    let ds1 = DataSheet::new(xy1_id.clone(), "XY 1Col", TableArchetype::Xy(xy1)).map_err(|e| e.to_string())?;
    let gid1 = SheetId::try_new("gs-xy1").map_err(|e| e.to_string())?;
    let mut gs1 = GraphSheet::new_uninitialized_for_data(gid1.clone(), &ds1).map_err(|e| e.to_string())?;
    gs1.initialized = true;
    gs1.spec.kind = GraphKind::Xy;
    assert_eq!(gs1.spec.series.len(), 1);
    single_doc.add_data_sheet(ds1).map_err(|e| e.to_string())?;
    single_doc.add_graph_sheet(gs1).map_err(|e| e.to_string())?;
    single_doc.select_in_navigator(&xy1_id).map_err(|e| e.to_string())?;

    let mut app1 = new_egui_app(single_doc);
    app1.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    let mut drv1 = make_driver(app1, ViewportMetrics::new(1280.0, 800.0, 1.0));

    // Right-clicking X cell renders `grid.ctx.duplicate_column` disabled; dismiss popup before right-clicking Y cell.
    let x_pos = drv1.frame_meta().find_by_widget_id("grid.xcell.0.0").ok_or("missing grid.xcell.0.0")?.clipped_rect.center();
    drv1.click_at_button(x_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(drv1.frame_meta().find_by_widget_id("grid.ctx.duplicate_column").is_some_and(|e| !e.enabled));
    drv1.press_key("Escape", KeyModifiers::default())?;
    assert!(drv1.frame_meta().find_by_widget_id("grid.ctx.duplicate_column").is_none());

    // Right-clicking Y data cell enables `grid.ctx.duplicate_column` and duplicates col 0 -> col 1 + updates linked graph series.
    let y_pos = drv1.frame_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing 1col Y cell")?.clipped_rect.center();
    drv1.click_at_button(y_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert!(drv1.frame_meta().find_by_widget_id("grid.ctx.duplicate_column").is_some_and(|e| e.enabled));
    drv1.click_widget_id("grid.ctx.duplicate_column")?;
    assert_eq!(drv1.state().last_error, None);

    let ds1_after = drv1.state().app.document().data_sheet(&xy1_id).ok_or("missing xy1 after duplicate")?;
    assert_eq!(ds1_after.archetype.column_count(), 2);
    let TableArchetype::Xy(xy_after) = &ds1_after.archetype else { return Err(String::from("expected Xy table")) };
    assert_eq!(xy_after.data_sets[1].title, "RawY (Copy)");
    for r in 0..4 {
        assert_eq!(xy_after.data_sets[1].cells[r][0].active_number(), Some((r + 1) as f64 * 10.0));
    }
    let gs1_after = drv1.state().app.document().graph_sheet(&gid1).ok_or("missing linked graph after duplicate")?;
    assert_eq!(gs1_after.spec.series.len(), 2);
    assert_eq!(gs1_after.spec.series[1].data_set_index, 1);
    Ok(())
}
