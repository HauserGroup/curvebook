#![forbid(unsafe_code)]

//! Targeted `EguiHumanPathDriver` regression suite for `GWF-18` through `GWF-21`
//! (non-Format-Graph GUI gap fixes + Heatmap persistence + `WF-60` XY copy/paste).

use std::sync::Arc;

use crate::common::{
    ShellTestState, TestRes, assert_modal_parity, cell_number, commit_shell_intents, seed_column_doc, seed_xy_doc, set_textbox, sync_app_to_driver,
    widget_center, widget_rect,
};
use crate::suite_helpers::{
    add_init_graph, assert_widget_enabled, assert_widgets, build_mv_sheet, click_all, make_human_driver, make_sync_driver, new_human_app,
    open_edit_params, open_format_graph, run_sheet_cmd,
};
use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, DiagnosticPlotKind, GraphKind, MultivariateSpec, OneWayAnovaSpec, OneWayMethod, ProjectDocument,
    ResultStatus, SheetId, TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole,
};
use curvebook_palette::Rgba8;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AxisTarget};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use curvebook_ui_egui::egui_shell::stats_modals::render_stat_params_modal;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::egui_shell::{Tokens, UiIntent};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use egui::{Key, Modifiers, Pos2};
use serde_json::json;

fn make_xy_and_graph_doc() -> Result<(ProjectDocument, SheetId, SheetId), String> {
    let (mut doc, xy_id) = seed_xy_doc(
        "GWF Gap Sweep Doc",
        "ds-xy",
        "XY Table",
        "Time (min)",
        &[10.0, 20.0, 30.0, 40.0],
        &[("Y1", &[20.0, 40.0, 60.0, 80.0]), ("Y2", &[5.0, 10.0, 15.0, 20.0])],
    )?;
    let gid = add_init_graph(&mut doc, &xy_id, "gs-xy", GraphKind::Xy)?;
    doc.select_in_navigator(&gid).map_err(|e| e.to_string())?;
    Ok((doc, xy_id, gid))
}

#[test]
fn gwf_18_format_axes_major_tick_interval_and_minor_ticks() -> TestRes {
    let (doc, _xy_id, gid) = make_xy_and_graph_doc()?;
    let mut app = new_human_app(doc);
    let gs = app.document().graph_sheet(&gid).ok_or("missing graph")?.clone();
    app.canvas_dialogs.format_axes.sync_from_graph(&gs, AxisTarget::Y);
    app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gid.clone() };
    app.sync_views();

    let mut driver = make_sync_driver(app, 1280.0, 800.0);
    driver.click_widget_id("edit.tab.axes.y")?;
    assert_widgets(&driver, &["axes.seg.0.major_tick_interval", "axes.seg.0.minor_ticks_per_interval"])?;
    assert_modal_parity(&driver, "FormatAxes initial")?;

    // Invalid interval (<= 0) displays error and blocks commit while keeping FormatAxes open.
    set_textbox(&mut driver, "axes.seg.0.major_tick_interval", "0")?;
    assert_widgets(&driver, &["axes.seg.0.ticks_error"])?;
    driver.click_widget_id("modal.ok")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));

    // Valid major tick interval = 5 and minor ticks = 9 via text + stepper.
    set_textbox(&mut driver, "axes.seg.0.major_tick_interval", "5")?;
    assert!(driver.frame_meta().find_by_widget_id("axes.seg.0.ticks_error").is_none());
    set_textbox(&mut driver, "axes.seg.0.minor_ticks_per_interval", "8")?;
    click_all(&mut driver, &["axes.seg.0.minor_ticks.plus", "modal.ok"])?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    let gs = driver.state().app.document().graph_sheet(&gid).ok_or("missing graph after commit")?;
    assert_eq!((gs.spec.y_axis.segments[0].major_tick_interval, gs.spec.y_axis.segments[0].minor_ticks_per_interval), (Some(5.0), 9));
    Ok(())
}

