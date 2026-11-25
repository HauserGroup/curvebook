#![forbid(unsafe_code)]

//! `R13.8` Phase B2 Part 1 (`R13.2`, `R13.3`, `R13.5`) desktop `egui 0.37` verification tests.

use crate::common::{TestRes, select_sheet_and_settle, sync_app_to_driver, widget_center, widget_rect};
use crate::suite_helpers::{click_all, make_sync_driver, new_shell_app, open_menu_dialog, run_cmd, run_sheet_cmd, verify_r13_matrix_and_accesskit};
use curvebook_doc::{
    Cell, DataSheet, ErrorBarMetric, FramePreset, GraphicalDataObject, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapLabelSpec,
    HeatmapRowLabelMode, LayoutItem, ProjectDocument, RegressionOverlaySpec, ReplicateLineMode, SheetId, SubcolumnMode, SurvivalDisplayMetric,
    SurvivalTable, TableArchetype, TickDirection, TickLabelFormat, XyTable,
};
use curvebook_palette::Rgba8;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::UiIntent;
use curvebook_ui_egui::egui_shell::transform_modals::TransformModalKind;
use curvebook_ui_egui::inspect::ZLayer;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton};
use serde_json::json;

fn build_seeded_project() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R13.8 Part 1 Suite").map_err(|e| e.to_string())?;
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).map_err(|e| e.to_string())?;
    xy.row_titles = vec!["Baseline".into(), "Mid".into(), "Peak".into(), "Late".into()];
    xy.row_titles_expanded = true;
    xy.data_sets[0].title = "Vehicle".into();
    xy.data_sets[1].title = "Treated".into();
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64) * 5.0);
        xy.data_sets[0].cells[r][1] = Cell::number(11.0 + (r as f64) * 4.5);
        xy.data_sets[1].cells[r][0] = Cell::number(14.0 + (r as f64) * 6.0);
        xy.data_sets[1].cells[r][1] = Cell::number(15.5 + (r as f64) * 5.5);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Assay Kinetics", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut surv = SurvivalTable::new(2, 4).map_err(|e| e.to_string())?;
    surv.cohorts[0].title = "Control".into();
    surv.cohorts[1].title = "Drug".into();
    for (r, (t, e0, e1)) in [(5.0, 1.0, 0.0), (10.0, 1.0, 1.0), (15.0, 0.0, 0.0), (20.0, 1.0, 1.0)].into_iter().enumerate() {
        surv.times[r] = Cell::number(t);
        surv.cohorts[0].cells[r][0] = Cell::number(e0);
        surv.cohorts[1].cells[r][0] = Cell::number(e1);
    }
    doc.add_data_sheet(DataSheet::new("surv-1", "Cohort Survival", TableArchetype::Survival(surv)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut app = new_shell_app(doc);
    for (sid, ds, kind, title) in [
        ("g-xy", "xy-1", "xy", "Primary XY"),
        ("g-xy-2", "xy-1", "xy", "Secondary XY"),
        ("g-surv", "surv-1", "survival", "Survival KM"),
        ("g-heat", "xy-1", "heatmap", "Assay Heatmap"),
    ] {
        run_cmd(&mut app, "graph.create", json!({ "sheet_id": sid, "data_sheet_id": ds, "kind": kind, "title": title }))?;
    }
    let res_id = run_sheet_cmd(&mut app, "analysis.run", json!({ "source_sheet_id": "xy-1", "kind": "linear_regression", "sheet_id": "res-lin" }))?;
    run_cmd(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "series_index": 0,
            "regression_overlay": RegressionOverlaySpec {
                result_sheet_id: res_id,
                sample_segments: 256,
                show_confidence_band: false,
                show_prediction_band: false,
                shaded_ribbon: true,
            },
        }),
    )?;
    run_cmd(&mut app, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }))?;
    run_cmd(&mut app, "layout.addPanel", json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy", "panel_id": "p-a" }))?;
    run_cmd(
        &mut app,
        "layout.addPanel",
        json!({ "sheet_id": "lay-1", "kind": "graph", "graph_sheet_id": "g-xy-2", "panel_id": "p-b", "scale_percent": 70.0 }),
    )?;
    app.sync_views();
    Ok(app)
}

fn series_alpha(gdo: &GraphicalDataObject) -> u8 {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => marker.map(|m| m.fill_color.a).or_else(|| line.map(|l| l.color.a)).unwrap_or(255),
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => fill_color.a,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color.a,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => line_color.a,
        GraphicalDataObject::PartsOfWhole { .. } => 255,
    }
}

