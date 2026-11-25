#![forbid(unsafe_code)]

//! Targeted `EguiHumanPathDriver` acceptance suite for Wave B Graph/Inspector GUI gaps:
//! regression overlays (`WF-51`/`54`/`60`), per-series GDO roundtrip (`WF-54`),
//! MV plot modes (`WF-32`/`34`), Grouped heatmap persistence (`WF-45`),
//! MV color/size cycling & localized clamping (`WF-48`), and canvas rulers (`WF-57`).

use crate::common::{ShellTestState, TestRes, assert_modal_parity, ensure, seed_xy_doc, set_textbox_and_commit};
use crate::suite_helpers::{
    add_init_graph, assert_widgets, build_mv_sheet, click_all, make_human_driver, make_sync_driver, new_human_app, open_bubble_plot,
    open_format_graph, run_sheet_cmd,
};
use curvebook_chart::compile_graph;
use curvebook_doc::{
    Cell, ColorChannelMapping, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject, GroupedTable, LayoutSheet, LayoutSpec,
    MultivariatePlotMode, ProjectDocument, SheetId, SizeChannelMapping, SubcolumnMode, SummaryMeasure, TableArchetype, VariableRole,
};
use curvebook_palette::{ColorRamp, ColormapId};
use curvebook_tools::canvas::PhysicalUnit;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy};
use serde_json::json;

fn seed_xy_doc_with_fits() -> Result<(UiApp, SheetId, SheetId, SheetId), String> {
    let (mut doc, xy_id) = seed_xy_doc(
        "R10.28 XY Fits",
        "ds-xy",
        "XY Kinetics",
        "Dose",
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[("Vehicle", &[3.0, 5.0, 7.0, 9.0, 11.0]), ("Drug", &[5.5, 9.0, 12.5, 16.0, 19.5])],
    )?;
    let gid = add_init_graph(&mut doc, &xy_id, "g-xy", GraphKind::Xy)?;
    doc.select_in_navigator(&gid).map_err(|e| e.to_string())?;
    let mut app = new_human_app(doc);
    let lin_id =
        run_sheet_cmd(&mut app, "analysis.run", json!({ "sheet_id": "res-lin", "source_sheet_id": xy_id.as_str(), "kind": "linear_regression" }))?;
    let nl_id = run_sheet_cmd(
        &mut app,
        "analysis.run",
        json!({ "sheet_id": "res-nl", "source_sheet_id": xy_id.as_str(), "kind": "nonlinear_regression", "equation_id": "straight_line" }),
    )?;
    let _ = run_sheet_cmd(&mut app, "transform.run", json!({ "sheet_id": "res-norm", "source_sheet_id": xy_id.as_str(), "kind": "normalize" }))?;
    app.sync_views();
    Ok((app, gid, lin_id, nl_id))
}

fn seed_mv_doc(cols: usize, rows: usize) -> Result<(UiApp, SheetId, SheetId), String> {
    let mut spec = vec![
        ("XVar", VariableRole::Continuous),
        ("YVar", VariableRole::Continuous),
        ("SizeVar", VariableRole::Continuous),
        ("ColorVar", VariableRole::Continuous),
    ];
    if cols > 4 {
        spec.push(("Group", VariableRole::Categorical));
    }
    let ds = build_mv_sheet("ds-mv", "MV Cohort", &spec[..cols], rows, |mv| {
        for r in 0..rows {
            let base = (r + 1) as f64;
            mv.variables[0].cells[r] = Cell::number(base);
            mv.variables[1].cells[r] = Cell::number(base * 2.5);
            mv.variables[2].cells[r] = Cell::number(if r + 1 == rows { 1000.0 } else { base * 4.0 });
            mv.variables[3].cells[r] = Cell::number(if r + 1 == rows { 500.0 } else { base * 5.0 });
            if cols > 4 {
                mv.variables[4].cells[r] = Cell::text(if r % 2 == 0 { "Ctrl" } else { "Treated" });
            }
        }
    })?;
    let ds_id = ds.id.clone();
    let mut doc = ProjectDocument::new("R10.28 MV").map_err(|e| e.to_string())?;
    doc.add_data_sheet(ds).map_err(|e| e.to_string())?;
    let gid = add_init_graph(&mut doc, &ds_id, "g-mv", GraphKind::MultipleVariables)?;
    doc.select_in_navigator(&gid).map_err(|e| e.to_string())?;
    Ok((new_human_app(doc), ds_id, gid))
}

