//! R10 Wave 5 Cross-Shard Integration Sweep (`r10-w5-integ`).
//!
//! Exercises every newly wired `Integ Handoff` hook end-to-end and sweeps
//! `validate_layout_tree_accessibility` + `verify_tree_invariants` across:
//! - Welcome launcher (empty & opened), all 8 `DataSheet` archetypes, `InfoSheet`,
//!   `ResultSheet`, `GraphSheet`, `LayoutSheet`,
//! - Split panes (`primary` + `secondary`), `mv_formula` right dock, `floating_notes` overlay,
//! - All 6 `ActiveInspector` panels,
//! - HiDPI viewports (`1.0x`, `1.5x`, `2.0x`) in LTR and RTL (`ar`, `he`),
//! - All `26` `ActiveDialog` variants.
#![forbid(unsafe_code)]

#[path = "human_wf_common/mod.rs"]
mod human_wf_common;

use curvebook_doc::{CellCoord, FloatingNote, RectPt, SheetId};
use curvebook_engine::Session;
use curvebook_ui_egui::dialogs_canvas::{ActiveDialog, ActiveInspector, ExportFigureFormat};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, UiInputEvent, dispatch_ui_event};
use curvebook_ui_egui::layout::WidgetId;
use curvebook_ui_egui::layout::a11y::{FocusScope, validate_layout_tree_accessibility};
use curvebook_ui_egui::layout::verify::verify_tree_invariants;
use curvebook_ui_egui::{UiApp, ViewportMetrics, dispatch_ui_command};
use human_wf_common::{click_widget_center, default_viewport, new_human_driver, press_cmd_shortcut, scroll_until_widget_visible};
use serde_json::json;

fn wid(id: &str) -> WidgetId {
    WidgetId::from(id)
}

fn assert_tree_clean(app: &mut UiApp, metrics: ViewportMetrics, label: &str) -> Result<(), String> {
    let tree = app.build_and_cache_layout_tree(metrics);
    validate_layout_tree_accessibility(&tree).map_err(|e| format!("{label} @ {metrics:?} a11y failed: {e}"))?;
    let report = verify_tree_invariants(&tree);
    if !report.is_clean() {
        return Err(format!("{label} @ {metrics:?} parity failed: {:?}", report.violations));
    }
    Ok(())
}