#[test]
fn gwf_19_transform_modals_transpose_prune_and_select_and_transform() -> TestRes {
    let (mut doc, xy_id, _gid) = make_xy_and_graph_doc()?;
    let mv_ds = build_mv_sheet(
        "ds-mv",
        "MV Cohort",
        &[("Dose", VariableRole::Continuous), ("Arm", VariableRole::Categorical), ("Response", VariableRole::Continuous)],
        4,
        |mv| {
            for r in 0..4 {
                mv.variables[0].cells[r] = Cell::number(r as f64 + 1.0);
                mv.variables[1].cells[r] = Cell::text(if r % 2 == 0 { "A" } else { "B" });
                mv.variables[2].cells[r] = Cell::number(10.0 + (r as f64) * 5.0);
            }
        },
    )?;
    let mv_id = mv_ds.id.clone();
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&xy_id).map_err(|e| e.to_string())?;
    let mut driver = make_human_driver(doc, 1280.0, 800.0);

    let open_xform = |drv: &mut EguiHumanPathDriver<'_, ShellTestState>, sid: &SheetId, kind: TransformModalKind, label: &str| -> TestRes {
        drv.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: sid.clone(), kind });
        commit_shell_intents(drv)?;
        assert_modal_parity(drv, label)
    };

    // 1. Standalone Transpose (`WF-55`) + Cmd+T re-entry.
    open_xform(&mut driver, &xy_id, TransformModalKind::Transpose, "Transpose modal")?;
    click_all(&mut driver, &["edit.transform.transpose.swap_titles", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
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

    open_edit_params(&mut driver, &transpose_rs.id)?;
    click_all(&mut driver, &["edit.transform.transpose.swap_titles", "modal.ok"])?;
    assert_eq!(
        driver.state().app.document().result_sheet(&transpose_rs.id).ok_or("missing updated transpose")?.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: true })
    );

    // 2. Standalone PruneRows (`WF-06`): step K=2 -> K=3, enable average_window, commit.
    open_xform(&mut driver, &xy_id, TransformModalKind::PruneRows, "PruneRows modal")?;
    click_all(&mut driver, &["edit.transform.prune.every_k.plus", "edit.transform.prune.average_window", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
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

    // 3. Standalone MV Filter & Transform (`WF-31`).
    open_xform(&mut driver, &mv_id, TransformModalKind::MvFilterAndTransform, "MvFilterAndTransform modal")?;
    set_textbox(&mut driver, "edit.transform.derived_title", "LogDose")?;
    set_textbox(&mut driver, "edit.transform.formula", "ln([Dose] + 1)")?;
    click_all(&mut driver, &["edit.transform.add_derived", "edit.tab.select_rows"])?;
    set_textbox(&mut driver, "edit.transform.filter_formula", "[Dose] > 1")?;
    click_all(&mut driver, &["edit.tab.select_cols", "edit.transform.col.1", "edit.tab.target_format", "edit.transform.target_archetype.grouped"])?;
    let rs_before = driver.state().app.document().result_sheets.len();
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    assert_eq!(driver.state().app.document().result_sheets.len(), rs_before + 1);
    let mv_rs = driver.state().app.document().result_sheets.last().ok_or("missing MV SelectAndTransform result")?;
    assert_eq!((mv_rs.status.clone(), mv_rs.analysis.selected_data_sets.clone()), (ResultStatus::UpToDate, vec![0, 2]));
    match &mv_rs.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns }) => {
            assert_eq!((filter_formula.as_deref(), *target_archetype, derived_columns.len()), (Some("[Dose] > 1"), TableKind::Grouped, 1));
            assert_eq!((derived_columns[0].title.as_str(), derived_columns[0].formula.as_str()), ("LogDose", "ln([Dose] + 1)"));
        }
        other => return Err(format!("expected SelectAndTransform, got {other:?}")),
    }

    // 4. Standalone Extract & Rearrange.
    open_xform(&mut driver, &mv_id, TransformModalKind::ExtractAndRearrange, "ExtractAndRearrange modal")?;
    click_all(&mut driver, &["edit.transform.target_archetype.column", "edit.tab.data_arrangement"])?;
    assert_widgets(&driver, &["edit.transform.extract.binding_x", "edit.transform.extract.binding_y"])?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    let ext_rs = driver.state().app.document().result_sheets.last().ok_or("missing ExtractAndRearrange result")?;
    assert_eq!(ext_rs.status, ResultStatus::UpToDate);
    assert!(matches!(
        ext_rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype: TableKind::Column, .. })
    ));

    // 5. Engine string shorthand `select_and_transform` parses `target_archetype` & `derived_columns`.
    let _ = run_sheet_cmd(
        &mut driver.state_mut().app,
        "transform.run",
        json!({
            "source_sheet_id": mv_id.as_str(),
            "kind": "select_and_transform",
            "target_archetype": "Grouped",
            "derived_columns": [{ "title": "Scaled", "formula": "[Response] * 2" }],
        }),
    )?;
    match &driver.state().app.document().result_sheets.last().ok_or("missing shorthand result")?.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, derived_columns, .. }) => {
            assert_eq!((*target_archetype, derived_columns.len(), derived_columns[0].title.as_str()), (TableKind::Grouped, 1, "Scaled"));
        }
        other => return Err(format!("expected SelectAndTransform shorthand, got {other:?}")),
    }
    Ok(())
}