fn assert_compiled_non_empty(app: &UiApp, gid: &SheetId) -> TestRes {
    let gs = app.document().graph_sheet(gid).ok_or_else(|| format!("missing `{gid}`"))?;
    let scene = compile_graph(gs, app.document()).map_err(|e| e.to_string())?;
    ensure(!scene.primitives.is_empty(), format!("empty scene for `{gid}`"))
}

#[test]
fn wf51_54_60_regression_overlay_attach_switch_and_clear_via_gui() -> TestRes {
    let (app, gid, lin_id, nl_id) = seed_xy_doc_with_fits()?;
    let mut driver = make_sync_driver(app, 1280.0, 800.0);
    open_format_graph(&mut driver, &gid)?;
    assert_modal_parity(&driver, "FormatGraph XY initial")?;
    assert_widgets(&driver, &["format_graph.overlay.attach.0", "format_graph.overlay.attach.1"])?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.attach.2").is_none());
    assert!(driver.frame_meta().find_by_widget_id("format_graph.overlay.clear").is_none());

    // Attach linear fit via Apply (keeps dialog open), then switch to nonlinear fit via OK.
    driver.click_widget_id("format_graph.overlay.attach.0")?;
    assert!(driver.state().app.canvas_dialogs.format_graph.show_fitted_curve);
    assert_widgets(&driver, &["format_graph.overlay.clear"])?;
    driver.click_widget_id("modal.apply")?;
    assert!(
        driver.state().last_intent_error.is_none() && matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatGraph { .. })
    );
    let ov0 = driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?.spec.series[0].regression_overlay.as_ref().ok_or("expected ov0")?;
    assert!((&ov0.result_sheet_id, ov0.show_confidence_band, ov0.show_prediction_band) == (&lin_id, true, false));

    click_all(&mut driver, &["format_graph.overlay.attach.1", "modal.ok"])?;
    assert!(driver.state().app.canvas_dialogs.active_dialog == ActiveDialog::None && driver.state().last_intent_error.is_none());
    assert_eq!(
        driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?.spec.series[0].regression_overlay.as_ref().map(|o| &o.result_sheet_id),
        Some(&nl_id)
    );
    assert_compiled_non_empty(&driver.state().app, &gid)?;

    // Re-open, clear series 0 overlay; then attach linear fit on series 1.
    open_format_graph(&mut driver, &gid)?;
    driver.click_widget_id("format_graph.overlay.clear")?;
    assert!(!driver.state().app.canvas_dialogs.format_graph.show_fitted_curve);
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?.spec.series[0].regression_overlay.is_none());

    open_format_graph(&mut driver, &gid)?;
    driver.click_widget_id("format_graph.series_next")?;
    assert_eq!(driver.state().app.canvas_dialogs.format_graph.selected_series_index, 1);
    click_all(&mut driver, &["format_graph.overlay.attach.0", "modal.ok"])?;
    let gs = driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?;
    assert!(gs.spec.series[0].regression_overlay.is_none());
    assert_eq!(gs.spec.series[1].regression_overlay.as_ref().map(|o| &o.result_sheet_id), Some(&lin_id));
    Ok(())
}

#[test]
fn wf54_per_series_gdo_bar_with_replicates_and_scatter_roundtrip() -> TestRes {
    let (app, gid, _, _) = seed_xy_doc_with_fits()?;
    let mut driver = make_sync_driver(app, 1280.0, 800.0);
    open_format_graph(&mut driver, &gid)?;
    assert_widgets(&driver, &["format_graph.series_gdo.scatter_line", "format_graph.series_gdo.bar_replicates"])?;
    assert!(driver.frame_meta().find_by_widget_id("format_graph.bar_border_width_pt").is_none());

    click_all(&mut driver, &["format_graph.series_next", "format_graph.series_gdo.bar_replicates"])?;
    assert_widgets(&driver, &["format_graph.bar_border_width_pt"])?;
    driver.click_widget_id("modal.ok")?;
    assert_eq!(driver.state().last_intent_error, None);
    {
        let gs = driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?;
        assert!(matches!(gs.spec.series[0].gdo, GraphicalDataObject::ScatterAndLine { .. }));
        assert!(matches!(
            gs.spec.series[1].gdo,
            GraphicalDataObject::BarWithReplicates { summary: SummaryMeasure::Mean, replicate_overlay: None, .. }
        ));
    }
    assert_compiled_non_empty(&driver.state().app, &gid)?;

    open_format_graph(&mut driver, &gid)?;
    assert_eq!(
        (driver.state().app.canvas_dialogs.format_graph.selected_series_index, driver.state().app.canvas_dialogs.format_graph.series_gdo_bar),
        (1, true)
    );
    driver.click_widget_id("format_graph.series_prev")?;
    assert!(!driver.state().app.canvas_dialogs.format_graph.series_gdo_bar);
    click_all(&mut driver, &["format_graph.series_next", "format_graph.series_gdo.scatter_line", "modal.ok"])?;
    let gs = driver.state().app.document().graph_sheet(&gid).ok_or("g-xy")?;
    assert!(matches!(gs.spec.series[1].gdo, GraphicalDataObject::ScatterAndLine { marker: Some(_), line: Some(_), .. }));
    Ok(())
}