fn build_rich_all_archetypes_app() -> Result<UiApp, String> {
    let mut app = UiApp::new();
    let specs = [
        ("ds-xy", "xy", "Dose Response XY", 2usize, 4usize),
        ("ds-col", "column", "Groups Column", 3usize, 5usize),
        ("ds-grp", "grouped", "Grouped Assay", 2usize, 4usize),
        ("ds-mv", "multiple_variables", "Multivariate Cohort", 4usize, 5usize),
        ("ds-cont", "contingency", "Contingency 2x2", 2usize, 2usize),
        ("ds-surv", "survival", "Kaplan Survival", 2usize, 4usize),
        ("ds-pow", "parts_of_whole", "Parts of Whole", 1usize, 4usize),
        ("ds-nest", "nested", "Nested Cohort", 2usize, 4usize),
    ];
    for (idx, (id, kind, title, cols, rows)) in specs.iter().enumerate() {
        let _ = app
            .execute_command(
                "sheet.createData",
                json!({
                    "sheet_id": id,
                    "title": title,
                    "kind": kind,
                    "data_sets": cols,
                    "cols": cols,
                    "rows": rows,
                    "replicates": if matches!(*kind, "grouped" | "nested") { 2 } else { 1 },
                    "auto_info": idx == 0,
                    "auto_graph": idx == 0,
                }),
            )
            .map_err(|e| e.to_string())?;
    }

    // Populate numeric cells on XY, Column, and MV tables so analyses & graphs have real data.
    let _ = app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [1.0, 2.0, 4.0, 8.0] })).map_err(|e| e.to_string())?;
    let _ = app
        .execute_command("table.setCells", json!({ "sheet_id": "ds-xy", "rows": [[10.0, 12.0], [22.0, 24.0], [45.0, 48.0], [82.0, 86.0]] }))
        .map_err(|e| e.to_string())?;
    let _ = app
        .execute_command(
            "table.setCells",
            json!({
                "sheet_id": "ds-col",
                "rows": [
                    [10.0, 14.0, 19.0],
                    [11.0, 15.0, 20.0],
                    [10.5, 14.8, 21.0],
                    [11.2, 15.2, 19.6],
                    [10.8, 14.6, 20.4]
                ]
            }),
        )
        .map_err(|e| e.to_string())?;
    let _ = app
        .execute_command(
            "table.setCells",
            json!({
                "sheet_id": "ds-mv",
                "rows": [
                    [1.0, 10.0, 100.0, 5.0],
                    [2.0, 20.0, 150.0, 7.0],
                    [3.0, 30.0, 210.0, 9.0],
                    [4.0, 40.0, 260.0, 11.0],
                    [5.0, 50.0, 310.0, 13.0]
                ]
            }),
        )
        .map_err(|e| e.to_string())?;

    // Run one-way ANOVA on ds-col so we have a real ResultSheet.
    let _ = app
        .execute_command(
            "analysis.add",
            json!({
                "source_sheet_id": "ds-col",
                "sheet_id": "res-anova",
                "kind": "one_way_anova"
            }),
        )
        .map_err(|e| e.to_string())?;

    // Ensure a GraphSheet and LayoutSheet exist.
    if app.document().graph_sheets.is_empty() {
        let _ = app.execute_command("graph.create", json!({ "data_sheet_id": "ds-xy", "sheet_id": "graph-xy" })).map_err(|e| e.to_string())?;
    }
    let _ = app.execute_command("layout.create", json!({ "sheet_id": "layout-1", "title": "Figure 1" })).map_err(|e| e.to_string())?;

    // Add a FloatingNote onto ds-xy and a second note onto the primary GraphSheet.
    let mut doc = app.document().clone();
    if let Some(ds) = doc.data_sheet_mut(&SheetId::new("ds-xy")) {
        ds.floating_notes.push(FloatingNote::new("note-xy", RectPt::new(28.0, 28.0, 150.0, 76.0), "Dilution verified"));
    }
    if let Some(gs) = doc.graph_sheets.first_mut() {
        gs.floating_notes.push(FloatingNote::new("note-gr", RectPt::new(32.0, 32.0, 150.0, 76.0), "4PL convergence"));
    }
    Ok(UiApp::from_session(Session::from_document(doc)))
}

