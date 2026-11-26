//! Round 2 S10 & companion `interaction-gui_actions.md` regression tests on native `egui` (`R0.19`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, ErrorCode, GraphAnnotation, GraphSheet, GraphicalDataObject, InfoSheet,
    OneWayAnovaSpec, OneWayMethod, PValueStackSpec, PowerAnalysisSpec, ResultSheet, SheetId,
};
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::control_io::dispatch_gui_action;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, AxisTarget};
use curvebook_ui_egui::egui_shell::center_stage::{CenterStageContext, render_center_stage};
use curvebook_ui_egui::egui_shell::inspect_bridge::HybridCoexistenceArbiter;
use curvebook_ui_egui::egui_shell::{LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens};
use curvebook_ui_egui::interaction::InlineEditorTarget;
use curvebook_ui_egui::modal_predicates::evaluate_dialog_widget;
use curvebook_ui_egui::power_wizard::PowerWizardStage;
use curvebook_ui_egui::workspace_nav::ActiveWorkspacePane;
use curvebook_ui_egui::{KeyModifiers, UiApp, ViewportMetrics, VirtualWorkspaceFolder, inspect_ui_tree, inspect_ui_tree_for_metrics};
use serde_json::json;

use crate::common::{TestRes, call_control, cell_number, ensure, key_down};
use crate::helpers::{exec, gui, run_panel_driver};