#[test]
fn wf32_34_mv_plot_mode_categorical_summary_vs_bubble_scatter() -> TestRes {
    let (app, _ds_id, gid) = seed_mv_doc(5, 6)?;
    let mut driver = make_sync_driver(app, 1280.0, 800.0);

    open_bubble_plot(&mut driver, &gid)?;
    assert_modal_parity(&driver, "BubblePlot plot_mode")?;
    assert_widgets(&driver, &["bubble.plot_mode.bubble_scatter", "bubble.plot_mode.categorical_summary"])?;
    for _ in 0..4 {
        driver.click_widget_id("bubble.x_next")?;
    }
    click_all(&mut driver, &["bubble.plot_mode.categorical_summary", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);
    assert!(matches!(
        driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|m| &m.plot_mode),
        Some(MultivariatePlotMode::CategoricalSummary { gdo: GraphicalDataObject::BoxAndWhisker { .. }, y_error_bars: None })
    ));
    assert_compiled_non_empty(&driver.state().app, &gid)?;

    for (btn, expect_bubble) in [("format_graph.mv.plot_mode.bubble_scatter", true), ("format_graph.mv.plot_mode.categorical_summary", false)] {
        open_format_graph(&mut driver, &gid)?;
        driver.click_widget_id("edit.tab.format_graph.axis_variables")?;
        assert_widgets(&driver, &["format_graph.mv.plot_mode.bubble_scatter", "format_graph.mv.plot_mode.categorical_summary"])?;
        click_all(&mut driver, &[btn, "modal.ok"])?;
        let mode = driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).map(|m| &m.plot_mode);
        if expect_bubble {
            assert_eq!(mode, Some(&MultivariatePlotMode::BubbleScatter));
        } else {
            assert!(matches!(
                mode,
                Some(MultivariatePlotMode::CategoricalSummary { gdo: GraphicalDataObject::BoxAndWhisker { .. }, y_error_bars: None })
            ));
        }
    }
    Ok(())
}

#[test]
fn wf45_grouped_heatmap_colormap_and_range_persistence() -> TestRes {
    let mut doc = ProjectDocument::new("R10.28 Grouped Heatmap").map_err(|e| e.to_string())?;
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 3, 3).map_err(|e| e.to_string())?;
    grp.row_titles = vec!["R1".into(), "R2".into(), "R3".into()];
    for (c, title) in ["A", "B", "C"].into_iter().enumerate() {
        grp.data_sets[c].title = title.into();
        for r in 0..3 {
            grp.data_sets[c].cells[r][0] = Cell::number(((r * 3 + c + 1) * 10) as f64);
        }
    }
    let grp_ds = DataSheet::new(SheetId::new("ds-grp"), "Grouped Matrix", TableArchetype::Grouped(grp)).map_err(|e| e.to_string())?;
    let ds_id = grp_ds.id.clone();
    doc.add_data_sheet(grp_ds).map_err(|e| e.to_string())?;
    let hm_gid = add_init_graph(&mut doc, &ds_id, "g-grp-hm", GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap))?;
    assert!(doc.graph_sheet(&hm_gid).is_some_and(|g| g.spec.multivariate.is_none()));
    doc.select_in_navigator(&hm_gid).map_err(|e| e.to_string())?;

    let mut driver = make_human_driver(doc, 1280.0, 800.0);
    open_format_graph(&mut driver, &hm_gid)?;
    assert_modal_parity(&driver, "Grouped Heatmap FormatGraph")?;
    click_all(&mut driver, &["format_graph.heatmap.cmap.Plasma", "format_graph.heatmap.reversed", "format_graph.heatmap.auto_range"])?;
    set_textbox_and_commit(&mut driver, "format_graph.heatmap.min", "5")?;
    set_textbox_and_commit(&mut driver, "format_graph.heatmap.max", "50")?;
    driver.click_widget_id("modal.ok")?;
    assert!(driver.state().last_intent_error.is_none() && driver.state().app.canvas_dialogs.active_dialog == ActiveDialog::None);

    let enc = driver.state().app.document().graph_sheet(&hm_gid).and_then(|g| g.spec.multivariate.as_ref()).ok_or("expected spec.multivariate")?;
    assert!(matches!(
        enc.color,
        ColorChannelMapping::Continuous { ramp: ColorRamp::Named { id: ColormapId::Plasma, reversed: true }, domain_clamp: Some((lo, hi)), .. }
        if (lo - 5.0).abs() < 1e-6 && (hi - 50.0).abs() < 1e-6
    ));
    assert_compiled_non_empty(&driver.state().app, &hm_gid)
}