#[test]
fn r10_w5_cross_shard_views_all_8_archetypes_splits_dock_notes_hidpi_and_rtl_parity() -> Result<(), String> {
    let mut app = build_rich_all_archetypes_app()?;
    let viewports = [
        ("en", ViewportMetrics::new(1280.0, 800.0, 1.0)),
        ("en", ViewportMetrics::new(1440.0, 900.0, 1.5)),
        ("en", ViewportMetrics::new(1600.0, 1000.0, 2.0)),
        ("ar", ViewportMetrics::new(1280.0, 800.0, 1.0)),
        ("he", ViewportMetrics::new(1600.0, 1000.0, 2.0)),
    ];

    // 1. Empty Welcome screen across LTR/RTL and 1.0x/1.5x/2.0x HiDPI.
    let mut empty_app = UiApp::new();
    for (locale, vp) in viewports {
        empty_app.locale = String::from(locale);
        assert_tree_clean(&mut empty_app, vp, &format!("welcome_empty_{locale}"))?;
    }

    // 2. Populated project: Welcome overlay open, all 8 DataSheet archetypes, InfoSheet, ResultSheet, GraphSheet, LayoutSheet.
    let info_id = app.document().info_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing info sheet")?;
    let graph_id = app.document().graph_sheets.first().map(|s| s.id.as_str().to_owned()).ok_or("missing graph sheet")?;
    let sheet_ids = vec![
        String::from("ds-xy"),
        String::from("ds-col"),
        String::from("ds-grp"),
        String::from("ds-mv"),
        String::from("ds-cont"),
        String::from("ds-surv"),
        String::from("ds-pow"),
        String::from("ds-nest"),
        info_id,
        String::from("res-anova"),
        graph_id.clone(),
        String::from("layout-1"),
    ];

    for (locale, vp) in viewports {
        app.locale = String::from(locale);
        app.nav.welcome.open = true;
        assert_tree_clean(&mut app, vp, &format!("welcome_overlay_{locale}"))?;
        app.nav.welcome.open = false;

        for sid in &sheet_ids {
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": sid })).map_err(|e| e.to_string())?;
            app.canvas_dialogs.active_dialog = ActiveDialog::None;
            assert_tree_clean(&mut app, vp, &format!("sheet_{sid}_{locale}"))?;
        }

        // 3. Docked mv_formula right panel open on ds-mv alongside open Inspector.
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": "ds-mv" })).map_err(|e| e.to_string())?;
        app.nav.inspector_open = true;
        if let Some(ds_mv) = app.document().data_sheet(&SheetId::new("ds-mv")).cloned() {
            let doc_mv = app.document().clone();
            app.grid.formula_bar.open_for_column(&ds_mv, 1, Some(&doc_mv));
            app.view_shards.mv_formula.right_panel_open = true;
            assert_tree_clean(&mut app, vp, &format!("ds_mv_formula_dock_{locale}"))?;
            app.grid.formula_bar.active = false;
            app.grid.formula_bar.target_column = None;
            app.view_shards.mv_formula.right_panel_open = false;
        }

        // 4. Split-pane view: DataSheet primary + ResultSheet secondary.
        app.split.enabled = true;
        app.split.primary_sheet_id = Some(SheetId::new("ds-col"));
        app.split.secondary_sheet_id = Some(SheetId::new("res-anova"));
        assert_tree_clean(&mut app, vp, &format!("split_data_result_{locale}"))?;
        app.split.enabled = false;

        // 5. All 6 ActiveInspector tabs on GraphSheet / LayoutSheet / ResultSheet.
        for insp in [
            ActiveInspector::FormatGraph,
            ActiveInspector::FormatAxes,
            ActiveInspector::BubblePlot,
            ActiveInspector::PairwiseComparisons,
            ActiveInspector::PageLayout,
            ActiveInspector::ResultDetails,
        ] {
            let target_sid = match insp {
                ActiveInspector::PageLayout => "layout-1",
                ActiveInspector::ResultDetails => "res-anova",
                _ => &graph_id,
            };
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": target_sid })).map_err(|e| e.to_string())?;
            app.canvas_dialogs.active_dialog = ActiveDialog::None;
            app.nav.inspector_open = true;
            app.canvas_dialogs.active_inspector = insp;
            assert_tree_clean(&mut app, vp, &format!("inspector_{insp:?}_{locale}"))?;
        }
    }

    Ok(())
}