fn ok_doc_and_app() -> Result<(UiApp, SheetId, SheetId, SheetId, SheetId), String> {
    let mut app = UiApp::new();
    let d1 = exec(&mut app, "sheet.createData", json!({ "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 4, "auto_graph": true }))?;
    let ds_id = SheetId::try_new(d1.payload["sheet_id"].as_str().ok_or("missing ds_id")?).map_err(|e| e.to_string())?;
    let gs_id = SheetId::try_new(
        d1.payload["auto_graph_sheet_id"].as_str().or_else(|| app.document().graph_sheets.first().map(|g| g.id.as_str())).ok_or("missing graph")?,
    )
    .map_err(|e| e.to_string())?;
    exec(&mut app, "table.paste", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "start_col": 0, "text": "2\t4\n3\t6\n5\t9\n7\t11" }))?;
    exec(&mut app, "table.setXCells", json!({ "sheet_id": ds_id.as_str(), "start_row": 0, "values": [1.0, 2.0, 3.0, 4.0] }))?;

    let d2 = exec(&mut app, "sheet.createData", json!({ "title": "Groups", "kind": "column", "cols": 2, "rows": 4, "auto_graph": false }))?;
    let ds2_id = SheetId::try_new(d2.payload["sheet_id"].as_str().ok_or("missing ds2_id")?).map_err(|e| e.to_string())?;
    exec(&mut app, "table.paste", json!({ "sheet_id": ds2_id.as_str(), "start_row": 0, "start_col": 0, "text": "1\t3\n2\t4\n5\t6\n4\t7" }))?;

    let lay = exec(&mut app, "layout.create", json!({ "title": "Figure 1" }))?;
    let ls_id = SheetId::try_new(lay.payload["sheet_id"].as_str().ok_or("missing ls_id")?).map_err(|e| e.to_string())?;

    let mut doc = app.document().clone();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
    exec(&mut app, "info.setConstant", json!({ "sheet_id": "info-1", "name": "EC50", "value": 12.5 }))?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    app.sync_views();
    Ok((app, ds_id, ds2_id, gs_id, ls_id))
}

#[test]
fn r2_s10_01_human_path_only_rejects_non_empty_invoke_menu_params() -> TestRes {
    let (app, _, _, _, _) = ok_doc_and_app()?;
    let mut h = crate::human_wf_common::EguiWfHarness::from_app(app, ViewportMetrics::default());
    h.app_mut().set_human_path_only(true);
    let err = dispatch_gui_action(h.app_mut(), "ui.invoke_menu", json!({ "command_id": "ui.analyze.open", "params": { "search": "anova" } }))
        .err()
        .ok_or("expected human_path_only to reject non-empty params")?;
    ensure(
        err.code == ErrorCode::Conflict && err.details.as_ref().and_then(|d| d.get("code")) == Some(&json!("human_path_only_policy")),
        format!("bad err: {err:?}"),
    )
}

fn assert_modal_contract(app: &mut UiApp, slug: &str, expected_wid: Option<&str>) -> TestRes {
    let dlg = inspect_ui_tree(app, Some("dialog"), false).dialog.ok_or_else(|| format!("expected `{slug}` open"))?;
    ensure(dlg.dialog_kind == slug, format!("expected `{slug}`, got `{}`", dlg.dialog_kind))?;
    let retired = dispatch_gui_action(app, "ui.answer_dialog", json!({ "action": "ok" }))
        .err()
        .ok_or_else(|| format!("expected retired ui.answer_dialog error on `{slug}`"))?;
    ensure(retired.code == ErrorCode::BadInput, format!("expected BadInput for retired ui.answer_dialog, got {:?}", retired.code))?;
    if let Some(wid) = expected_wid {
        ensure(dlg.controls.iter().any(|c| c.widget_id == wid), format!("missing `{wid}` on `{slug}`"))?;
    }
    app.canvas_dialogs.close_dialog();
    Ok(())
}

#[test]
fn r2_s10_02_19_exhaustive_modal_contract_and_fail_closed() -> TestRes {
    let (mut app, ds_id, ds2_id, gs_id, ls_id) = ok_doc_and_app()?;
    let mut doc = app.document().clone();
    for (id, kind) in [
        (
            "rs-1",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::OrdinaryAnova, compute_effect_sizes: true, ..Default::default() }),
        ),
        ("rs-pwr-1", AnalysisKind::PowerAnalysis(PowerAnalysisSpec::default())),
        ("rs-ps-1", AnalysisKind::PValueStack(PValueStackSpec::default())),
    ] {
        let sp = AnalysisSpec::new(id, ds2_id.clone(), kind).map_err(|e| e.to_string())?;
        doc.add_result_sheet(ResultSheet::new(sp).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    let (rs_id, pwr_id, ps_id) = (SheetId::new("rs-1"), SheetId::new("rs-pwr-1"), SheetId::new("rs-ps-1"));
    let doc = app.document().clone();

    let cases: Vec<(&str, Option<&str>)> = vec![
        ("analyze_sheet", Some("analyze.search")),
        ("duplicate_analysis_prompt", Some("duplicate.choice")),
        ("edit_anova", Some("edit.pvalue.digits.val")),
        ("edit_power", Some("power.sidebar.alpha")),
        ("edit_pstack", Some("edit.pvalue_stack.threshold.val")),
        ("power_and_sample_size", Some("power.sidebar.alpha")),
        ("format_selected_points", Some("fmt_pts.hidden")),
        ("format_graph", Some("format_graph.show_legend")),
        ("format_axes", Some("axes.title")),
        ("pairwise_comparisons", Some("pairwise.hide_non_significant")),
        ("bubble_plot_inspector", Some("bubble.show_convex_hulls")),
        ("apply_style_from", Some("apply_style.clone_palette")),
        ("page_layout_composer", Some("layout.align_plot_origins")),
        ("export_figure", Some("export.dpi")),
        ("convert_archetype", Some("convert.allow_lossy")),
        ("import_reshape", Some("import_paste.input.title")),
        ("exclude_with_reason", Some("exclude.note")),
        ("randomize_groups", Some("rand_groups.seed.val")),
        ("random_sample_rows", Some("rand_sample.n.val")),
        ("simulate_distributions", Some("simulate.use_multiple_variables")),
        ("insert_model_equation", Some("eq.bordered")),
        ("print", Some("print.copies")),
        ("change_data_analyzed", Some("change_data.col.0")),
        ("change_graph_type", Some("graph_type.summary_measure")),
        ("constant_hook_picker", Some("hook.search_input")),
        ("command_palette", Some("palette.query")),
        ("about", None),
        ("result_viewer", None),
    ];

    for (tag, expected_wid) in cases {
        let slug = match tag {
            "analyze_sheet" => {
                app.canvas_dialogs.open_analyze_dialog(&doc, &ds2_id);
                tag
            }
            "duplicate_analysis_prompt" => {
                app.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
                    source_sheet_id: ds2_id.clone(),
                    pending_entry_id: "stats.one_way_anova".into(),
                    existing_result_ids: vec![rs_id.clone()],
                };
                tag
            }
            "edit_anova" => {
                app.canvas_dialogs.open_edit_parameters(&doc, &rs_id);
                if let Some(d) = &mut app.canvas_dialogs.edit_params.draft {
                    d.p_value_format = curvebook_doc::PValueFormat::new_custom(4, Some(1e-4));
                }
                app.canvas_dialogs.edit_params.set_p_value_decimals(3)?;
                gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.tab.options" }))?;
                "edit_analysis_parameters"
            }
            "edit_power" => {
                app.canvas_dialogs.open_edit_parameters(&doc, &pwr_id);
                "edit_analysis_parameters"
            }
            "edit_pstack" => {
                app.canvas_dialogs.open_edit_parameters(&doc, &ps_id);
                "edit_analysis_parameters"
            }
            "power_and_sample_size" => {
                app.canvas_dialogs.open_power_wizard();
                tag
            }
            "format_selected_points" => {
                app.canvas_dialogs.open_format_selected_points(&doc, &gs_id, vec![(0, 0)]);
                tag
            }
            "format_graph" => {
                app.canvas_dialogs.open_format_graph(&doc, &gs_id);
                gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.tab.format_graph.settings" }))?;
                tag
            }
            "format_axes" => {
                app.canvas_dialogs.format_axes.sync_from_graph(doc.graph_sheet(&gs_id).ok_or("missing graph")?, AxisTarget::Y);
                app.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: gs_id.clone() };
                gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "edit.tab.axes.titles" }))?;
                tag
            }
            "pairwise_comparisons" => {
                app.canvas_dialogs.open_pairwise_comparisons(&doc, &gs_id);
                tag
            }
            "bubble_plot_inspector" => {
                app.canvas_dialogs.open_bubble_plot(&doc, &gs_id);
                tag
            }
            "apply_style_from" => {
                app.canvas_dialogs.open_apply_style_from(&doc, &gs_id);
                tag
            }
            "page_layout_composer" => {
                app.canvas_dialogs.open_page_layout_composer(&doc, &ls_id);
                tag
            }
            "export_figure" => {
                app.canvas_dialogs.open_export_figure(&gs_id);
                tag
            }
            "convert_archetype" => {
                app.canvas_dialogs.open_convert_archetype(&doc, &ds2_id);
                tag
            }
            "import_reshape" => {
                app.canvas_dialogs.open_import_reshape();
                tag
            }
            "exclude_with_reason" => {
                app.canvas_dialogs.open_exclude_with_reason(ds_id.clone(), vec![GridCellCoord::new(0, 0, 0)]);
                tag
            }
            "randomize_groups" => {
                app.canvas_dialogs.open_randomize_groups(&doc);
                tag
            }
            "random_sample_rows" => {
                app.canvas_dialogs.open_random_sample_rows(&doc, Some(&ds_id));
                tag
            }
            "simulate_distributions" => {
                app.canvas_dialogs.open_simulate_distributions();
                tag
            }
            "insert_model_equation" => {
                app.canvas_dialogs.open_insert_model_equation(&doc, &gs_id);
                tag
            }
            "print" => {
                app.open_print_dialog_for(Some(&gs_id));
                tag
            }
            "change_data_analyzed" => {
                app.canvas_dialogs.open_change_data_analyzed(&doc, &rs_id);
                tag
            }
            "change_graph_type" => {
                exec(&mut app, "sheet.select", json!({ "sheet_id": gs_id.as_str() }))?;
                gui(&mut app, "ui.invoke_menu", json!({ "command_id": "ui.changeGraphType.open" }))?;
                tag
            }
            "constant_hook_picker" => {
                app.canvas_dialogs.open_hook_picker(&doc, &rs_id, Some("Bottom".into()));
                tag
            }
            "command_palette" => {
                app.canvas_dialogs.open_command_palette();
                tag
            }
            "about" => {
                app.canvas_dialogs.active_dialog = ActiveDialog::About;
                tag
            }
            "result_viewer" => {
                app.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: rs_id.clone() };
                tag
            }
            _ => return Err(format!("unknown modal tag `{tag}`")),
        };
        assert_modal_contract(&mut app, slug, expected_wid)?;
    }
    Ok(())
}