fn apply_mv_clamp(
    driver: &mut EguiHumanPathDriver<'_, ShellTestState>,
    gid: &SheetId,
    min_s: &str,
    max_s: &str,
    click_oor: Option<&str>,
    expected: Option<(f64, f64)>,
) -> TestRes {
    set_textbox_and_commit(driver, "format_graph.mv.color_clamp_min", min_s)?;
    set_textbox_and_commit(driver, "format_graph.mv.color_clamp_max", max_s)?;
    if let Some(wid) = click_oor {
        driver.click_widget_id(wid)?;
    }
    driver.click_widget_id("modal.ok")?;
    ensure(driver.state().last_intent_error.is_none(), format!("clamp commit err: {:?}", driver.state().last_intent_error))?;
    let color = driver.state().app.document().graph_sheet(gid).and_then(|g| g.spec.multivariate.as_ref()).map(|e| &e.color);
    let ok = match (color, expected) {
        (Some(ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. }), Some((elo, ehi))) => {
            (*lo - elo).abs() < 1e-6 && (*hi - ehi).abs() < 1e-6
        }
        (Some(ColorChannelMapping::Continuous { domain_clamp: None, .. }), None) => true,
        _ => false,
    };
    ensure(ok, format!("expected domain_clamp={expected:?}, got {color:?}"))
}