#[test]
fn r13_2_format_graph_opacity_overlays_single_undo_survival_row_titles_and_spaghetti() -> TestRes {
    let mut drv = make_sync_driver(build_seeded_project()?, 1280.0, 800.0);

    // 1. Format Graph: Opacity 50%, hide raw data & fitted curve, apply globally (single undo).
    open_menu_dialog(&mut drv, "ui.graph.format.open", json!({ "sheet_id": "g-xy" }))?;
    click_all(
        &mut drv,
        &["format_graph.alpha.50", "format_graph.show_raw_data", "format_graph.show_fitted_curve", "format_graph.apply_globally", "modal.ok"],
    )?;
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    {
        let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?;
        assert!(!gs.spec.series[0].show_raw_data && !gs.spec.series[1].show_raw_data);
        assert!(gs.spec.series[0].regression_overlay.is_none());
        assert!((120..=135).contains(&series_alpha(&gs.spec.series[0].gdo)));
        assert!((120..=135).contains(&series_alpha(&gs.spec.series[1].gdo)));
    }

    run_cmd(&mut drv.state_mut().app, "edit.undo", json!({}))?;
    {
        let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after undo")?;
        assert!(gs.spec.series[0].show_raw_data && gs.spec.series[1].show_raw_data && gs.spec.series[0].regression_overlay.is_some());
        assert_eq!((series_alpha(&gs.spec.series[0].gdo), series_alpha(&gs.spec.series[1].gdo)), (255, 255));
    }

    // 2. TrailingStudio live GraphInspector Data & Canvas controls.
    drv.state_mut().app.nav.inspector_open = true;
    drv.state_mut().app.egui_views.trailing_collapsed_rail = false;
    select_sheet_and_settle(&mut drv, "g-xy")?;
    click_all(
        &mut drv,
        &["inspector.graph.alpha.25", "inspector.graph.show_raw_data", "inspector.graph.tab.canvas", "inspector.graph.paper_fill.cream"],
    )?;
    {
        let gs = drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy after studio")?;
        assert!(!gs.spec.series[0].show_raw_data && (58..=70).contains(&series_alpha(&gs.spec.series[0].gdo)));
        assert_eq!(gs.spec.paper_fill, Some(Rgba8::rgb(255, 251, 235)));
    }

    // 3. Change Graph Type survival metric / point-to-point / error display.
    open_menu_dialog(&mut drv, "ui.changeGraphType.open", json!({ "sheet_id": "g-surv" }))?;
    click_all(&mut drv, &["graph_type.survival.metric.pct_event", "graph_type.survival.point_to_point", "graph_type.survival.err.sem", "modal.ok"])?;
    match drv.state().app.document().graph_sheet(&SheetId::new("g-surv")).ok_or("missing g-surv")?.spec.series[0].gdo {
        GraphicalDataObject::KaplanMeierStep { display_metric, staircase, error_display, show_greenwood_ci, .. } => {
            assert_eq!(display_metric, SurvivalDisplayMetric::PercentCumulativeEvent);
            assert!(!staircase && error_display == Some(ErrorBarMetric::Sem) && show_greenwood_ci);
        }
        ref other => return Err(format!("expected KaplanMeierStep, got {other:?}")),
    }

    // 4. View > Row Titles toggle (`ui.view.rowTitles.toggle`) synced immediately to `data_grid`.
    select_sheet_and_settle(&mut drv, "xy-1")?;
    open_menu_dialog(&mut drv, "ui.view.rowTitles.toggle", json!({}))?;
    assert_eq!(drv.state().app.grid.row_titles_collapsed_override, Some(true));
    assert!(drv.state().views.column_grid.row_titles_collapsed);
    assert!(drv.frame_meta().find_by_widget_id("grid.row_titles.toggle").is_some());

    // 5. Spaghetti trajectory hover HUD (`Rep N`).
    run_cmd(
        &mut drv.state_mut().app,
        "graph.setSeries",
        json!({ "sheet_id": "g-xy", "series_index": 0, "show_raw_data": true, "replicate_lines": ReplicateLineMode::IndividualReplicateTrajectories }),
    )?;
    select_sheet_and_settle(&mut drv, "g-xy")?;
    let scene = drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy")).map_err(|e| e.to_string())?;
    let first_pt = scene.plotted_points.first().ok_or("missing plotted point for spaghetti hover")?;
    let canvas_rect = widget_rect(&drv, "canvas.viewport")?;
    let sx = ((canvas_rect.width() - 32.0).max(80.0) / (scene.width_pt.max(1.0) as f32))
        .min((canvas_rect.height() - 32.0).max(80.0) / (scene.height_pt.max(1.0) as f32));
    let paper_min =
        egui::pos2(canvas_rect.center().x - (scene.width_pt as f32) * sx * 0.5, canvas_rect.center().y - (scene.height_pt as f32) * sx * 0.5);
    drv.hover_at(egui::pos2(paper_min.x + (first_pt.center_pt.x as f32) * sx, paper_min.y + (first_pt.center_pt.y as f32) * sx))?;
    let readout = drv.state().views.canvas_view.hover_readout.as_ref().ok_or("expected spaghetti hover readout")?;
    assert!(readout.label.contains("Rep ") && readout.value_summary.contains("Rep "), "got {:?}", readout);
    Ok(())
}