#[test]
fn r2_s10_05_06_07_08_inspect_predicates_and_format_graph() -> TestRes {
    let mut big_app = UiApp::new();
    let out = exec(&mut big_app, "sheet.createData", json!({ "title": "Big", "kind": "xy", "cols": 1, "rows": 2000, "auto_graph": false }))?;
    let big_id = SheetId::try_new(out.payload["sheet_id"].as_str().ok_or("big id")?).map_err(|e| e.to_string())?;
    let doc = big_app.document().clone();
    big_app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    big_app.grid.extend_shift_range(doc.data_sheet(&big_id).ok_or("big ds")?, GridCellCoord::new(1999, 0, 0));
    big_app.grid.refresh_selection_stats(&doc);

    let full = inspect_ui_tree(&big_app, None, false);
    ensure(
        full.grid_selection.selected_cells_count == 2000
            && full.grid_selection.selected_coords.len() <= 500
            && full.grid_selection.selected_rows.len() <= 500,
        "grid snapshot cap failed",
    )?;
    let tb = inspect_ui_tree(&big_app, Some("toolbar"), false);
    ensure(tb.grid_selection.selected_coords.is_empty() && tb.grid_cells.is_none(), "toolbar scope leaked grid")?;

    let (mut app, _, ds2_id, gs_id, _) = ok_doc_and_app()?;
    let rs = ResultSheet::new(
        AnalysisSpec::new("rs-1", ds2_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::KruskalWallis, ..Default::default() }))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let rs_id = rs.id.clone();
    let mut d = app.document().clone();
    d.add_result_sheet(rs).map_err(|e| e.to_string())?;
    app = UiApp::from_session(curvebook_engine::Session::from_document(d));
    app.canvas_dialogs.open_edit_parameters(&app.document().clone(), &rs_id);
    app.canvas_dialogs.edit_params.assume_gaussian = false;
    ensure(
        !evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunnett").enabled
            && evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunn").enabled
            && evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.multiplicity_method").enabled,
        "R2-S10-06 predicate routing failed",
    )?;
    app.canvas_dialogs.close_dialog();

    for (btn_wid, on) in [("dialog.button.apply", true), ("dialog.button.ok", false)] {
        app.canvas_dialogs.open_format_graph(&app.document().clone(), &gs_id);
        app.canvas_dialogs.format_graph.half_violin_with_points = on;
        gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": btn_wid }))?;
    }
    let still_half = app.document().graph_sheet(&gs_id).and_then(|g| g.spec.series.first()).and_then(|s| match s.gdo {
        GraphicalDataObject::Violin { half_violin_with_points, .. } => Some(half_violin_with_points),
        _ => None,
    });
    ensure(still_half == Some(false), format!("half_violin_with_points mismatch: {still_half:?}"))?;

    let sb = inspect_ui_tree_for_metrics(&app, ViewportMetrics::new(1024.0, 640.0, 1.0), Some("toolbar"), true)
        .toolbar
        .and_then(|t| t.last()?.bounds)
        .ok_or("small bounds")?;
    let lb = inspect_ui_tree_for_metrics(&app, ViewportMetrics::new(1920.0, 1200.0, 2.0), Some("toolbar"), true)
        .toolbar
        .and_then(|t| t.last()?.bounds)
        .ok_or("large bounds")?;
    ensure(sb != lb, "viewport metrics did not change inspect bounds")
}