#[test]
fn wf48_mv_color_size_cycling_domain_clamp_and_bubble_controls() -> TestRes {
    let (app, _ds_id, gid) = seed_mv_doc(4, 6)?;
    let mut driver = make_sync_driver(app, 1280.0, 800.0);

    // 1. BubblePlot color/size cycling & constant/variable toggle.
    open_bubble_plot(&mut driver, &gid)?;
    assert_modal_parity(&driver, "BubblePlot WF48")?;
    assert_widgets(&driver, &["bubble.color_col_next", "bubble.size_variable"])?;
    assert!(driver.frame_meta().find_by_widget_id("bubble.size_col_next").is_none());
    click_all(&mut driver, &["bubble.color_col_next", "bubble.color_col_next", "bubble.size_variable"])?;
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.color_channel, ColorChannelMapping::Continuous { column: 1, .. }));
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Variable { column: 0, .. }));
    click_all(&mut driver, &["bubble.size_col_next", "bubble.size_col_next"])?;
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Variable { column: 2, .. }));
    driver.click_widget_id("bubble.size_variable")?;
    assert!(matches!(driver.state().app.canvas_dialogs.bubble_plot.size_channel, SizeChannelMapping::Constant { .. }));
    click_all(&mut driver, &["bubble.size_variable", "bubble.size_col_next", "bubble.size_col_next", "modal.ok"])?;
    assert_eq!(driver.state().last_intent_error, None);

    // 2. FormatGraph MV continuous color/size col cycling + domain clamp (0, 30).
    open_format_graph(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.axis_variables")?;
    assert_modal_parity(&driver, "FormatGraph MV AxisVariables")?;
    click_all(
        &mut driver,
        &[
            "format_graph.mv.color_continuous",
            "format_graph.mv.color_col_next",
            "format_graph.mv.color_col_next",
            "edit.tab.format_graph.data_objects",
        ],
    )?;
    assert_modal_parity(&driver, "FormatGraph MV DataObjects")?;
    driver.click_widget_id("format_graph.mv.size_col_next")?;
    apply_mv_clamp(&mut driver, &gid, "0", "30", None, Some((0.0, 30.0)))?;
    let enc = driver.state().app.document().graph_sheet(&gid).and_then(|g| g.spec.multivariate.as_ref()).ok_or("missing enc")?;
    assert!(
        matches!(enc.color, ColorChannelMapping::Continuous { column: 3, .. }) && matches!(enc.size, SizeChannelMapping::Variable { column: 3, .. })
    );
    assert_compiled_non_empty(&driver.state().app, &gid)?;

    // 3. Re-open -> oor_omit clears drafts -> oor_clamp sets (2, 20) -> `de` comma bounds (0.5, 12.5) -> clear.
    open_format_graph(&mut driver, &gid)?;
    assert_eq!(
        (driver.state().views.graph_modals.mv_clamp_min_draft.as_str(), driver.state().views.graph_modals.mv_clamp_max_draft.as_str()),
        ("0", "30")
    );
    click_all(&mut driver, &["edit.tab.format_graph.data_objects", "format_graph.mv.oor_omit"])?;
    assert_eq!(
        (driver.state().views.graph_modals.mv_clamp_min_draft.as_str(), driver.state().views.graph_modals.mv_clamp_max_draft.as_str()),
        ("", "")
    );
    apply_mv_clamp(&mut driver, &gid, "2", "20", Some("format_graph.mv.oor_clamp"), Some((2.0, 20.0)))?;

    assert_eq!(driver.state_mut().app.set_locale("de"), "de");
    open_format_graph(&mut driver, &gid)?;
    assert_eq!(
        (driver.state().views.graph_modals.mv_clamp_min_draft.as_str(), driver.state().views.graph_modals.mv_clamp_max_draft.as_str()),
        ("2", "20")
    );
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    apply_mv_clamp(&mut driver, &gid, "0,5", "12,5", None, Some((0.5, 12.5)))?;

    open_format_graph(&mut driver, &gid)?;
    driver.click_widget_id("edit.tab.format_graph.data_objects")?;
    apply_mv_clamp(&mut driver, &gid, "", "", None, None)
}

#[test]
fn wf57_canvas_rulers_1440x600_no_text_bleed() -> TestRes {
    let mut doc = ProjectDocument::new("R10.28 WF57 Rulers").map_err(|e| e.to_string())?;
    let lid = SheetId::try_new("lay-57").map_err(|e| e.to_string())?;
    doc.add_layout_sheet(LayoutSheet::new(lid, "Schematic Layout", LayoutSpec::default()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut app = new_human_app(doc);
    let _ = app.execute_command("sheet.select", json!({ "sheet_id": "lay-57" })).map_err(|e| e.to_string())?;
    let mut driver = make_sync_driver(app, 1440.0, 600.0);

    driver.click_widget_id("toolbar.rulers")?;
    ensure(driver.state().app.canvas_dialogs.rulers_visible, "expected rulers_visible == true")?;
    for unit in [PhysicalUnit::Points, PhysicalUnit::Inches, PhysicalUnit::Millimeters] {
        driver.state_mut().app.canvas_dialogs.ruler_unit = unit;
        driver.run_settled();
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        assert_modal_parity(&driver, &format!("WF-57 rulers {unit:?} 1440x600"))?;

        driver.set_bleed_policy(TextBleedPolicy::StrictAllEdges);
        let ruler_bleeds: Vec<_> = driver
            .verify_pixel_to_hit_box_parity()
            .violations
            .into_iter()
            .filter(|v| {
                v.check == "text_unclipped_bleed"
                    && v.detail.split('`').nth(1).is_some_and(|txt| txt.parse::<i32>().is_ok() || matches!(txt, "pt" | "in" | "mm"))
            })
            .collect();
        ensure(ruler_bleeds.is_empty(), format!("WF-57 strict ruler text bleed for {unit:?}: {ruler_bleeds:?}"))?;
    }
    driver.click_widget_id("toolbar.rulers")?;
    ensure(!driver.state().app.canvas_dialogs.rulers_visible, "expected rulers_visible == false after second click")
}