#[test]
fn gwf_20_stats_modal_r7_correlation_sphericity_and_two_group_diagnostics() -> TestRes {
    let (doc, col_id) = seed_column_doc(
        "GWF-20 Stats Modals",
        "col-rm",
        "Paired Table",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("Ctrl", &[11.0, 12.0, 13.0, 14.0, 15.0]), ("Mid", &[13.4, 14.8, 16.2, 17.6, 19.0]), ("High", &[16.8, 18.6, 20.4, 22.2, 24.0])],
    )?;
    let mut app = new_human_app(doc);
    let doc_clone = app.document().clone();
    assert!(app.canvas_dialogs.open_analyze_dialog(&doc_clone, &col_id));
    app.sync_views();
    let mut driver = make_sync_driver(app, 1280.0, 800.0);

    // R7 in AnalyzeSheet: heatmap checkbox enabled when all_pairs=true, disabled when unchecked.
    set_textbox(&mut driver, "analyze.search", "Correlation Matrix")?;
    driver.click_widget_id("analyze.entry.stats.corr_matrix")?;
    assert_modal_parity(&driver, "AnalyzeSheet Correlation")?;
    assert_widget_enabled(&driver, "analyze.correlation_all_pairs", true)?;
    assert_widget_enabled(&driver, "analyze.correlation.heatmap", true)?;
    let before_heat = driver.state().app.canvas_dialogs.analyze.correlation_heatmap;
    driver.click_widget_id("analyze.correlation.heatmap")?;
    assert_eq!(driver.state().app.canvas_dialogs.analyze.correlation_heatmap, !before_heat);
    driver.click_widget_id("analyze.correlation_all_pairs")?;
    assert!(!driver.state().app.canvas_dialogs.analyze.correlation_all_pairs);
    assert_widget_enabled(&driver, "analyze.correlation.heatmap", false)?;
    driver.click_widget_id("analyze.correlation_all_pairs")?;
    assert_widget_enabled(&driver, "analyze.correlation.heatmap", true)?;
    driver.click_widget_id("modal.cancel")?;

    // CorrelationMatrix EditAnalysisParameters R7 checkboxes (`edit.corr.*`).
    let corr_rs_id = run_sheet_cmd(
        &mut driver.state_mut().app,
        "stats.run",
        json!({ "source_sheet_id": col_id.as_str(), "kind": AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: false }) }),
    )?;
    open_edit_params(&mut driver, &corr_rs_id)?;
    assert_modal_parity(&driver, "EditParams CorrelationMatrix")?;
    assert_widget_enabled(&driver, "edit.corr.all_pairs", true)?;
    assert_widget_enabled(&driver, "edit.corr.heatmap", true)?;
    let ep_heat0 = driver.state().app.canvas_dialogs.edit_params.correlation_heatmap;
    driver.click_widget_id("edit.corr.heatmap")?;
    assert_eq!(driver.state().app.canvas_dialogs.edit_params.correlation_heatmap, !ep_heat0);
    driver.click_widget_id("edit.corr.all_pairs")?;
    assert!(!driver.state().app.canvas_dialogs.edit_params.correlation_all_pairs);
    assert_widget_enabled(&driver, "edit.corr.heatmap", false)?;
    driver.click_widget_id("modal.cancel")?;

    // Two-Group comparison diagnostics (`edit.diag.qq`, `edit.diag.shapiro`) + Welch R4 tab gate.
    let t_rs_id = run_sheet_cmd(
        &mut driver.state_mut().app,
        "stats.run",
        json!({
            "source_sheet_id": col_id.as_str(),
            "selected_data_sets": [0, 1],
            "kind": AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, ..TwoGroupSpec::default() }),
        }),
    )?;
    open_edit_params(&mut driver, &t_rs_id)?;
    driver.click_widget_id("edit.tab.diagnostics")?;
    assert_modal_parity(&driver, "EditParams TwoGroup Diagnostics")?;
    let (qq0, chk0) =
        (driver.state().app.canvas_dialogs.edit_params.diag_qq_plot, driver.state().app.canvas_dialogs.edit_params.diag_assumption_checklist);
    click_all(&mut driver, &["edit.diag.qq", "edit.diag.shapiro"])?;
    assert_eq!(
        (driver.state().app.canvas_dialogs.edit_params.diag_qq_plot, driver.state().app.canvas_dialogs.edit_params.diag_assumption_checklist),
        (!qq0, !chk0)
    );
    driver.click_widget_id("edit.tab.experimental_design")?;
    if let Some(mut spec) = driver.state().app.canvas_dialogs.edit_params.draft.clone() {
        spec.kind = AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() });
        driver.state_mut().intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(spec) });
        commit_shell_intents(&mut driver)?;
    }
    assert_widget_enabled(&driver, "edit.tab.diagnostics", false)?;
    driver.click_widget_id("modal.cancel")?;

    // RM ANOVA Geisser-Greenhouse via `render_stat_params_modal` directly (stat_params.rs Options tab).
    let anova_rs_id = run_sheet_cmd(
        &mut driver.state_mut().app,
        "stats.run",
        json!({
            "source_sheet_id": col_id.as_str(),
            "kind": AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }, ..OneWayAnovaSpec::default() }),
        }),
    )?;
    let mut rm_app = driver.state().app.clone();
    let rm_doc = rm_app.document().clone();
    assert!(rm_app.canvas_dialogs.open_edit_parameters(&rm_doc, &anova_rs_id));
    rm_app.sync_views();
    let mut sp_driver = EguiHumanPathDriver::new(ShellTestState::new(rm_app), ViewportMetrics::new(1280.0, 800.0, 1.0), |ui, meta, st| {
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
    assert_widget_enabled(&sp_driver, "edit.sphericity.geisser_greenhouse", true)?;
    sp_driver.click_widget_id("edit.sphericity.geisser_greenhouse")?;
    assert!(sp_driver.state().app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind).is_some_and(
        |k| matches!(k, AnalysisKind::OneWayAnova(a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })),
    ));
    sp_driver.state_mut().app.canvas_dialogs.edit_params.paired_or_matched = false;
    sync_app_to_driver(&mut sp_driver);
    assert_widget_enabled(&sp_driver, "edit.sphericity.geisser_greenhouse", false)
}