#[test]
fn r2_s10_10_13_14_15_17_18_line_arrow_formula_discontig_print_wizard_row_labels() -> TestRes {
    let (mut app, _, _, gs_id, _) = ok_doc_and_app()?;

    // R2-S10-10: LineOrArrow preserves reverse direction across hit-test & translation.
    exec(
        &mut app,
        "graph.addAnnotation",
        json!({ "sheet_id": gs_id.as_str(), "kind": "arrow", "start_pt": [140.0, 60.0], "end_pt": [80.0, 120.0], "arrowhead": true }),
    )?;
    exec(&mut app, "sheet.select", json!({ "sheet_id": gs_id.as_str() }))?;
    app.sync_views();
    let gs = app.document().graph_sheet(&gs_id).ok_or("missing gs")?.clone();
    let scene = curvebook_chart::compile_graph(&gs, app.document()).map_err(|e| e.to_string())?;
    let hit = curvebook_tools::canvas::hit_test_graph(&scene, &gs.spec, curvebook_chart::PointPt::new(110.0, 90.0), 8.0);
    ensure(matches!(hit, Some(curvebook_tools::canvas::CanvasHitTarget::Annotation { index: 0 })), format!("arrow hit: {hit:?}"))?;
    if let Some(GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color }) = gs.spec.annotations.first() {
        let mut next = gs.spec.annotations.clone();
        next[0] = GraphAnnotation::LineOrArrow {
            start_pt: (start_pt.0 + 20.0, start_pt.1 + 10.0),
            end_pt: (end_pt.0 + 20.0, end_pt.1 + 10.0),
            arrowhead: *arrowhead,
            dash: *dash,
            color: *color,
        };
        exec(&mut app, "graph.setSpec", json!({ "sheet_id": gs_id.as_str(), "spec": { "annotations": next } }))?;
    }
    if let Some(GraphAnnotation::LineOrArrow { start_pt, end_pt, .. }) = app.document().graph_sheet(&gs_id).and_then(|g| g.spec.annotations.first()) {
        ensure(start_pt.0 > end_pt.0 && start_pt.1 < end_pt.1, "arrow orientation collapsed")?;
    }

    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "mv-fb", "title": "MV Formula", "kind": "multiple_variables", "rows": 4, "cols": 2, "variables": [{ "title": "A", "role": "continuous" }, { "title": "B", "role": "continuous" }], "auto_graph": false }),
    )?;
    exec(&mut app, "table.paste", json!({ "sheet_id": "mv-fb", "start_row": 0, "start_col": 0, "text": "1\t2\n3\t4\n5\t6\n7\t8" }))?;
    let mv_fb_id = SheetId::new("mv-fb");
    exec(&mut app, "sheet.select", json!({ "sheet_id": "mv-fb" }))?;

    // R2-S10-13: select_row_click preserves active formula bar; commit + toolbar enabled.
    let doc_mv = app.document().clone();
    let ds = doc_mv.data_sheet(&mv_fb_id).ok_or("mv-fb")?;
    app.grid.formula_bar.open_for_new_column(ds, "Ratio", Some(&doc_mv));
    app.grid.formula_bar.set_input("=A + 1", None, &doc_mv);
    app.grid.select_row_click(ds, 0, false, false);
    ensure(app.grid.formula_bar.active && app.grid.formula_bar.input == "=A + 1", "formula bar reset")?;
    let commit = inspect_ui_tree(&app, Some("grid"), false)
        .grid_cells
        .and_then(|v| v.into_iter().find(|n| n.widget_id == "formula_bar.commit"))
        .ok_or("missing formula_bar.commit")?;
    ensure(commit.enabled, "formula_bar.commit disabled")?;
    let mut fb_app = app.clone();
    fb_app.nav.welcome.open = false;
    let fb_h = crate::human_wf_common::EguiWfHarness::from_app(fb_app, ViewportMetrics::default());
    ensure(
        fb_h.driver.frame_meta().find_by_widget_id("toolbar.formula_bar").ok_or("missing toolbar.formula_bar")?.enabled,
        "toolbar.formula_bar disabled",
    )?;
    app.grid.formula_bar.active = false;

    // R2-S10-14: Discontiguous Cmd+Click preserves exact 2 selected cells.
    app.grid.select_single_sync(GridCellCoord::new(0, 0, 0));
    app.grid.toggle_cmd_selection(ds, GridCellCoord::new(2, 1, 0));
    app.grid.refresh_selection_stats(&doc_mv);
    let sel: Vec<GridCellCoord> = app
        .grid
        .visible_frame(app.document())
        .ok_or("visible_frame")?
        .rows
        .iter()
        .flat_map(|r| r.cells.iter())
        .filter(|c| c.visual.is_selected)
        .map(|c| c.coord)
        .collect();
    ensure(sel == vec![GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 1, 0)], format!("discontig sel: {sel:?}"))?;
    ensure(
        app.grid.quick_stats.map(|q| q.selected_cells) == Some(2) && app.grid.emit_toggle_exclude_intents(ds, true) == 2,
        "discontig stats/exclude mismatch",
    )?;

    // R2-S10-18: FormatSelectedPoints row labels placed at distinct coordinates.
    app.canvas_dialogs.open_format_selected_points(&doc_mv, &gs_id, vec![(0, 0), (0, 2)]);
    app.canvas_dialogs.format_points.show_row_labels = true;
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))?;
    let pos: Vec<(f64, f64)> = app
        .document()
        .graph_sheet(&gs_id)
        .map(|g| {
            g.spec
                .annotations
                .iter()
                .filter_map(|a| match a {
                    GraphAnnotation::TextBox { bounds_pt, .. } => Some((bounds_pt.x, bounds_pt.y)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    ensure(pos.len() >= 2 && pos[0] != pos[1], format!("row label coords collapsed: {pos:?}"))?;

    // R2-S10-15: Print reports spooled/submitted honestly.
    app.open_print_dialog_for(Some(&gs_id));
    let pr = gui(&mut app, "ui.pointer_gesture", json!({ "action": "click", "widget_id": "dialog.button.ok" }))?;
    ensure(pr["spooled"] == json!(true) && pr["submitted_to_os"] == json!(false) && pr["printed"] == json!(false), format!("print: {pr}"))?;

    // R2-S10-17: PowerWizard switch_tab validates intermediate stage.
    app.canvas_dialogs.open_power_wizard();
    app.canvas_dialogs.power_wizard.alpha = 2.0;
    ensure(dispatch_gui_action(&mut app, "ui.switch_tab", json!({ "tab": "results" })).is_err(), "invalid alpha must fail switch_tab")?;
    app.canvas_dialogs.power_wizard.alpha = 0.05;
    gui(&mut app, "ui.switch_tab", json!({ "tab": "results" }))?;
    ensure(
        app.canvas_dialogs.power_wizard.stage == PowerWizardStage::Results && app.canvas_dialogs.power_wizard.result.is_some(),
        "PowerWizard results missing",
    )
}

#[test]
fn r2_s10_19_virtual_workspace_validates_and_inline_backspace_syncs() -> TestRes {
    let mut vws = VirtualWorkspaceFolder::new("invalid-vws");
    vws.write_file("document.json", b"{ \"invalid\": true ".to_vec())?;
    ensure(vws.open_unpacked_snapshot().is_err(), "corrupt document.json must fail")?;

    let (mut app, ds_id, _, _, _) = ok_doc_and_app()?;
    app.inline_editor.begin(InlineEditorTarget::GridCell { sheet_id: ds_id, coord: GridCellCoord::new(0, 0, 0) }, "123".into());
    app.grid.tool.editing_buffer = Some("123".into());
    key_down(&mut app, "Backspace", KeyModifiers::default())?;
    ensure(app.grid.tool.editing_buffer.as_deref() == Some("12"), format!("editing_buffer={:?}", app.grid.tool.editing_buffer))
}

#[test]
fn interaction_gui_actions_items_1_3_4_5_new1() -> TestRes {
    let (mut app, ds_id, _, gs_id, _) = ok_doc_and_app()?;
    let rs =
        ResultSheet::new(AnalysisSpec::new("rs-1", ds_id.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let rs_id = rs.id.clone();
    let mut d = app.document().clone();
    d.add_result_sheet(rs).map_err(|e| e.to_string())?;
    app = UiApp::from_session(curvebook_engine::Session::from_document(d));
    exec(&mut app, "sheet.select", json!({ "sheet_id": rs_id.as_str() }))?;

    // Item 1: Cmd+T must not escape open modal.
    app.canvas_dialogs.open_command_palette();
    app.focus_widget_id = Some("formula_bar.input".into());
    gui(&mut app, "ui.press_key", json!({ "key": "t", "modifiers": ["meta"] }))?;
    ensure(matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette), "Cmd+T escaped modal")?;
    app.canvas_dialogs.close_dialog();

    // Item 3: Escape closes focused formula bar.
    exec(&mut app, "sheet.select", json!({ "sheet_id": ds_id.as_str() }))?;
    let doc_ds = app.document().clone();
    app.grid.formula_bar.open_for_new_column(doc_ds.data_sheet(&ds_id).ok_or("ds")?, "NewCol", Some(&doc_ds));
    app.focus_widget_id = Some("formula_bar.input".into());
    gui(&mut app, "ui.press_key", json!({ "key": "Escape" }))?;
    ensure(!app.grid.formula_bar.active, "formula_bar Escape failed")?;

    // Item 4: empty right-click clears context_menu_target.
    app.interaction.context_menu_target = Some("grid".into());
    gui(&mut app, "ui.pointer_gesture", json!({ "action": "right_click", "x": 5.0, "y": 5.0 }))?;
    ensure(app.interaction.context_menu_target.is_none(), "context_menu_target not cleared")?;

    // Item 5: de-DE decimal separator in ui.type_text.
    let loc = call_control(&mut app, &json!({ "id": 1, "method": "ui.locale.set", "params": { "locale": "de-DE" } }))?;
    ensure(loc["ok"] == json!(true) && app.locale == "de", "de-DE locale set failed")?;
    for (wid, txt) in [("grid.cell.0.0.0", "3,75"), ("grid.xcell.0.0", "1,25"), ("grid.cell.1.0.0", "4,5\n5,5")] {
        gui(&mut app, "ui.type_text", json!({ "widget_id": wid, "text": txt }))?;
    }
    let ds_after = app.document().data_sheet(&ds_id).ok_or("missing ds")?;
    ensure(
        cell_number(ds_after, 0, 0) == Some(3.75)
            && ds_after.x_or_time_cell(0, 0).and_then(|c| c.value.finite_f64()) == Some(1.25)
            && cell_number(ds_after, 1, 0) == Some(4.5)
            && cell_number(ds_after, 2, 0) == Some(5.5),
        "de-DE ui.type_text mismatch",
    )?;

    // NEW-1: active inline_editor auto-commits on sheet switch.
    app.inline_editor.begin(InlineEditorTarget::GridCell { sheet_id: ds_id.clone(), coord: GridCellCoord::new(0, 1, 0) }, "42,5".into());
    gui(&mut app, "ui.switch_tab", json!({ "sheet_id": gs_id.as_str() }))?;
    ensure(cell_number(app.document().data_sheet(&ds_id).ok_or("ds")?, 0, 1) == Some(42.5) && !app.inline_editor.active, "NEW-1 auto-commit failed")
}

fn assert_rect_inside(outer: egui::Rect, inner: egui::Rect, label: &str) -> TestRes {
    ensure(
        inner.left() >= outer.left() - 1.0
            && inner.top() >= outer.top() - 1.0
            && inner.right() <= outer.right() + 1.0
            && inner.bottom() <= outer.bottom() + 1.0,
        format!("{label} rect {inner:?} escaped {outer:?}"),
    )
}

#[test]
fn r2_s10_09_split_pane_preserves_primary_singleton_geometry() -> TestRes {
    let (mut app, ds_id, ds2_id, gs_id, _) = ok_doc_and_app()?;
    let warn = ChecklistItem {
        assumption: "Normality".into(),
        status: ChecklistStatus::Warn,
        diagnostic_metric: None,
        explanation: "Minor skew".into(),
        remediation: None,
    };
    let mut doc = app.document().clone();
    let gs2 = GraphSheet::new_uninitialized_for_data("g-split-2", doc.data_sheet(&ds2_id).ok_or("ds2")?).map_err(|e| e.to_string())?;
    let gs2_id = gs2.id.clone();
    doc.add_graph_sheet(gs2).map_err(|e| e.to_string())?;
    for (rid, src) in [("rs-split-1", &ds_id), ("rs-split-2", &ds2_id)] {
        let mut rs =
            ResultSheet::new(AnalysisSpec::new(rid, src.clone(), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        rs.checklist.push(warn.clone());
        doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
    }
    let info2 = InfoSheet::new_preseeded("info-2", "Secondary Info").map_err(|e| e.to_string())?;
    let info2_id = info2.id.clone();
    doc.add_info_sheet(info2).map_err(|e| e.to_string())?;
    app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
    app.nav.welcome.open = false;
    app.split.enabled = true;

    type SplitRenderResult = Result<(egui::Rect, egui::Rect, Vec<(String, egui::Rect)>), String>;
    let render_split = |p: SheetId, s: SheetId| -> SplitRenderResult {
        let mut ap = app.clone();
        ap.split.primary_sheet_id = Some(p);
        ap.split.secondary_sheet_id = Some(s);
        let views = ap.egui_views.clone();
        let drv = run_panel_driver((ap, views, PendingUiIntents::new(), None, None), 1280.0, 800.0, |ui, meta, (a, vw, ints, pr, sr)| {
            let tokens = Tokens::for_mode(a.theme.mode);
            let decision = HybridCoexistenceArbiter::evaluate(a, false);
            let layout = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
                viewport_w_pt: ui.max_rect().width(),
                viewport_h_pt: ui.max_rect().height(),
                empty_workspace: false,
                navigator_open: false,
                inspector_open: false,
                family_open: false,
                leading_user_collapsed: false,
                trailing_user_collapsed: false,
                leading_width_override_pt: None,
                trailing_width_override_pt: None,
                host_draws_menu_bar: false,
                is_rtl: false,
            });
            let rep = render_center_stage(
                ui,
                a,
                vw,
                CenterStageContext {
                    tokens: &tokens,
                    decision: &decision,
                    layout,
                    focused_pane: ActiveWorkspacePane::MainCanvasOrGrid,
                    enabled: true,
                },
                meta,
                ints,
            );
            *pr = rep.primary_rect_pt;
            *sr = rep.secondary_rect_pt;
        });
        let entries = drv.frame_meta().entries.iter().map(|e| (e.widget_id.clone(), e.clipped_rect)).collect();
        Ok((drv.state().3.ok_or("missing primary_rect")?, drv.state().4.ok_or("missing secondary_rect")?, entries))
    };

    let (gp, gs_rect, g_entries) = render_split(ds_id.clone(), ds2_id)?;
    assert_rect_inside(gp, g_entries.iter().find(|(id, _)| id == "grid.cell.0.0.0").ok_or("grid.cell")?.1, "primary grid.cell.0.0.0")?;
    ensure(gs_rect.is_positive(), "missing secondary DataSheet pane")?;

    let (grp, grs, _) = render_split(gs_id.clone(), gs2_id)?;
    ensure(grp.is_positive() && grs.is_positive() && !grp.intersects(grs), "invalid split GraphSheet rects")?;

    let (rp, rs_rect, r_entries) = render_split(SheetId::new("rs-split-1"), SheetId::new("rs-split-2"))?;
    let banner = r_entries.iter().find(|(id, _)| id == "result.diagnostic_banner").ok_or("missing diagnostic_banner")?.1;
    ensure(
        banner.top() >= rp.bottom() - 1.0 && banner.bottom() <= 800.0 && !rp.intersects(banner) && !rs_rect.intersects(banner),
        "diagnostic_banner collided",
    )?;
    let (_, _, d_entries) = render_split(ds_id.clone(), SheetId::new("rs-split-2"))?;
    ensure(!d_entries.iter().any(|(id, _)| id == "result.diagnostic_banner"), "secondary ResultSheet emitted primary banner")?;

    let (ip, _, i_entries) = render_split(SheetId::new("info-1"), info2_id)?;
    assert_rect_inside(ip, i_entries.iter().find(|(id, _)| id == "info.notes_editor").ok_or("info.notes_editor")?.1, "primary info.notes_editor")?;

    let (mp, ms, m_entries) = render_split(ds_id, gs_id)?;
    assert_rect_inside(mp, m_entries.iter().find(|(id, _)| id == "grid.cell.0.0.0").ok_or("mixed cell")?.1, "mixed grid.cell")?;
    assert_rect_inside(ms, m_entries.iter().find(|(id, _)| id.starts_with("canvas.")).ok_or("mixed canvas")?.1, "mixed canvas")
}