#[test]
fn r10_w5_all_26_active_dialogs_pass_a11y_and_parity() -> Result<(), String> {
    let mut app = build_rich_all_archetypes_app()?;
    let doc = app.document().clone();
    let ds_xy = SheetId::new("ds-xy");
    let ds_col = SheetId::new("ds-col");
    let res_id = SheetId::new("res-anova");
    let graph_id = doc.graph_sheets.first().ok_or("missing graph sheet")?.id.clone();
    let layout_id = SheetId::new("layout-1");

    type DialogSetupFn = fn(&mut UiApp, &SheetId, &SheetId, &SheetId, &SheetId, &SheetId);
    let setups: [(&str, DialogSetupFn); 26] = [
        ("FormatAxes", |a, _, _, _, g, _| {
            let d = a.document().clone();
            if let Some(gs) = d.graph_sheet(g) {
                a.canvas_dialogs.format_axes.sync_from_graph(gs, curvebook_ui_egui::dialogs_canvas::AxisTarget::Y);
            }
            a.canvas_dialogs.active_dialog = ActiveDialog::FormatAxes { sheet_id: g.clone() };
        }),
        ("AnalyzeSheet", |a, xy, _, _, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_analyze_dialog(&d, xy);
        }),
        ("DuplicateAnalysisPrompt", |a, _, col, res, _, _| {
            a.canvas_dialogs.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
                source_sheet_id: col.clone(),
                existing_result_ids: vec![res.clone()],
                pending_entry_id: String::from("stats.one_way_anova"),
            };
        }),
        ("EditAnalysisParameters", |a, _, _, res, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_edit_parameters(&d, res);
        }),
        ("ChangeDataAnalyzed", |a, _, _, res, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_change_data_analyzed(&d, res);
        }),
        ("ResultViewer", |a, _, _, res, _, _| {
            a.canvas_dialogs.active_dialog = ActiveDialog::ResultViewer { result_sheet_id: res.clone() };
        }),
        ("ConstantHookPicker", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_hook_picker(&d, g, Some(String::from("Mean")));
        }),
        ("ChangeGraphType", |a, _, _, _, g, _| {
            let d = a.document().clone();
            if let Some(gs) = d.graph_sheet(g) {
                a.canvas_dialogs.change_graph_type.open_for_graph(gs, &d);
            }
            a.canvas_dialogs.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: g.clone() };
        }),
        ("FormatGraph", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_format_graph(&d, g);
        }),
        ("PairwiseComparisons", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_pairwise_comparisons(&d, g);
        }),
        ("BubblePlotInspector", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_bubble_plot(&d, g);
        }),
        ("ApplyStyleFrom", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_apply_style_from(&d, g);
        }),
        ("PageLayoutComposer", |a, _, _, _, _, lay| {
            let d = a.document().clone();
            a.canvas_dialogs.open_page_layout_composer(&d, lay);
        }),
        ("ExportFigure", |a, _, _, _, g, _| {
            a.canvas_dialogs.open_export_figure(g);
        }),
        ("ConvertArchetype", |a, xy, _, _, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_convert_archetype(&d, xy);
        }),
        ("ImportReshape", |a, _, _, _, _, _| {
            a.canvas_dialogs.open_import_reshape();
        }),
        ("ExcludeWithReason", |a, xy, _, _, _, _| {
            a.canvas_dialogs.open_exclude_with_reason(xy.clone(), vec![curvebook_tools::GridCellCoord::new(0, 0, 0)]);
        }),
        ("FormatSelectedPoints", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_format_selected_points(&d, g, vec![(0, 0)]);
        }),
        ("PowerAndSampleSize", |a, _, _, _, _, _| {
            a.canvas_dialogs.open_power_wizard();
        }),
        ("Print", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_print_dialog(&d, Some(g), vec![String::from("System Default PDF")]);
        }),
        ("RandomizeGroups", |a, _, _, _, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_randomize_groups(&d);
        }),
        ("RandomSampleRows", |a, _, col, _, _, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_random_sample_rows(&d, Some(col));
        }),
        ("SimulateDistributions", |a, _, _, _, _, _| {
            a.canvas_dialogs.open_simulate_distributions();
        }),
        ("InsertModelEquation", |a, _, _, _, g, _| {
            let d = a.document().clone();
            a.canvas_dialogs.open_insert_model_equation(&d, g);
        }),
        ("CommandPalette", |a, _, _, _, _, _| {
            a.canvas_dialogs.open_command_palette();
        }),
        ("About", |a, _, _, _, _, _| {
            a.canvas_dialogs.active_dialog = ActiveDialog::About;
        }),
    ];

    let viewports = [
        ("en", ViewportMetrics::new(1280.0, 800.0, 1.0)),
        ("en", ViewportMetrics::new(1440.0, 900.0, 2.0)),
        ("ar", ViewportMetrics::new(1280.0, 800.0, 1.0)),
    ];

    for (locale, vp) in viewports {
        app.locale = String::from(locale);
        for (name, setup) in &setups {
            app.interaction.scroll_offsets.offsets.clear();
            setup(&mut app, &ds_xy, &ds_col, &res_id, &graph_id, &layout_id);
            if app.canvas_dialogs.active_dialog == ActiveDialog::None {
                return Err(format!("setup `{name}` left ActiveDialog::None"));
            }
            let tree = app.build_and_cache_layout_tree(vp);
            if tree.active_focus_scope != FocusScope::ModalDialog {
                return Err(format!("dialog `{name}` expected ModalDialog focus scope, got {:?}", tree.active_focus_scope));
            }
            assert_tree_clean(&mut app, vp, &format!("dialog_{name}_{locale}"))?;
        }
    }

    Ok(())
}