#[test]
fn r13_3_create_graph_gallery_multiselect_apply_style_normalize_and_heatmap_labels() -> TestRes {
    let mut drv = make_sync_driver(build_seeded_project()?, 1280.0, 800.0);
    let initial_graphs = drv.state().app.document().graph_sheets.len();

    // 1. Create New Graph dialog (`one_per_data_set` + `y_axis_title`).
    open_menu_dialog(&mut drv, "ui.graph.create.open", json!({ "data_sheet_id": "xy-1" }))?;
    click_all(&mut drv, &["create_graph.kind.xy", "create_graph.one_per_data_set", "create_graph.y_axis_title"])?;
    drv.type_text("Signal (RFU)")?;
    drv.click_widget_id("modal.ok")?;
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(drv.state().app.document().graph_sheets.len(), initial_graphs + 2);
    let created_last_id = drv.state().app.document().graph_sheets.last().ok_or("missing created graph")?.id.clone();
    assert_eq!(drv.state().app.document().graph_sheets.last().ok_or("missing created graph")?.spec.y_axis.title, "Signal (RFU)");

    // 2. Portfolio Gallery `Cmd/Ctrl+Click` + `Shift+Click` -> batch `ApplyStyleFrom`.
    run_cmd(&mut drv.state_mut().app, "graph.setSpec", json!({ "sheet_id": "g-xy", "frame_preset": "PlainBoxed" }))?;
    open_menu_dialog(&mut drv, "ui.gallery.toggle", json!({ "open": true, "folder": "graphs" }))?;
    let p_xy2 = widget_center(&drv, "gallery.card.g-xy-2")?;
    drv.click_at_button(p_xy2, PointerButton::Primary, KeyModifiers { ctrl: true, ..Default::default() })?;
    let p_last = widget_center(&drv, &format!("gallery.card.{}", created_last_id.as_str()))?;
    drv.click_at_button(p_last, PointerButton::Primary, KeyModifiers { shift: true, ..Default::default() })?;
    assert!(drv.state().app.nav.selected_gallery_ids().len() >= 2, "expected multi-selection via Shift+Click");
    drv.click_widget_id("gallery.apply_style_from")?;
    assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
    click_all(&mut drv, &["apply_style.donor.0", "apply_style.target.select_all", "modal.ok"])?;
    assert_eq!(drv.state().app.document().graph_sheet(&SheetId::new("g-xy-2")).ok_or("g-xy-2")?.spec.frame_preset, FramePreset::PlainBoxed);

    // 3. Standalone Normalize Transform modal `create_new_graph` checkbox.
    let (graphs_before, results_before) = (drv.state().app.document().graph_sheets.len(), drv.state().app.document().result_sheets.len());
    select_sheet_and_settle(&mut drv, "xy-1")?;
    drv.state_mut().intents.push(UiIntent::OpenTransformModal { sheet_id: SheetId::new("xy-1"), kind: TransformModalKind::Normalize });
    drv.run_settled();
    drv.run_settled();
    click_all(&mut drv, &["edit.transform.normalize.create_graph", "modal.ok"])?;
    assert_eq!(drv.state().app.document().result_sheets.len(), results_before + 1);
    assert_eq!(drv.state().app.document().graph_sheets.len(), graphs_before + 1);

    // 4. HeatmapLabelSpec controls in Format Graph.
    open_menu_dialog(&mut drv, "ui.graph.format.open", json!({ "sheet_id": "g-heat" }))?;
    click_all(
        &mut drv,
        &[
            "edit.tab.format_graph.heatmap_labels",
            "format_graph.heatmap.row_labels.numbers",
            "format_graph.heatmap.col_labels.letters",
            "format_graph.heatmap.angle.90",
            "modal.ok",
        ],
    )?;
    assert_eq!(
        drv.state().app.document().graph_sheet(&SheetId::new("g-heat")).ok_or("missing g-heat")?.spec.heatmap_labels,
        HeatmapLabelSpec {
            row_labels: HeatmapRowLabelMode::RowNumbers,
            column_labels: HeatmapColumnLabelMode::ColumnLetters,
            column_angle: HeatmapColumnLabelAngle::Vertical90,
            ..HeatmapLabelSpec::default()
        }
    );
    Ok(())
}

