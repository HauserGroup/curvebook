#![forbid(unsafe_code)]

//! `R13.10` integration tests (`EguiHumanPathDriver`) for `Format Graph` and `Graph Inspector` controls.

use crate::common::{ShellTestState, TestRes, fill_col_values, select_sheet_and_settle, widget_rect};
use crate::suite_helpers::{assert_widgets, build_mv_sheet, click_all, make_sync_driver, new_shell_app, open_format_graph, run_cmd};
use curvebook_doc::{
    CartesianOrientation, Cell, ColumnDesignHint, ColumnTable, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject, GroupedTable,
    ProjectDocument, ReplicateLineMode, SheetId, SubcolumnMode, SummaryMeasure, TableArchetype, VariableRole, XyTable,
};
use curvebook_palette::{MarkerStyle, Rgba8};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::EguiHumanPathDriver;
use serde_json::json;

fn set_scrub_field_text(drv: &mut EguiHumanPathDriver<'_, ShellTestState>, wid: &str, right_offset_pt: f32, text: &str) -> TestRes {
    let r = widget_rect(drv, wid)?;
    drv.click_at(egui::pos2((r.right() - right_offset_pt).max(r.left() + 6.0), r.center().y))?;
    for _ in 0..12 {
        drv.press_egui_key(egui::Key::Backspace, egui::Modifiers::NONE);
    }
    drv.type_text(text)?;
    drv.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    drv.run_settled();
    Ok(())
}