type HeatmapPrimitives = (Vec<(f64, f64)>, Vec<(String, Rgba8)>);

fn collect_tile_rects_and_texts(scene: &ChartScene) -> HeatmapPrimitives {
    let (mut tiles, mut texts) = (Vec::new(), Vec::new());
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, fill: Some(_), stroke: Some(_), .. } => {
                if let [PathCmd::MoveTo(p0), PathCmd::LineTo(p1), PathCmd::LineTo(_), PathCmd::LineTo(p3), PathCmd::Close] = path.commands.as_slice()
                {
                    let (w, h) = ((p1.x - p0.x).abs(), (p3.y - p0.y).abs());
                    if w > 5.0 && h > 5.0 && w < 200.0 && h < 200.0 {
                        tiles.push((w, h));
                    }
                }
            }
            ScenePrimitive::Text(pt) => texts.push((pt.rich.plain_text(), pt.color)),
            _ => {}
        }
    }
    (tiles, texts)
}

#[test]
fn gwf_21_heatmap_annotations_cell_gap_and_wf60_xy_column_copy_paste() -> TestRes {
    // Part A (`WF-45`): Heatmap annotations + cell_gap_pt committed & rendered in ChartScene.
    let mut doc = ProjectDocument::new("GWF-21 Heatmap").map_err(|e| e.to_string())?;
    let mv_ds = build_mv_sheet(
        "mv-hm",
        "Heatmap Data",
        &[("V1", VariableRole::Continuous), ("V2", VariableRole::Continuous), ("V3", VariableRole::Continuous)],
        3,
        |mv| {
            for r in 0..3 {
                for c in 0..3 {
                    mv.variables[c].cells[r] = Cell::number((r * 3 + c + 1) as f64 * 10.0);
                }
            }
        },
    )?;
    let mv_id = mv_ds.id.clone();
    doc.add_data_sheet(mv_ds).map_err(|e| e.to_string())?;
    let hm_gid = add_init_graph(&mut doc, &mv_id, "gs-hm", GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))?;
    doc.select_in_navigator(&hm_gid).map_err(|e| e.to_string())?;

    let base_scene = compile_graph(doc.graph_sheet(&hm_gid).ok_or("missing hm graph")?, &doc).map_err(|e| e.to_string())?;
    let (base_tiles, base_texts) = collect_tile_rects_and_texts(&base_scene);
    assert!(!base_tiles.is_empty(), "heatmap must emit cell tile rectangles");

    let mut driver = make_human_driver(doc, 1280.0, 800.0);
    open_format_graph(&mut driver, &hm_gid)?;
    driver.click_widget_id("edit.tab.format_graph.heatmap_labels")?;
    assert_modal_parity(&driver, "FormatGraph Heatmap Labels")?;
    driver.click_widget_id("format_graph.heatmap.annotate_cells")?;
    assert!(driver.state().app.canvas_dialogs.format_graph.heatmap_labels.annotate_cells);
    driver.click_widget_id("edit.tab.format_graph.heatmap_borders")?;
    let gap_rect = widget_rect(&driver, "format_graph.heatmap.cell_gap")?;
    driver.drag(Pos2::new(gap_rect.min.x + 8.0, gap_rect.center().y), Pos2::new(gap_rect.min.x + 90.0, gap_rect.center().y))?;
    assert!(driver.state().app.canvas_dialogs.format_graph.heatmap_labels.cell_gap_pt > 0.0);
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);

    let doc_hm = driver.state().app.document();
    let hm_after = doc_hm.graph_sheet(&hm_gid).ok_or("missing committed heatmap")?;
    assert!(hm_after.spec.heatmap_labels.annotate_cells && hm_after.spec.heatmap_labels.cell_gap_pt > 0.0);
    let (after_tiles, after_texts) = collect_tile_rects_and_texts(&compile_graph(hm_after, doc_hm).map_err(|e| e.to_string())?);
    assert_eq!(after_tiles.len(), base_tiles.len());
    assert!(after_tiles[0].0 < base_tiles[0].0 - 0.1 && after_tiles[0].1 < base_tiles[0].1 - 0.1);
    assert!(after_texts.len() >= base_texts.len() + 9);
    for val in ["10", "90"] {
        assert!(after_texts.iter().any(|(t, c)| t == val && (*c == Rgba8::WHITE || *c == Rgba8::BLACK)));
    }

    // Part B (`WF-60`): Multi-row Y column Cmd+C -> Cmd+V on an XY sheet.
    let (xy_doc_2col, xy_id, _) = make_xy_and_graph_doc()?;
    let mut xy_app = new_human_app(xy_doc_2col);
    xy_app.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    let _ = xy_app.execute_command("sheet.select", json!({ "sheet_id": xy_id.as_str() })).map_err(|e| e.to_string())?;
    let mut grid_driver = make_sync_driver(xy_app, 1280.0, 800.0);
    grid_driver.click_widget_id("grid.cell.0.0.0")?;
    let y1_end = widget_center(&grid_driver, "grid.cell.3.0.0")?;
    grid_driver.click_at_button(y1_end, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    grid_driver.press_egui_key(Key::C, Modifiers::COMMAND);
    grid_driver.click_widget_id("grid.cell.0.1.0")?;
    grid_driver.press_egui_key(Key::V, Modifiers::COMMAND);
    let ds_after = grid_driver.state().app.document().data_sheet(&xy_id).ok_or("missing xy after copy/paste")?;
    assert!((0..4).all(|r| cell_number(ds_after, r, 1) == cell_number(ds_after, r, 0)));

    // Part C (`WF-60`): Duplicate Column context-menu action on a 1-Y-column XY sheet with linked GraphSheet.
    let (mut single_doc, xy1_id) =
        seed_xy_doc("WF-60 Duplicate Column", "ds-xy1", "XY 1Col", "X", &[1.0, 2.0, 3.0, 4.0], &[("RawY", &[10.0, 20.0, 30.0, 40.0])])?;
    let gid1 = add_init_graph(&mut single_doc, &xy1_id, "gs-xy1", GraphKind::Xy)?;
    assert_eq!(single_doc.graph_sheet(&gid1).ok_or("missing gs-xy1")?.spec.series.len(), 1);
    single_doc.select_in_navigator(&xy1_id).map_err(|e| e.to_string())?;
    let mut app1 = new_human_app(single_doc);
    app1.set_clipboard_adapter(Arc::new(StubPlatformClipboardAdapter::new()));
    let mut drv1 = make_sync_driver(app1, 1280.0, 800.0);

    let x_pos = widget_center(&drv1, "grid.xcell.0.0")?;
    drv1.click_at_button(x_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_widget_enabled(&drv1, "grid.ctx.duplicate_column", false)?;
    drv1.press_key("Escape", KeyModifiers::default())?;
    assert!(drv1.frame_meta().find_by_widget_id("grid.ctx.duplicate_column").is_none());

    let y_pos = widget_center(&drv1, "grid.cell.0.0.0")?;
    drv1.click_at_button(y_pos, PointerButton::Secondary, KeyModifiers::default())?;
    assert_widget_enabled(&drv1, "grid.ctx.duplicate_column", true)?;
    drv1.click_widget_id("grid.ctx.duplicate_column")?;
    assert_eq!(drv1.state().last_intent_error, None);

    let ds1_after = drv1.state().app.document().data_sheet(&xy1_id).ok_or("missing xy1 after duplicate")?;
    assert_eq!(ds1_after.archetype.column_count(), 2);
    let TableArchetype::Xy(xy_after) = &ds1_after.archetype else { return Err("expected Xy table".into()) };
    assert_eq!(xy_after.data_sets[1].title, "RawY (Copy)");
    assert!((0..4).all(|r| xy_after.data_sets[1].cells[r][0].active_number() == Some((r + 1) as f64 * 10.0)));
    let gs1_after = drv1.state().app.document().graph_sheet(&gid1).ok_or("missing linked graph after duplicate")?;
    assert_eq!((gs1_after.spec.series.len(), gs1_after.spec.series[1].data_set_index), (2, 1));
    Ok(())
}