#[test]
fn r10_w5_integ_hooks_end_to_end_wiring() -> Result<(), String> {
    let mut app = build_rich_all_archetypes_app()?;
    let metrics = default_viewport();
    let graph_id = app.document().graph_sheets.first().ok_or("missing graph sheet")?.id.clone();

    // 1. `table.setVariableRole` `"auto"` restores `role_inferred: true`.
    let _ =
        app.execute_command("table.setVariableRole", json!({ "sheet_id": "ds-mv", "col": 0, "role": "categorical" })).map_err(|e| e.to_string())?;
    let auto_out =
        app.execute_command("table.setVariableRole", json!({ "sheet_id": "ds-mv", "col": 0, "role": "auto" })).map_err(|e| e.to_string())?;
    if auto_out.payload["role_inferred"].as_bool() != Some(true) {
        return Err(format!("expected role_inferred=true for role=\"auto\", got {}", auto_out.payload));
    }

    // 2. `ui.editParams.open` and parameterless `analysis.updateParams` routing.
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" })).map_err(|e| e.to_string())?;
    let ep_res = dispatch_ui_command(&mut app, "ui.editParams.open", &json!({}))?;
    if ep_res.is_none() || !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!("ui.editParams.open did not open EditAnalysisParameters: {ep_res:?}"));
    }
    let upd_res = dispatch_ui_command(&mut app, "analysis.updateParams", &json!({}))?;
    if upd_res.is_none() || !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        return Err(format!("parameterless analysis.updateParams did not open EditAnalysisParameters: {upd_res:?}"));
    }
    app.canvas_dialogs.active_dialog = ActiveDialog::None;

    // 3. `Cmd+T` across split panes: primary = ds-col (active), secondary = res-anova, focus on secondary.
    let mut driver = new_human_driver(app, metrics);
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    driver.app_mut().split.enabled = true;
    driver.app_mut().split.primary_sheet_id = Some(SheetId::new("ds-col"));
    driver.app_mut().split.secondary_sheet_id = Some(SheetId::new("res-anova"));
    driver.app_mut().focus_widget_id = Some(String::from("secondary.results.table"));
    press_cmd_shortcut(&mut driver, "t")?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { ref result_sheet_id } if result_sheet_id.as_str() == "res-anova")
    {
        return Err(format!(
            "Cmd+T on focused secondary ResultSheet pane did not open res-anova params: {:?}",
            driver.app().canvas_dialogs.active_dialog
        ));
    }
    driver.press_key("Escape", KeyModifiers::default())?;
    driver.app_mut().split.enabled = false;

    // 4. Space toggles checkbox, Escape closes nav.welcome.open, welcome.action.open_existing requests host open.
    driver.app_mut().nav.welcome.open = true;
    click_widget_center(&mut driver, &wid("welcome.action.open_existing"))?;
    // Verify Escape dismisses `nav.welcome.open`.
    driver.app_mut().nav.welcome.open = true;
    let esc_out = driver.press_key("Escape", KeyModifiers::default())?;
    if !esc_out.handled || driver.app().nav.welcome.open {
        return Err(String::from("Escape did not close nav.welcome.open"));
    }

    // 5. Space key (`" "`) toggles dialog checkbox when focused (`edit.assume_gaussian`).
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "res-anova" })).map_err(|e| e.to_string())?;
    press_cmd_shortcut(&mut driver, "t")?;
    let was_gaussian = driver.app().canvas_dialogs.edit_params.assume_gaussian;
    driver.app_mut().focus_widget_id = Some(String::from("edit.assume_gaussian"));
    driver.press_key(" ", KeyModifiers::default())?;
    if driver.app().canvas_dialogs.edit_params.assume_gaussian == was_gaussian {
        return Err(String::from("Spacebar (`\" \"`) did not toggle focused edit.assume_gaussian checkbox"));
    }
    driver.press_key("Escape", KeyModifiers::default())?;

    // 6. FloatingNote full overlay & pointer drag (`floating_note.ds-xy.note-xy.drag_handle` -> note.set_pos).
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).map_err(|e| e.to_string())?;
    let drag_id = wid("floating_note.ds-xy.note-xy.drag_handle");
    let card_id = wid("floating_note.ds-xy.note-xy.card");
    let before_rect = driver.layout_tree().node(card_id.as_str()).ok_or("missing floating_note.ds-xy.note-xy.card")?.bounds.rect_px;
    let before_pos = driver
        .app()
        .view_shards
        .nav_notes
        .note_positions_pt
        .get("ds-xy::note-xy")
        .copied()
        .or_else(|| {
            driver.app().document().data_sheet(&SheetId::new("ds-xy")).and_then(|d| d.floating_notes.first()).map(|n| [n.bounds_pt.x, n.bounds_pt.y])
        })
        .ok_or("missing floating note on ds-xy")?;
    let drag_center = human_wf_common::widget_hit_center(&mut driver, &drag_id)?;
    let _ = driver.drag_logical(drag_center[0], drag_center[1], drag_center[0] + 44.0, drag_center[1] + 28.0)?;
    let after_pos = driver
        .app()
        .view_shards
        .nav_notes
        .note_positions_pt
        .get("ds-xy::note-xy")
        .copied()
        .ok_or("floating_note drag did not record updated note_positions_pt")?;
    let after_rect = driver.layout_tree().node(card_id.as_str()).ok_or("missing floating_note card after drag")?.bounds.rect_px;
    if (after_pos[0] - before_pos[0]) < 10.0
        || (after_pos[1] - before_pos[1]) < 10.0
        || after_rect.x <= before_rect.x
        || after_rect.y <= before_rect.y
    {
        return Err(format!(
            "floating_note drag did not update position/layout: before_pos={before_pos:?}, after_pos={after_pos:?}, before_rect={before_rect:?}, after_rect={after_rect:?}"
        ));
    }

    // 7. Cmd+E (no Shift) toggles exclusion on selected grid cell.
    click_widget_center(&mut driver, &wid("grid.cell.0.0.0"))?;
    let ex_before =
        driver.app().document().data_sheet(&SheetId::new("ds-xy")).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).is_some_and(|c| c.excluded);
    press_cmd_shortcut(&mut driver, "e")?;
    let ex_after =
        driver.app().document().data_sheet(&SheetId::new("ds-xy")).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).is_some_and(|c| c.excluded);
    if ex_after == ex_before {
        return Err(String::from("Cmd+E did not toggle cell exclusion on active DataSheet"));
    }

    // 8. Secondary click across all 5 context-menu targets & ZLayer::ContextMenu click-away dismissal.
    let _ = driver
        .app_mut()
        .execute_command(
            "table.pasteLink",
            json!({
                "target_sheet_id": "ds-xy",
                "source_sheet_id": "ds-col",
                "source_kind": "data",
                "anchor": { "row": 1, "col": 0, "subcol": 0 }
            }),
        )
        .map_err(|e| e.to_string())?;
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "ds-xy" })).map_err(|e| e.to_string())?;

    for target_wid in ["grid.cell.0.0.0", "grid.col_title.0", "grid.cell.1.0.0", "nav.sheet.ds-col"] {
        let pos = human_wf_common::widget_hit_center(&mut driver, &wid(target_wid))?;
        let _ = dispatch_ui_event(
            driver.app_mut(),
            UiInputEvent::PointerDown { x: pos[0], y: pos[1], button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
            metrics,
        )?;
        let _ = dispatch_ui_event(
            driver.app_mut(),
            UiInputEvent::PointerUp { x: pos[0], y: pos[1], button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
            metrics,
        )?;
        if driver.app().interaction.context_menu_target.is_none() {
            return Err(format!("secondary click on `{target_wid}` failed to open context menu"));
        }
        // Left-clicking outside context menu on workspace dismisses popup (ZLayer < ContextMenu).
        let _ = driver.click_logical(pos[0] - 4.0, pos[1] - 4.0)?;
        if driver.app().interaction.context_menu_target.is_some() {
            return Err(format!("workspace click did not dismiss context menu opened from `{target_wid}`"));
        }
    }

    // 9. DuplicateAnalysisPrompt resolved cleanly via modal.ok (ModifyExisting & CreateNew).
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": "ds-col" })).map_err(|e| e.to_string())?;
    let count_before_dup = driver.app().document().result_sheets.len();
    click_widget_center(&mut driver, &wid("toolbar.analyze"))?;
    let anova_id = wid("analyze.entry.stats.one_way_anova");
    scroll_until_widget_visible(&mut driver, &wid("analyze.entries_list"), &anova_id, 140.0, 10)?;
    click_widget_center(&mut driver, &anova_id)?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if !matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::DuplicateAnalysisPrompt { .. }) {
        return Err(format!("expected DuplicateAnalysisPrompt, got {:?}", driver.app().canvas_dialogs.active_dialog));
    }
    click_widget_center(&mut driver, &wid("analyze_picker.dup.modify_existing"))?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().result_sheets.len() != count_before_dup {
        return Err(String::from("DuplicateAnalysisPrompt ModifyExisting should reuse existing ResultSheet"));
    }

    // 10. ImportReshape multiline paste + modal.ok commit.
    let _ = dispatch_ui_command(driver.app_mut(), "ui.import.reshape.open", &json!({}))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::ImportReshape {
        return Err(String::from("ImportReshape dialog did not open"));
    }
    click_widget_center(&mut driver, &wid("import_paste.source.clipboard"))?;
    scroll_until_widget_visible(&mut driver, &wid("import_paste.panel.source"), &wid("import_paste.input.csv"), 80.0, 12)?;
    click_widget_center(&mut driver, &wid("import_paste.input.csv"))?;
    press_cmd_shortcut(&mut driver, "a")?;
    let _ = driver.press_key("Backspace", KeyModifiers::default())?;
    let _ = driver.type_text("A,B\n1,2\n3,4")?;
    let ds_before_imp = driver.app().document().data_sheets.len();
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().document().data_sheets.len() != ds_before_imp + 1 {
        return Err(String::from("modal.ok on ImportReshape did not create new DataSheet"));
    }

    // 11. CommandPalette ArrowDown / ArrowUp while search input is focused.
    press_cmd_shortcut(&mut driver, "k")?;
    click_widget_center(&mut driver, &wid("palette.query"))?;
    let _ = driver.type_text("export")?;
    let sel0 = driver.app().canvas_dialogs.command_palette.selected_index;
    driver.press_key("ArrowDown", KeyModifiers::default())?;
    let sel1 = driver.app().canvas_dialogs.command_palette.selected_index;
    if sel1 == sel0 {
        return Err(String::from("ArrowDown in CommandPalette search input did not advance selected item"));
    }
    driver.press_key("ArrowUp", KeyModifiers::default())?;
    driver.press_key("Escape", KeyModifiers::default())?;

    // 12. Graph canvas point click updates `inspector.selected_point_summary` & ExportFigure extra options confirm.
    let _ = driver.app_mut().execute_command("sheet.select", json!({ "sheet_id": graph_id.as_str() })).map_err(|e| e.to_string())?;
    if let Ok(scene) = driver.app().session().render_sheet_scene(&graph_id)
        && let Some(pt) = scene.plotted_points.first()
        && let Some(paper) = driver.layout_tree().scene_papers.first().cloned()
    {
        let px = f64::from(paper.paper.x) + (pt.center_pt.x / scene.width_pt) * f64::from(paper.paper.width);
        let py = f64::from(paper.paper.y) + (pt.center_pt.y / scene.height_pt) * f64::from(paper.paper.height);
        let [lx, ly] = metrics.physical_to_logical_px([px, py]);
        let _ = driver.click_logical(lx, ly)?;
        if driver.app().view_shards.inspector.selected_point_summary.is_empty() {
            return Err(String::from("clicking canvas data point did not update inspector.selected_point_summary"));
        }
    }
    // Check secondary click on GraphSheet canvas opens graph_object context menu too.
    let canvas_id = WidgetId::from(format!("canvas.surface.main.{}", graph_id.as_str()));
    let cpos = human_wf_common::widget_hit_center(&mut driver, &canvas_id)?;
    let _ = dispatch_ui_event(
        driver.app_mut(),
        UiInputEvent::PointerDown { x: cpos[0], y: cpos[1], button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    let _ = dispatch_ui_event(
        driver.app_mut(),
        UiInputEvent::PointerUp { x: cpos[0], y: cpos[1], button: PointerButton::Secondary, modifiers: KeyModifiers::default() },
        metrics,
    )?;
    if driver.app().interaction.context_menu_target.is_none() {
        return Err(String::from("secondary click on canvas.primary failed to open graph_object context menu"));
    }
    driver.press_key("Escape", KeyModifiers::default())?;

    // Open ExportFigure, toggle SVG + transparent background, confirm cleanly.
    driver.app_mut().canvas_dialogs.export_figure.format = ExportFigureFormat::Svg;
    click_widget_center(&mut driver, &wid("toolbar.export_svg"))?;
    click_widget_center(&mut driver, &wid("export.fmt.svg"))?;
    click_widget_center(&mut driver, &wid("export.transparent_bg"))?;
    click_widget_center(&mut driver, &wid("modal.ok"))?;
    if driver.app().canvas_dialogs.active_dialog != ActiveDialog::None {
        return Err(String::from("ExportFigure did not close after modal.ok"));
    }

    assert_tree_clean(driver.app_mut(), metrics, "r10_w5_integ_hooks_end_to_end")?;
    Ok(())
}