fn seed_r13_10_document() -> Result<UiApp, String> {
    let mut doc = ProjectDocument::new("R13.10 Format Graph Controls").map_err(|e| e.to_string())?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).map_err(|e| e.to_string())?;
    grp.row_titles = vec!["Vehicle".into(), "Treated".into()];
    grp.data_sets[0].title = "Low Dose".into();
    grp.data_sets[1].title = "High Dose".into();
    for g in 0..2 {
        for r in 0..2 {
            for sub in 0..2 {
                grp.data_sets[g].cells[r][sub] = Cell::number(10.0 + (g * 5 + r * 3 + sub) as f64);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("grp-1", "Grouped Assay", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).map_err(|e| e.to_string())?;
    fill_col_values(&mut col.data_sets[0], "Control", &[12.0, 13.0, 11.5]);
    fill_col_values(&mut col.data_sets[1], "Treatment", &[18.0, 19.5, 17.0]);
    doc.add_data_sheet(DataSheet::new("col-1", "Column Bars", TableArchetype::Column(col)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 4).map_err(|e| e.to_string())?;
    xy.data_sets[0].title = "Arm A".into();
    xy.data_sets[1].title = "Arm B".into();
    for r in 0..4 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number(5.0 + r as f64 * 2.0);
        xy.data_sets[0].cells[r][1] = Cell::number(5.4 + r as f64 * 2.2);
        xy.data_sets[1].cells[r][0] = Cell::number(100.0 + r as f64 * 40.0);
        xy.data_sets[1].cells[r][1] = Cell::number(108.0 + r as f64 * 42.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "Dual Axis Kinetics", TableArchetype::Xy(xy)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    doc.add_data_sheet(build_mv_sheet("mv-1", "Cohort MV", &[("V1", VariableRole::Continuous), ("V2", VariableRole::Categorical)], 3, |_| {})?)
        .map_err(|e| e.to_string())?;

    let mut app = new_shell_app(doc);
    for (sid, ds, kind, title) in [
        ("g-grp", "grp-1", "grouped", "Grouped Graph"),
        ("g-col", "col-1", "column", "Column Bars"),
        ("g-xy", "xy-1", "xy", "Dual Axis XY"),
        ("g-forest", "mv-1", "forest", "Forest Plot"),
    ] {
        run_cmd(&mut app, "graph.create", json!({ "sheet_id": sid, "data_sheet_id": ds, "kind": kind, "title": title }))?;
    }
    run_cmd(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-col",
            "all_series": true,
            "gdo": GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::Mean,
                fill_color: Rgba8::rgb(40, 120, 220),
                border_color: Rgba8::BLACK,
                border_width_pt: 1.0,
                replicate_overlay: None,
            },
        }),
    )?;
    run_cmd(
        &mut app,
        "graph.setSeries",
        json!({
            "sheet_id": "g-xy",
            "all_series": true,
            "gdo": GraphicalDataObject::ScatterAndLine {
                marker: Some(MarkerStyle::default()),
                line: None,
                area_fill: None,
                replicate_lines: ReplicateLineMode::SummaryCenter,
            },
        }),
    )?;
    Ok(app)
}

#[test]
fn gwf01_orientation_in_format_graph_and_graph_inspector() -> TestRes {
    let mut drv = make_sync_driver(seed_r13_10_document()?, 1280.0, 800.0);
    let gid = SheetId::new("g-grp");
    open_format_graph(&mut drv, &gid)?;
    click_all(&mut drv, &["edit.tab.format_graph.series", "format_graph.orientation.horizontal", "modal.ok"])?;
    assert_eq!(drv.state().last_intent_error, None);
    assert_eq!(drv.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(drv.state().app.document().graph_sheet(&gid).ok_or("missing g-grp")?.spec.orientation, CartesianOrientation::Horizontal);

    drv.state_mut().app.nav.inspector_open = true;
    drv.state_mut().app.egui_views.trailing_collapsed_rail = false;
    select_sheet_and_settle(&mut drv, "g-grp")?;
    assert_widgets(&drv, &["inspector.graph.orientation.horizontal"])?;
    drv.click_widget_id("inspector.graph.orientation.vertical")?;
    assert_eq!(drv.state().app.document().graph_sheet(&gid).ok_or("missing g-grp")?.spec.orientation, CartesianOrientation::Vertical);
    Ok(())
}

#[test]
fn gwf02_show_symbols_replicate_lines_and_retopologize_preserve() -> TestRes {
    let mut drv = make_sync_driver(seed_r13_10_document()?, 1280.0, 800.0);
    let gid = SheetId::new("g-xy");
    open_format_graph(&mut drv, &gid)?;
    assert_widgets(
        &drv,
        &["format_graph.show_symbols", "format_graph.replicate_lines.summary_center", "format_graph.replicate_lines.individual_trajectories"],
    )?;
    click_all(&mut drv, &["format_graph.show_symbols", "format_graph.replicate_lines.individual_trajectories", "modal.ok"])?;
    let assert_scatter = |app: &UiApp, expect_marker: bool| -> TestRes {
        match &app.document().graph_sheet(&gid).ok_or("missing g-xy")?.spec.series[0].gdo {
            GraphicalDataObject::ScatterAndLine { marker, replicate_lines, .. } => {
                assert_eq!(marker.is_some(), expect_marker);
                assert_eq!(*replicate_lines, ReplicateLineMode::IndividualReplicateTrajectories);
                Ok(())
            }
            other => Err(format!("expected ScatterAndLine, got {other:?}")),
        }
    };
    assert_scatter(&drv.state().app, false)?;

    open_format_graph(&mut drv, &gid)?;
    assert!(!drv.state().app.canvas_dialogs.format_graph.show_symbols);
    click_all(&mut drv, &["format_graph.show_symbols", "modal.ok"])?;
    assert_scatter(&drv.state().app, true)?;

    // Reopening Format Graph and committing intents must preserve `replicate_lines`.
    open_format_graph(&mut drv, &gid)?;
    drv.state_mut().app.canvas_dialogs.format_graph.show_symbols = true;
    let intents = drv.state().app.canvas_dialogs.format_graph.commit_intents();
    for intent in intents {
        run_cmd(&mut drv.state_mut().app, &intent.command_id, intent.params)?;
    }
    assert_scatter(&drv.state().app, true)
}

#[test]
fn gwf17_series_select_right_y2_and_horizontal_nudge() -> TestRes {
    let mut drv = make_sync_driver(seed_r13_10_document()?, 1280.0, 800.0);
    let gid = SheetId::new("g-xy");
    open_format_graph(&mut drv, &gid)?;
    drv.click_widget_id("edit.tab.format_graph.series")?;
    assert_widgets(
        &drv,
        &[
            "format_graph.series_select.0",
            "format_graph.series_select.1",
            "format_graph.series_axis.left_y",
            "format_graph.series_axis.right_y2",
            "format_graph.series_offset_x",
        ],
    )?;
    click_all(&mut drv, &["format_graph.series_select.1", "format_graph.series_axis.right_y2"])?;
    assert_eq!(drv.state().app.canvas_dialogs.format_graph.selected_series_index, 1);
    set_scrub_field_text(&mut drv, "format_graph.series_offset_x", 24.0, "0.25")?;
    drv.click_widget_id("modal.ok")?;
    let gs = drv.state().app.document().graph_sheet(&gid).ok_or("missing g-xy")?;
    assert!(!gs.spec.series[0].bind_to_right_y2 && gs.spec.series[1].bind_to_right_y2);
    assert!((gs.spec.series[1].offset.x_data - 0.25).abs() < 1e-6);
    Ok(())
}

#[test]
fn wf64_bar_border_apply_globally_and_wf63_forest_pooled_diamond() -> TestRes {
    let mut drv = make_sync_driver(seed_r13_10_document()?, 1280.0, 800.0);

    // 1. WF-64: Column bar graph -> Apply Globally + Bar Border 0.5 pt
    let col_gid = SheetId::new("g-col");
    open_format_graph(&mut drv, &col_gid)?;
    assert_widgets(&drv, &["format_graph.bar_border_width_pt"])?;
    drv.click_widget_id("format_graph.apply_globally")?;
    set_scrub_field_text(&mut drv, "format_graph.bar_border_width_pt", 56.0, "0.5")?;
    drv.click_widget_id("modal.ok")?;
    for (idx, s) in drv.state().app.document().graph_sheet(&col_gid).ok_or("missing g-col")?.spec.series.iter().enumerate() {
        match s.gdo {
            GraphicalDataObject::BarWithReplicates { border_width_pt, .. } => assert!((border_width_pt - 0.5).abs() < 1e-6, "series {idx}"),
            ref other => return Err(format!("expected BarWithReplicates on series {idx}, got {other:?}")),
        }
    }

    // 2. WF-63: Diagnostic Forest Plot -> toggle `forest_show_pooled_diamond`
    let f_gid = SheetId::new("g-forest");
    let gs_f = drv.state().app.document().graph_sheet(&f_gid).ok_or("missing g-forest")?;
    assert_eq!(gs_f.spec.kind, GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot));
    let initial_pooled = gs_f.spec.forest_show_pooled_diamond;
    open_format_graph(&mut drv, &f_gid)?;
    click_all(&mut drv, &["format_graph.forest_show_pooled_diamond", "modal.ok"])?;
    assert_eq!(drv.state().app.document().graph_sheet(&f_gid).ok_or("missing g-forest")?.spec.forest_show_pooled_diamond, !initial_pooled);
    Ok(())
}