#[test]
fn r13_5_axes_ticks_paper_fill_equalize_scaling_studio_zorder_and_context_menu() -> TestRes {
    let mut drv = make_sync_driver(build_seeded_project()?, 1280.0, 800.0);

    // 1. Format Axes: TickDirection::Inside, show_tick_labels: false, AbsoluteDecimal.
    open_menu_dialog(&mut drv, "ui.graph.axes.open", json!({ "sheet_id": "g-xy", "target_axis": "y" }))?;
    click_all(&mut drv, &["edit.tab.axes.y", "axes.tick_format.abs_decimal", "axes.tick_dir.inside", "axes.show_tick_labels", "modal.ok"])?;
    {
        let y = &drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?.spec.y_axis;
        assert_eq!((y.tick_format, y.tick_direction, y.show_tick_labels), (TickLabelFormat::AbsoluteDecimal, TickDirection::Inside, false));
    }

    // 2. Format Graph: Page Background Fill (`Transparent` -> None).
    open_menu_dialog(&mut drv, "ui.graph.format.open", json!({ "sheet_id": "g-xy" }))?;
    click_all(&mut drv, &["edit.tab.format_graph.settings", "format_graph.paper_fill.transparent", "modal.ok"])?;
    assert_eq!(drv.state().app.document().graph_sheet(&SheetId::new("g-xy")).ok_or("g-xy")?.spec.paper_fill, None);

    // 3. LayoutSheet: Trailing Studio `Equalize Scaling...`, Z-order chips, and right-click ZLayer::ContextMenu.
    drv.state_mut().app.nav.inspector_open = true;
    drv.state_mut().app.egui_views.trailing_collapsed_rail = false;
    select_sheet_and_settle(&mut drv, "lay-1")?;
    drv.click_widget_id("inspector.layout.equalize_scaling")?;
    assert!(matches!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::EqualizeScaling { .. }));
    click_all(&mut drv, &["equalize_scaling.mode.explicit", "equalize_scaling.percent.75", "modal.ok"])?;
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    {
        let sub0 = drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy")).map_err(|e| e.to_string())?;
        let sub1 = drv.state().app.session().render_sheet_scene(&SheetId::new("g-xy-2")).map_err(|e| e.to_string())?;
        let ls = drv.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?;
        let [LayoutItem::LiveGraph { bounds_pt: b0, scale_percent: s0, .. }, LayoutItem::LiveGraph { bounds_pt: b1, scale_percent: s1, .. }] =
            ls.spec.items.as_slice()
        else {
            return Err("expected 2 LiveGraph items on lay-1".into());
        };
        let eff0 = (b0.width / sub0.width_pt.max(1.0)).min(b0.height / sub0.height_pt.max(1.0)) * (*s0 / 100.0);
        let eff1 = (b1.width / sub1.width_pt.max(1.0)).min(b1.height / sub1.height_pt.max(1.0)) * (*s1 / 100.0);
        assert!((eff0 - 0.75).abs() < 1e-4 && (eff1 - 0.75).abs() < 1e-4, "expected 0.75 effective scale, got ({eff0}, {eff1})");
    }

    // Studio Z-order chip -> Bring panel 0 (`p-a`) to front, then ContextMenu -> Bring `p-b` back to front.
    drv.state_mut().app.egui_views.canvas_view.selected_panel_index = Some(0);
    sync_app_to_driver(&mut drv);
    drv.click_widget_id("inspector.layout.zorder.front")?;
    let order =
        |d: &curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver<'_, crate::common::ShellTestState>| -> Result<(String, String), String> {
            let ls = d.state().app.document().layout_sheet(&SheetId::new("lay-1")).ok_or("missing lay-1")?;
            Ok((ls.spec.items[0].id().to_owned(), ls.spec.items[1].id().to_owned()))
        };
    assert_eq!(order(&drv)?, ("p-b".into(), "p-a".into()));

    let p0_center = widget_center(&drv, "canvas.layout_panel.0")?;
    drv.click_at_button(p0_center, PointerButton::Secondary, KeyModifiers::default())?;
    assert_eq!(drv.frame_meta().find_by_widget_id("canvas.layout_ctx.front").ok_or("missing layout_ctx.front")?.z_layer, ZLayer::ContextMenu);
    drv.click_widget_id("canvas.layout_ctx.front")?;
    assert_eq!(order(&drv)?, ("p-a".into(), "p-b".into()));
    Ok(())
}

#[test]
fn r13_8_p1_14_beta_locales_3_themes_2_sizes_2_dpis_and_modal_capture() -> TestRes {
    let base_app = build_seeded_project()?;
    let create_case = ("ui.graph.create.open", json!({ "data_sheet_id": "xy-1" }));
    let eq_case = ("ui.layout.equalizeScaling.open", json!({ "sheet_id": "lay-1" }));
    verify_r13_matrix_and_accesskit(base_app, &[create_case.clone(), eq_case], &[create_case], |app| {
        let doc = app.document().clone();
        app.canvas_dialogs.open_equalize_scaling(&doc, &SheetId::new("lay-1"))
    })
}
