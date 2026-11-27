#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `R15.3` Trailing 7-Context Studio Rail (`280 pt -> 36 pt`) & `26 pt` Status & Telemetry Footer
//! integration tests.

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistItem, ChecklistStatus, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, HookSource,
    ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LinkSource, LinkedDataRegion, MultipleVariablesTable,
    NonlinearFitSpec, ParameterRule, ParameterScope, PhysicalUnit, ProjectDocument, ResultScalarEntry, ResultSheet, ResultStatus, SheetCellRange,
    SheetId, SubcolumnMode, TableArchetype,
};
use curvebook_engine::Session;
use curvebook_math::AxisScale;
use curvebook_palette::PaletteId;
use curvebook_tools::GridCellCoord;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::canvas_view::CanvasHoverReadout;
use curvebook_ui_egui::egui_shell::driver::{
    BETA_LOCALE_CODES, EguiHumanPathDriver, accesskit_accessible_name, build_driver_font_definitions, collect_tooltip_texts_from_accesskit,
};
use curvebook_ui_egui::egui_shell::inspect_bridge::{check_accesskit_scrollbar_non_overlap, render_egui_shell_frame};
use curvebook_ui_egui::egui_shell::status_footer::{QUICK_STATS_CELL_CAP, QuickStatsCache, compute_cached_quick_stats};
use curvebook_ui_egui::egui_shell::trailing_studio::{
    GraphStudioTab, ResultsStudioTab, StudioContextSurface, collect_info_downstream_references, resolve_formula_operand_highlights,
    resolve_studio_surface,
};
use curvebook_ui_egui::egui_shell::{EguiShellViewState, FrameInspectMeta, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::UiThemeMode;
use curvebook_ui_egui::{UiApp, UiThemeSpec};
use serde_json::json;

use crate::common::{ShellTestState, assert_clean_parity, fill_col_values, make_shell_driver_with_state};

type BoxRes = Result<(), Box<dyn std::error::Error>>;

fn select_sheet(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &str) -> BoxRes {
    let _ = driver.state_mut().app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
    driver.run_settled();
    Ok(())
}

fn checklist_item(assumption: &str, status: ChecklistStatus, metric: &str, explanation: &str) -> ChecklistItem {
    ChecklistItem {
        assumption: assumption.to_owned(),
        status,
        diagnostic_metric: Some(metric.to_owned()),
        explanation: explanation.to_owned(),
        remediation: None,
    }
}

fn seed_full_workspace_app() -> Result<UiApp, Box<dyn std::error::Error>> {
    let mut doc = ProjectDocument::new("R15.3 Studio & Footer Test")?;

    let mut mv = MultipleVariablesTable::new(3, 4)?;
    for (idx, title) in ["Dose", "Response", "LogDose"].iter().enumerate() {
        mv.variables[idx].title = (*title).to_owned();
    }
    for r in 0..4 {
        let d = ((r + 1) * 10) as f64;
        mv.variables[0].cells[r] = Cell::number(d);
        mv.variables[1].cells[r] = Cell::number(d * 0.25 + 1.0);
    }
    mv.variables[2].set_calculated_formula("ln([Dose])");
    doc.add_data_sheet(DataSheet::new("mv-1", "PK Multiple Variables", TableArchetype::MultipleVariables(mv))?)?;

    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 3, 5)?;
    fill_col_values(&mut col.data_sets[0], "Control", &[10.0, 11.0, 12.0, 13.0, 14.0]);
    fill_col_values(&mut col.data_sets[1], "Treated Low", &[15.0, 16.0, 17.0, 18.0, 19.0]);
    fill_col_values(&mut col.data_sets[2], "Treated High", &[22.0, 23.0, 24.0, 25.0, 26.0]);
    let mut ds_col = DataSheet::new("col-1", "Column Assay", TableArchetype::Column(col))?;
    ds_col.linked_regions.push(LinkedDataRegion {
        id: String::from("lnk-1"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("mv-1"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(3, 0, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 2, 0), CellCoord::new(3, 2, 0)),
        provenance_summary: String::from("Linked from PK Multiple Variables"),
    });
    doc.add_data_sheet(ds_col)?;

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.linked_data_sheets.push(SheetId::new("col-1"));
    info.set_constant("VehicleMass", ConstantValue::Number(12.5), Some(String::from("mg")))?;
    info.set_constant("DrugName", ConstantValue::Text(String::from("Compound-X")), None)?;
    doc.add_info_sheet(info)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    let _ = app.execute_command(
        "analysis.run",
        json!({ "sheet_id": "res-anova", "source_sheet_id": "col-1", "kind": "one_way_anova", "multiplicity": "tukey" }),
    )?;

    let mut doc2 = app.document().clone();
    if let Some(rs) = doc2.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist = vec![
            checklist_item(
                "Normality of residuals",
                ChecklistStatus::Pass,
                "Shapiro-Wilk W = 0.96, p = 0.62",
                "Residuals are consistent with Gaussian noise.",
            ),
            checklist_item(
                "Homogeneity of variances",
                ChecklistStatus::Warn,
                "Brown-Forsythe F = 3.12, p = 0.048",
                "Mild variance inequality across treatment arms.",
            ),
            checklist_item("Extreme leverage outlier", ChecklistStatus::Violated, "ROUT Q = 1% flagged row 4", "High residual leverage observed."),
        ];
        rs.scalars.push(ResultScalarEntry { key: String::from("R2"), value: 0.912, units: None, data_set_index: None });
    }

    let mut nlr_fit = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nlr_fit.parameter_rules = vec![ParameterRule {
        name: String::from("Bottom"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("VehicleMass"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    }];
    let mut rs_nlr = ResultSheet::new(AnalysisSpec::new("res-nlr", "col-1", AnalysisKind::NonlinearRegression(nlr_fit))?)?;
    rs_nlr.status = ResultStatus::UpToDate;
    doc2.add_result_sheet(rs_nlr)?;
    *app.session_mut() = Session::from_document(doc2);

    for (cmd, params) in [
        ("graph.create", json!({ "data_sheet_id": "col-1", "sheet_id": "graph-1", "kind": "column", "title": "Response Graph" })),
        ("graph.setSpec", json!({ "sheet_id": "graph-1", "spec": { "title": "Response ({{info:VehicleMass}})" } })),
        ("layout.create", json!({ "sheet_id": "layout-1", "title": "Main Figure", "preset": "double_column" })),
        ("layout.addPanel", json!({ "sheet_id": "layout-1", "kind": "graph", "graph_sheet_id": "graph-1", "row": 0, "col": 0 })),
        (
            "layout.addPanel",
            json!({ "sheet_id": "layout-1", "kind": "text", "markdown": "Compound: {{info:DrugName}}", "bounds_pt": [20.0, 20.0, 120.0, 24.0] }),
        ),
        ("sheet.select", json!({ "sheet_id": "mv-1" })),
    ] {
        let _ = app.execute_command(cmd, params)?;
    }
    app.sync_views();
    Ok(app)
}

fn verify_studio_mv_and_table_surfaces(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) -> BoxRes {
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MultipleVariablesFormula);
    if let Some(ds) = driver.state().app.document().data_sheet(&SheetId::new("mv-1"))
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        assert_eq!(resolve_formula_operand_highlights(&mv.variables, "ln([Dose]) + [Response]").len(), 2);
    }
    driver.click_widget_id("inspector.mv.col.2")?;
    let mv_hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(mv_hls.len(), 1);
    assert_eq!(mv_hls[0].variable_title, "Dose");

    driver.state_mut().views.trailing_studio.mv_formula_draft = String::from("=[Dose] * 2 + [Response]");
    driver.run_settled();
    let hls = &driver.state().views.trailing_studio.mv_operand_highlights;
    assert_eq!(hls.len(), 2);
    assert_ne!(hls[0].color_hex, hls[1].color_hex);

    driver.click_widget_id("inspector.mv.apply_formula")?;
    if let TableArchetype::MultipleVariables(mv) = &driver.state().app.document().data_sheet(&SheetId::new("mv-1")).ok_or("mv-1 missing")?.archetype {
        assert_eq!(mv.variables[2].source, curvebook_doc::VariableSource::Calculated { formula: String::from("[Dose] * 2 + [Response]") });
    }

    select_sheet(driver, "col-1")?;
    driver.state_mut().app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    driver.state_mut().app.grid.tool.extend_selection(GridCellCoord::new(1, 0, 0));
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::TableSelection);
    driver.click_widget_id("inspector.table.decimals.3")?;
    assert_eq!(driver.state().views.trailing_studio.table_decimals, Some(3));
    driver.click_widget_id("inspector.table.exclude_subset")?;
    let ds_col = driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?;
    assert!(ds_col.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded) && ds_col.cell(CellCoord::new(1, 0, 0)).is_some_and(|c| c.excluded));
    driver.click_widget_id("inspector.table.include_subset")?;
    assert!(
        !driver.state().app.document().data_sheet(&SheetId::new("col-1")).ok_or("col-1")?.cell(CellCoord::new(0, 0, 0)).is_some_and(|c| c.excluded)
    );

    select_sheet(driver, "info-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::MetadataHooks);
    let info_sheet = driver.state().app.document().info_sheet(&SheetId::new("info-1")).cloned().ok_or("info-1")?;
    let refs = collect_info_downstream_references(driver.state().app.document(), &info_sheet);
    for expected in ["res-nlr", "graph-1", "layout-1"] {
        assert!(refs.iter().any(|r| r.target_sheet_id.as_str() == expected), "missing downstream ref `{expected}`");
    }
    let vm_idx = info_sheet.constants.iter().position(|c| c.name == "VehicleMass").ok_or("VehicleMass missing")?;
    driver.click_widget_id(&format!("inspector.info.const.{vm_idx}"))?;
    driver.state_mut().views.trailing_studio.info_value_draft = String::from("25.0");
    driver.click_widget_id("inspector.info.save_const")?;
    assert_eq!(
        driver
            .state()
            .app
            .document()
            .info_sheet(&SheetId::new("info-1"))
            .and_then(|i| i.get_constant("VehicleMass"))
            .and_then(|c| c.value.as_finite_f64()),
        Some(25.0)
    );
    driver.click_widget_id("inspector.info.ref.0")?;
    assert_ne!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("info-1"));
    Ok(())
}

fn verify_studio_results_graph_drillback_and_layout(driver: &mut EguiHumanPathDriver<'_, ShellTestState>) -> BoxRes {
    select_sheet(driver, "res-anova")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::ResultsInterpret);
    driver.click_widget_id("inspector.results.filter_flagged")?;
    assert!(driver.state().views.trailing_studio.results_severity_flagged_only);
    for (wid, expected) in
        [("inspector.results.tab.guidance", ResultsStudioTab::Guidance), ("inspector.results.tab.methods", ResultsStudioTab::Methods)]
    {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().views.trailing_studio.results_tab, expected);
    }
    driver.click_widget_id("inspector.methods.fmt.plain")?;
    driver.click_widget_id("inspector.copy_methods")?;
    assert!(driver.state().views.trailing_studio.last_copied_methods);
    assert!(!driver.state().views.trailing_studio.last_copied_methods_text.trim().is_empty());
    assert!(driver.state().app.canvas_dialogs.result_view.last_methods_report.is_some());
    driver.click_widget_id("inspector.results.edit_params")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;

    select_sheet(driver, "graph-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::GraphInspector);
    for (wid, expected) in [
        ("inspector.graph.tab.data", GraphStudioTab::Data),
        ("inspector.graph.tab.axes", GraphStudioTab::Axes),
        ("inspector.graph.tab.legends", GraphStudioTab::Legends),
        ("inspector.graph.tab.annotations", GraphStudioTab::Annotations),
        ("inspector.graph.tab.canvas", GraphStudioTab::Canvas),
    ] {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().views.trailing_studio.graph_tab, expected);
    }
    driver.click_widget_id("inspector.graph.tab.data")?;
    driver.click_widget_id("inspector.graph.palette.PaulTolBright")?;
    assert_eq!(driver.state().app.document().graph_sheet(&SheetId::new("graph-1")).unwrap().spec.palette, PaletteId::PaulTolBright);
    driver.click_widget_id("inspector.graph.tab.axes")?;
    driver.click_widget_id("inspector.graph.scale.Log10")?;
    assert_eq!(driver.state().app.document().graph_sheet(&SheetId::new("graph-1")).unwrap().spec.y_axis.segments[0].scale, AxisScale::Log10);
    driver.click_widget_id("inspector.graph.more_axes")?;
    assert!(matches!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::FormatAxes { .. }));
    driver.state_mut().app.canvas_dialogs.active_dialog = ActiveDialog::None;

    driver.state_mut().app.brush_from_grid_cell(&SheetId::new("col-1"), GridCellCoord::new(1, 0, 0));
    driver.run_settled();
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::DataPointDrillBack);
    driver.click_widget_id("inspector.drill_back.toggle_exclude")?;
    assert!(driver.state().app.document().data_sheet(&SheetId::new("col-1")).unwrap().cell(CellCoord::new(1, 0, 0)).unwrap().excluded);
    driver.click_widget_id("inspector.drill_back.back")?;
    assert!(driver.state().app.brushed_cell.is_none());
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::GraphInspector);

    driver.state_mut().app.brush_from_grid_cell(&SheetId::new("col-1"), GridCellCoord::new(2, 1, 0));
    driver.run_settled();
    driver.click_widget_id("inspector.drill_back.jump")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));
    assert_eq!(driver.state().app.grid.tool.active, Some(GridCellCoord::new(2, 1, 0)));

    driver.state_mut().app.clear_brush();
    select_sheet(driver, "layout-1")?;
    assert_eq!(resolve_studio_surface(&driver.state().app), StudioContextSurface::PageLayoutComposer);
    for (wid, expected_w) in
        [("inspector.layout.preset.85mm", 85.0_f64), ("inspector.layout.preset.114mm", 114.0), ("inspector.layout.preset.174mm", 174.0)]
    {
        driver.click_widget_id(wid)?;
        let spec = &driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec;
        assert!((spec.width - expected_w).abs() < 0.05 && spec.unit == PhysicalUnit::Millimeters, "{wid}: got {} {:?}", spec.width, spec.unit);
    }
    driver.click_widget_id("inspector.layout.dpi.600")?;
    assert_eq!(driver.state().views.trailing_studio.layout_dpi, 600);
    let snap_before = driver.state().views.trailing_studio.snap_enabled;
    driver.click_widget_id("inspector.layout.snap")?;
    assert_ne!(driver.state().views.trailing_studio.snap_enabled, snap_before);
    driver.click_widget_id("inspector.layout.grid.2x2")?;
    assert_eq!(driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.grid_template, Some((2, 2)));
    let align_before = driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.align_plot_origins;
    driver.click_widget_id("inspector.layout.align_origins")?;
    assert_ne!(driver.state().app.document().layout_sheet(&SheetId::new("layout-1")).unwrap().spec.align_plot_origins, align_before);
    Ok(())
}

#[test]
fn studio_resolves_all_seven_context_surfaces_and_commits_each_domain() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 800.0, 1.0), Tokens::default());
    verify_studio_mv_and_table_surfaces(&mut driver)?;
    verify_studio_results_graph_drillback_and_layout(&mut driver)
}

#[test]
fn shortcut_cmd_shift_i_and_responsive_auto_collapse_breakpoints() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 600.0, 1.0), Tokens::default());

    let l_1440 = driver.state().leading_rect_pt.ok_or("leading 1440")?.width();
    let t_1440 = driver.state().trailing_rect_pt.ok_or("trailing 1440")?.width();
    assert!((200.0..270.0).contains(&l_1440) && (260.0..300.0).contains(&t_1440), "1440 widths: l={l_1440}, t={t_1440}");

    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(driver.state().views.trailing_collapsed_rail);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing collapsed")?.width() - 36.0).abs() < 1.0);
    driver.press_egui_key(egui::Key::I, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert!(!driver.state().views.trailing_collapsed_rail);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing expanded")?.width() - 280.0).abs() < 1.0);

    driver.set_viewport(ViewportMetrics::new(900.0, 600.0, 1.0));
    assert!(driver.state().leading_rect_pt.ok_or("leading 900")?.width() > 200.0);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing 900")?.width() - 36.0).abs() < 1.0);

    driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
    assert!((driver.state().leading_rect_pt.ok_or("leading 640")?.width() - 36.0).abs() < 1.0);
    assert!((driver.state().trailing_rect_pt.ok_or("trailing 640")?.width() - 36.0).abs() < 1.0);
    assert_clean_parity(&driver, "640x420 collapsed")?;
    Ok(())
}

#[test]
fn status_footer_history_sections_switcher_links_cursor_cvd_rulers_and_zoom_gates() -> BoxRes {
    let mut driver =
        make_shell_driver_with_state(ShellTestState::new(seed_full_workspace_app()?), ViewportMetrics::new(1440.0, 600.0, 1.0), Tokens::default());
    assert!((driver.state().footer_rect_pt.ok_or("footer rect")?.height() - 26.0).abs() < 1.0);

    // Welcome: rulers and zoom hidden.
    {
        let mut doc = driver.state().app.document().clone();
        doc.active_sheet_id = None;
        *driver.state_mut().app.session_mut() = Session::from_document(doc);
        driver.run_settled();
    }
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none() && driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none());

    select_sheet(&mut driver, "col-1")?;
    assert!(driver.frame_meta().find_by_widget_id("status.link_indicator").is_some());
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_none() && driver.frame_meta().find_by_widget_id("status.zoom.fit").is_none());
    assert!(driver.frame_meta().find_by_widget_id("status.section.info").is_none());

    for (chip, sheet, has_rulers, has_zoom) in [
        ("nav.chip.info", "nav.sheet.info-1", false, false),
        ("nav.chip.results", "nav.sheet.res-anova", false, false),
        ("nav.chip.graphs", "nav.sheet.graph-1", true, true),
    ] {
        crate::common::select_nav_kind_filter(&mut driver, chip)?;
        driver.click_widget_id(sheet)?;
        assert_eq!(driver.frame_meta().find_by_widget_id("status.rulers").is_some(), has_rulers, "{sheet} rulers");
        assert_eq!(driver.frame_meta().find_by_widget_id("status.zoom.in").is_some(), has_zoom, "{sheet} zoom");
    }

    driver.dispatch_events(vec![egui::Event::PointerMoved(egui::pos2(520.0, 260.0))]);
    let ptr_summary = driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary ptr")?;
    assert!(ptr_summary.label.contains("X:") && ptr_summary.label.contains("Y:"));
    driver.state_mut().hover_readout_override = Some(CanvasHoverReadout {
        source_sheet_id: Some(SheetId::new("col-1")),
        point_index: Some(0),
        row: Some(0),
        col: Some(0),
        label: String::from("Control (X=1, Y=10.0)"),
        value_summary: String::from("10.00"),
    });
    driver.run_settled();
    assert!(driver.frame_meta().find_by_widget_id("status.summary").ok_or("status.summary")?.label.contains("10.00"));
    driver.state_mut().hover_readout_override = None;

    driver.click_widget_id("status.rulers")?;
    assert!(driver.state().app.canvas_dialogs.rulers_visible);
    driver.click_widget_id("status.zoom.in")?;
    assert!(driver.state().app.canvas_dialogs.canvas.zoom > 1.0);
    driver.click_widget_id("status.zoom.reset")?;
    assert!((driver.state().app.canvas_dialogs.canvas.zoom - 1.0).abs() < 1e-6);

    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.layouts")?;
    driver.click_widget_id("nav.sheet.layout-1")?;
    crate::common::select_nav_kind_filter(&mut driver, "nav.chip.all")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));
    assert!(driver.frame_meta().find_by_widget_id("status.rulers").is_some() && driver.frame_meta().find_by_widget_id("status.zoom.in").is_some());

    driver.click_widget_id("status.section.gallery")?;
    assert!(
        driver.state().app.nav.gallery.open
            && driver.frame_meta().find_by_widget_id("status.rulers").is_none()
            && driver.frame_meta().find_by_widget_id("status.zoom.in").is_some()
    );
    driver.click_widget_id("status.section.gallery")?;
    assert!(!driver.state().app.nav.gallery.open);

    driver.click_widget_id("status.history.back")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("graph-1"));
    driver.click_widget_id("status.history.forward")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("layout-1"));

    assert!(driver.frame_meta().find_by_widget_id("status.sheet_switcher").is_none());
    driver.click_widget_id("nav.collapse_rail")?;
    driver.click_widget_id("status.sheet_switcher")?;
    driver.click_widget_id("status.sheet_switcher.item.col-1")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("col-1"));

    driver.click_widget_id("status.link_indicator")?;
    driver.click_widget_id("status.link_indicator.item.0")?;
    assert_eq!(driver.state().app.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("mv-1"));
    assert!(driver.frame_meta().find_by_widget_id("status.preview_filter").is_none());
    Ok(())
}

#[test]
fn quick_stats_50k_cells_under_half_ms_and_cached_steady_frames() -> BoxRes {
    let mut table = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 1, 60_000)?;
    for r in 0..60_000 {
        table.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    let mut doc = ProjectDocument::new("50k Perf")?;
    doc.add_data_sheet(DataSheet::new("col-50k", "Big Sheet", TableArchetype::Column(table))?)?;
    doc.select_in_navigator(&SheetId::new("col-50k"))?;
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(49_999, 0, 0));

    let mut cache = QuickStatsCache::default();
    let t0 = Instant::now();
    let stats = compute_cached_quick_stats(&app, &mut cache).clone();
    let first_ms = t0.elapsed().as_secs_f64() * 1000.0;

    assert_eq!((stats.n, stats.truncated_at_cap, cache.recompute_count), (QUICK_STATS_CELL_CAP, false, 1));
    assert!((stats.mean.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.median.unwrap() - 25_000.5).abs() < 1e-6);
    assert!((stats.range.unwrap() - 49_999.0).abs() < 1e-6);
    assert!(stats.sd.unwrap() > 14_433.0 && stats.sd.unwrap() < 14_435.0);
    assert!((stats.sem.unwrap() - stats.sd.unwrap() / (50_000.0_f64).sqrt()).abs() < 1e-6);

    let mut miss_samples_ms = Vec::with_capacity(5);
    for shift in 1..=5_usize {
        app.grid.tool.select_single(GridCellCoord::new(shift, 0, 0));
        app.grid.tool.extend_selection(GridCellCoord::new(shift + 49_999, 0, 0));
        let ts = Instant::now();
        let next_stats = compute_cached_quick_stats(&app, &mut cache);
        miss_samples_ms.push(ts.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(next_stats.n, QUICK_STATS_CELL_CAP);
    }
    miss_samples_ms.sort_by(f64::total_cmp);
    let median_budget_ms = if cfg!(debug_assertions) { 2.0 } else { 0.5 };
    assert!(
        first_ms < 25.0 && miss_samples_ms[2] < median_budget_ms,
        "Quick-Stats timing: first={first_ms:.3} ms, median={:.3} ms",
        miss_samples_ms[2]
    );

    let count_before_steady = cache.recompute_count;
    let t1 = Instant::now();
    for _ in 0..1_000 {
        assert_eq!(compute_cached_quick_stats(&app, &mut cache).n, QUICK_STATS_CELL_CAP);
    }
    assert_eq!(cache.recompute_count, count_before_steady);
    assert!(t1.elapsed().as_secs_f64() * 1000.0 < 0.5);

    app.grid.tool.select_single(GridCellCoord::new(0, 0, 0));
    app.grid.tool.extend_selection(GridCellCoord::new(59_999, 0, 0));
    let capped = compute_cached_quick_stats(&app, &mut cache).clone();
    assert_eq!((capped.n, capped.truncated_at_cap, cache.recompute_count), (QUICK_STATS_CELL_CAP, true, count_before_steady + 1));
    Ok(())
}

#[test]
fn responsive_640x420_1440x600_scrollbar_non_overlap_themes_and_all_14_beta_locales() -> BoxRes {
    let base_app = seed_full_workspace_app()?;
    let long_title = "UltraLongAssayCalibrationConstantThatTriggersStudioEllipsisCorrectlyAcrossLocales";

    for (idx, locale) in BETA_LOCALE_CODES.iter().enumerate() {
        let mode = [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast][idx % 3];
        let mut app = base_app.clone();
        assert_eq!(app.set_locale(locale), *locale);
        app.set_theme_mode(mode);
        let mut doc = app.document().clone();
        if let Some(ds) = doc.data_sheet_mut(&SheetId::new("col-1")) {
            ds.title = long_title.to_owned();
        }
        *app.session_mut() = Session::from_document(doc);
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": "col-1" }))?;
        app.sync_views();

        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(mode), locale);
        let mut driver = make_shell_driver_with_state(ShellTestState::new(app), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);
        assert_clean_parity(&driver, &format!("{locale}/{mode:?} 1440x600"))?;

        if let Some(sw) = driver.frame_meta().find_by_widget_id("status.sheet_switcher").cloned()
            && sw.ellipsized
        {
            let expected_full = sw.full_label.clone().unwrap_or_default();
            driver.verify_ellipsis_tooltip_and_accesskit("status.sheet_switcher", &expected_full)?;
            let ak = driver.accesskit_tree().ok_or("missing accesskit")?;
            let ak_node = ak.nodes.iter().find(|(id, _)| *id == sw.accesskit_id).map(|(_, n)| n).ok_or("missing sw ak")?;
            assert_eq!(accesskit_accessible_name(ak_node), Some(expected_full.as_str()));
            assert!(collect_tooltip_texts_from_accesskit(&ak.nodes).iter().any(|t| t.contains(&expected_full)));
        }

        driver.set_viewport(ViewportMetrics::new(640.0, 420.0, 1.0));
        assert_clean_parity(&driver, &format!("{locale}/{mode:?} 640x420"))?;
    }

    for (w, h) in [(1440.0_f32, 600.0), (640.0, 420.0)] {
        let mut ak_app = base_app.clone();
        let _ = ak_app.execute_command("sheet.select", json!({ "sheet_id": "mv-1" }))?;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.set_fonts(build_driver_font_definitions());
        let (mut views, mut meta, mut intents) = (EguiShellViewState::default(), FrameInspectMeta::default(), PendingUiIntents::new());
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), ..Default::default() },
            |ui| {
                meta.begin_pass(ui.ctx());
                let _ = render_egui_shell_frame(ui, &ak_app, &mut views, &mut meta, &mut intents);
            },
        );
        out.textures_delta.clear();
        check_accesskit_scrollbar_non_overlap(out.platform_output.accesskit_update.as_ref().ok_or("expected AccessKit TreeUpdate")?)?;
    }
    Ok(())
}

#[test]
fn studio_assumptions_wrap_and_offscreen_scroll_buttons_culled_at_1440x600() -> BoxRes {
    let mut app = seed_full_workspace_app()?;
    let mut doc = app.document().clone();
    if let Some(rs) = doc.result_sheet_mut(&SheetId::new("res-anova")) {
        rs.checklist.insert(
            0,
            checklist_item(
                "Repeated-measures condition normality (Shapiro-Wilk)",
                ChecklistStatus::Warn,
                "VeryLongDiagnosticMetricHeader W = 0.8421, p = 0.0142 across treatment conditions",
                "Long assumption diagnostic explanation verifying word wrap inside 280pt Studio card without horizontal overflow.",
            ),
        );
    }
    *app.session_mut() = Session::from_document(doc);

    for sheet_id in ["res-anova", "graph-1", "layout-1"] {
        let _ = app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }))?;
        app.sync_views();
        let tokens = Tokens::from_theme_spec(&UiThemeSpec::for_mode(UiThemeMode::Light), "en");
        let driver = make_shell_driver_with_state(ShellTestState::new(app.clone()), ViewportMetrics::new(1440.0, 600.0, 1.0), tokens);
        assert_clean_parity(&driver, &format!("{sheet_id} studio 1440x600"))?;
    }
    Ok(())
}

#[test]
fn aud_09_08_18_19_20_41_43_46_info_layout_grid_and_trailing_studio() -> BoxRes {
    use crate::helpers::{click_input, run_egui_pass_with_ctx};
    use curvebook_doc::{DescriptiveSpec, InfoConstant, LayoutItem, LayoutSpec, RectPt, SourceCellId, VariableRole, VariableSource};
    use curvebook_ui_egui::egui_shell::info_view::{InfoViewEguiState, render_info_view_for_sheet};
    use curvebook_ui_egui::egui_shell::trailing_studio::{
        TrailingStudioContext, TrailingStudioEguiState, arrange_layout_grid_items, render_trailing_studio,
    };
    use curvebook_ui_egui::egui_shell::{LayoutMetrics, ShellLayoutInput, UiIntent};
    use serde_json::Value;

    // 1. AUD-09-08 & AUD-09-43: InfoView comma decimal, disabled Escape guard, and sheet-switch notes flush.
    let mut idoc = ProjectDocument::new("Info View")?;
    idoc.info_sheets.push(InfoSheet::new_preseeded("info-1", "Info 1")?);
    idoc.info_sheets.push(InfoSheet::new_preseeded("info-2", "Info 2")?);
    let mut iapp = UiApp::from_session(Session::from_document(idoc));
    iapp.set_locale("de");
    iapp.execute_command("info.setConstant", json!({ "sheet_id": "info-1", "name": "Dose", "value": 12.5 }))?;
    let itok = Tokens::from_theme_spec(&iapp.theme, &iapp.locale);

    let (mut m_ro, mut i_ro) = (FrameInspectMeta::default(), PendingUiIntents::new());
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(900.0, 500.0), Default::default(), true, |_, ui| {
        render_info_view_for_sheet(ui, &iapp, Some(&SheetId::new("info-1")), &mut InfoViewEguiState::default(), &itok, true, &mut m_ro, &mut i_ro);
    })?;
    assert!(
        m_ro.entries.iter().find(|e| e.label == "Dose").and_then(|e| e.value.as_ref()?.get("value")?.as_str()).is_some_and(|s| s.contains("12,5"))
    );

    let mut ist = InfoViewEguiState {
        notes_draft: Some((SheetId::new("info-1"), "Updated note".into())),
        notes_dirty: true,
        editing_sheet_id: Some(SheetId::new("info-1")),
        editing_key: Some("Dose".into()),
        ..Default::default()
    };
    let esc = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(900.0, 500.0), esc, true, |_, ui| {
        render_info_view_for_sheet(
            ui,
            &iapp,
            Some(&SheetId::new("info-1")),
            &mut ist,
            &itok,
            false,
            &mut FrameInspectMeta::default(),
            &mut PendingUiIntents::new(),
        );
    })?;
    assert_eq!(ist.editing_key.as_deref(), Some("Dose"));

    let mut i_sw = PendingUiIntents::new();
    run_egui_pass_with_ctx(&egui::Context::default(), egui::vec2(900.0, 500.0), Default::default(), true, |_, ui| {
        render_info_view_for_sheet(ui, &iapp, Some(&SheetId::new("info-2")), &mut ist, &itok, true, &mut FrameInspectMeta::default(), &mut i_sw);
    })?;
    assert!(i_sw.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, params } if command_id == "info.setNote" && params["sheet_id"] == json!("info-1") && params["text"] == json!("Updated note"))));
    assert!(ist.editing_key.is_none() && ist.editing_sheet_id.is_none());

    // 2. AUD-09-19: arrange_layout_grid_items expands rows to keep excess panels within page bounds.
    let lspec = LayoutSpec {
        items: (0..5)
            .map(|i| LayoutItem::LiveGraph {
                id: format!("p-{i}"),
                graph_sheet_id: SheetId::new(format!("g-{i}")),
                bounds_pt: RectPt::new(0.0, 0.0, 100.0, 100.0),
                scale_percent: 100.0,
                badge_override: None,
            })
            .collect(),
        ..Default::default()
    };
    let arranged = arrange_layout_grid_items(&lspec, 2, 2, 10);
    let page = lspec.page_size_pt();
    assert_eq!(arranged.len(), 5);
    assert!(arranged.iter().all(|it| !matches!(it, LayoutItem::LiveGraph { bounds_pt: b, .. } if b.x < 0.0 || b.y < 0.0 || b.x + b.width > page.width + 1e-6 || b.y + b.height > page.height + 1e-6)));

    let layout = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
        viewport_w_pt: 1440.0,
        viewport_h_pt: 900.0,
        empty_workspace: false,
        navigator_open: true,
        inspector_open: true,
        family_open: true,
        leading_user_collapsed: false,
        trailing_user_collapsed: false,
        leading_width_override_pt: None,
        trailing_width_override_pt: None,
        host_draws_menu_bar: false,
        is_rtl: false,
    });
    let pass_ts = |ctx: &egui::Context,
                   app: &UiApp,
                   ts: &mut TrailingStudioEguiState,
                   tok: &Tokens,
                   raw: egui::RawInput|
     -> Result<(FrameInspectMeta, PendingUiIntents), String> {
        let (mut m, mut i) = (FrameInspectMeta::default(), PendingUiIntents::new());
        run_egui_pass_with_ctx(ctx, egui::vec2(1200.0, 800.0), raw, false, |c, ui| {
            m.begin_pass(c);
            let ctx_in = TrailingStudioContext {
                tokens: tok,
                layout,
                enabled: true,
                focused: true,
                selected_panel_index: None,
                selected_graph_annotation: None,
            };
            render_trailing_studio(ui, app, ts, ctx_in, &mut m, &mut i);
        })?;
        Ok((m, i))
    };

    // 3. AUD-09-18: MV Trailing Studio `+ New` and `add_new_formula` omit `col`.
    let mut mv_app = UiApp::new();
    mv_app.execute_command("sheet.createData", json!({ "sheet_id": "mv-18", "kind": "multiple_variables", "cols": 2, "rows": 2, "variables": [{ "title": "A", "role": "continuous" }, { "title": "B", "role": "continuous" }], "auto_graph": false }))?;
    mv_app.execute_command("sheet.select", json!({ "sheet_id": "mv-18" }))?;
    mv_app.sync_views();
    let tok_mv = Tokens::from_theme_spec(&mv_app.theme, &mv_app.locale);
    let (ctx_mv, mut ts_mv) = (egui::Context::default(), TrailingStudioEguiState::default());
    let (m0, _) = pass_ts(&ctx_mv, &mv_app, &mut ts_mv, &tok_mv, Default::default())?;
    let _ = pass_ts(
        &ctx_mv,
        &mv_app,
        &mut ts_mv,
        &tok_mv,
        click_input(m0.find_by_widget_id("inspector.mv.new_calc_col").unwrap().clipped_rect.center()),
    )?;
    assert_eq!(ts_mv.mv_selected_col, 2);
    ts_mv.mv_formula_draft = "[A] + [B]".into();
    let (m2, _) = pass_ts(&ctx_mv, &mv_app, &mut ts_mv, &tok_mv, Default::default())?;
    let (_, i3) = pass_ts(
        &ctx_mv,
        &mv_app,
        &mut ts_mv,
        &tok_mv,
        click_input(m2.find_by_widget_id("inspector.mv.apply_formula").unwrap().clipped_rect.center()),
    )?;
    assert!(i3.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, params } if command_id == "table.setCalculatedVariable" && params["formula"] == json!("[A] + [B]") && params.get("col").is_none())));
    (ts_mv.mv_selected_col, ts_mv.mv_synced_col, ts_mv.mv_formula_draft) = (0, Some(0), "[A] * 2".into());
    let (m4, _) = pass_ts(&ctx_mv, &mv_app, &mut ts_mv, &tok_mv, Default::default())?;
    let (_, i5) = pass_ts(
        &ctx_mv,
        &mv_app,
        &mut ts_mv,
        &tok_mv,
        click_input(m4.find_by_widget_id("inspector.mv.add_new_formula").unwrap().clipped_rect.center()),
    )?;
    assert!(i5.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, params } if command_id == "table.setCalculatedVariable" && params["formula"] == json!("[A] * 2") && params.get("col").is_none())));

    // 4. AUD-09-20: Info downstream refs bound by info_sheet_id; blank Date emits null w/o kind; Remove Constant hides on rename.
    let mut info_doc = ProjectDocument::new("Info Hooks")?;
    let mut info_a = InfoSheet::new_preseeded("info-a", "A")?;
    info_a.constants.push(InfoConstant::new("RunDate", ConstantValue::Date("2026-05-01".into()), None)?);
    info_a.constants.push(InfoConstant::new("Factor", ConstantValue::Number(2.0), None)?);
    let mut info_b = InfoSheet::new_preseeded("info-b", "B")?;
    info_b.constants.push(InfoConstant::new("Factor", ConstantValue::Number(5.0), None)?);
    info_doc.info_sheets.extend([info_a.clone(), info_b.clone()]);
    let mut mv_b = MultipleVariablesTable::new(2, 2)?;
    mv_b.variables[0].title = "A".into();
    mv_b.variables[1].title = "Scaled".into();
    mv_b.variables[1].role = VariableRole::Calculated;
    mv_b.variables[1].source = VariableSource::Calculated { formula: "[A] * Info(\"Factor\")".into() };
    let mut ds_b = DataSheet::new("ds-bound", "Bound", TableArchetype::MultipleVariables(mv_b))?;
    ds_b.info_sheet_id = Some(SheetId::new("info-a"));
    info_doc.data_sheets.push(ds_b);
    info_doc.active_sheet_id = Some(SheetId::new("info-a"));
    assert_eq!((collect_info_downstream_references(&info_doc, &info_a).len(), collect_info_downstream_references(&info_doc, &info_b).len()), (1, 0));

    let info_app = UiApp::from_session(Session::from_document(info_doc));
    let tok_info = Tokens::from_theme_spec(&info_app.theme, &info_app.locale);
    let mut ts_info = TrailingStudioEguiState {
        info_selected_constant: Some("RunDate".into()),
        info_synced_key: Some((SheetId::new("info-a"), "RunDate".into())),
        info_name_draft: "RunDate".into(),
        info_value_draft: String::new(),
        info_kind_draft: 2,
        ..Default::default()
    };
    let ctx_info = egui::Context::default();
    let (mi0, _) = pass_ts(&ctx_info, &info_app, &mut ts_info, &tok_info, Default::default())?;
    let (_, ii1) = pass_ts(
        &ctx_info,
        &info_app,
        &mut ts_info,
        &tok_info,
        click_input(mi0.find_by_widget_id("inspector.info.save_const").unwrap().clipped_rect.center()),
    )?;
    let sp = ii1
        .intents()
        .iter()
        .find_map(|i| match i {
            UiIntent::EngineCommand { command_id, params } if command_id == "info.setConstant" => Some(params),
            _ => None,
        })
        .unwrap();
    assert!(sp.get("value") == Some(&Value::Null) && sp.get("kind").is_none());
    ts_info.info_name_draft = "Different".into();
    let (mi2, _) = pass_ts(&ctx_info, &info_app, &mut ts_info, &tok_info, Default::default())?;
    assert!(mi0.find_by_widget_id("inspector.info.remove_const").is_some() && mi2.find_by_widget_id("inspector.info.remove_const").is_none());

    // 5. AUD-09-41 & AUD-09-46: brushed deleted sheet fallback, no positional series[0] fallback, fresh Right Y2 title, sticky Methods copy reset.
    let mut g_app = UiApp::new();
    g_app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "ds-g41", "kind": "column", "cols": 2, "rows": 2, "auto_graph": true, "graph_sheet_id": "g-41" }),
    )?;
    let mut gdoc = g_app.document().clone();
    if let Some(gs) = gdoc.graph_sheet_mut(&SheetId::new("g-41"))
        && gs.spec.series.len() >= 2
    {
        gs.spec.series.remove(0);
    }
    gdoc.active_sheet_id = Some(SheetId::new("g-41"));
    let mut g_app = UiApp::from_session(Session::from_document(gdoc));
    g_app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("deleted"), row: 0, col: 0, subcol: 0 });
    assert_eq!(resolve_studio_surface(&g_app), StudioContextSurface::GraphInspector);
    g_app.brushed_cell = Some(SourceCellId { sheet_id: SheetId::new("ds-g41"), row: 0, col: 0, subcol: 0 });
    let tok_g = Tokens::from_theme_spec(&g_app.theme, &g_app.locale);
    let (mg, _) = pass_ts(&egui::Context::default(), &g_app, &mut TrailingStudioEguiState::default(), &tok_g, Default::default())?;
    assert!(!mg.find_by_widget_id("inspector.drill_back.format_point").unwrap().enabled);

    g_app.brushed_cell = None;
    let (ctx_y2, mut ts_y2) =
        (egui::Context::default(), TrailingStudioEguiState { graph_tab: GraphStudioTab::Axes, graph_axis_target: 2, ..Default::default() });
    let (my0, _) = pass_ts(&ctx_y2, &g_app, &mut ts_y2, &tok_g, Default::default())?;
    let (_, iy1) = pass_ts(
        &ctx_y2,
        &g_app,
        &mut ts_y2,
        &tok_g,
        click_input(my0.find_by_widget_id("inspector.graph.scale.Log10").unwrap().clipped_rect.center()),
    )?;
    assert!(iy1.intents().iter().any(|i| matches!(i, UiIntent::EngineCommand { command_id, params } if command_id == "graph.setAxes" && params["y2_axis"]["title"] == json!("Right Y"))));

    let (mut r_app, r_ds) =
        crate::helpers::seed_column_app(&[("A", &[1.0, 3.0, 5.0]), ("B", &[2.0, 4.0, 6.0])], ColumnDesignHint::UnpairedReplicates);
    r_app.execute_command(
        "analysis.run",
        json!({ "source_sheet_id": r_ds.as_str(), "kind": AnalysisKind::DescriptiveStats(DescriptiveSpec::default()) }),
    )?;
    let r_id = r_app.document().result_sheets.first().unwrap().id.clone();
    r_app.execute_command("sheet.select", json!({ "sheet_id": r_id.as_str() }))?;
    r_app.sync_views();
    let tok_r = Tokens::from_theme_spec(&r_app.theme, &r_app.locale);
    let mut ts_r = TrailingStudioEguiState {
        results_tab: ResultsStudioTab::Methods,
        methods_format: "markdown".into(),
        last_copied_methods: true,
        last_copied_methods_text: "stale".into(),
        ..Default::default()
    };
    let _ = pass_ts(&egui::Context::default(), &r_app, &mut ts_r, &tok_r, Default::default())?;
    assert!(!ts_r.last_copied_methods);
    Ok(())
}

mod methods_tab_studio_tests {
    use std::sync::Arc;

    use curvebook_ui_egui::UiApp;
    use curvebook_ui_egui::egui_shell::trailing_studio::{ResultsStudioTab, TrailingStudioContext, TrailingStudioEguiState, render_trailing_studio};
    use curvebook_ui_egui::egui_shell::{FrameInspectMeta, LayoutMetrics, PendingUiIntents, ShellLayoutInput, Tokens, UiIntent};
    use curvebook_ui_egui::is_desktop_host_routed_command;
    use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
    use egui::{Pos2, Vec2};
    use serde_json::{Value, json};

    use crate::helpers::run_egui_pass_with_ctx;

    fn click_at_pos(pos: Pos2) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
            ..Default::default()
        }
    }

    fn render_methods_studio_once(
        ctx: &egui::Context,
        tokens: &Tokens,
        layout: curvebook_ui_egui::egui_shell::ResponsiveShellLayout,
        app_ref: &UiApp,
        state: &mut TrailingStudioEguiState,
        raw: egui::RawInput,
    ) -> Result<(FrameInspectMeta, PendingUiIntents), String> {
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        run_egui_pass_with_ctx(ctx, Vec2::new(1200.0, 800.0), raw, false, |c, ui| {
            meta.begin_pass(c);
            let ctx_in =
                TrailingStudioContext { tokens, layout, enabled: true, focused: true, selected_panel_index: None, selected_graph_annotation: None };
            render_trailing_studio(ui, app_ref, state, ctx_in, &mut meta, &mut intents);
        })?;
        Ok((meta, intents))
    }

    fn preview_text_from_meta(meta: &FrameInspectMeta) -> Result<&str, String> {
        meta.find_by_widget_id("inspector.methods.preview")
            .and_then(|e| e.value.as_ref())
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("missing methods preview text"))
    }

    fn click_methods_widget(
        ctx: &egui::Context,
        tokens: &Tokens,
        layout: curvebook_ui_egui::egui_shell::ResponsiveShellLayout,
        meta: &FrameInspectMeta,
        widget_id: &str,
        app: &UiApp,
        state: &mut TrailingStudioEguiState,
    ) -> Result<PendingUiIntents, String> {
        let entry = meta.find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
        let (_, intents) = render_methods_studio_once(ctx, tokens, layout, app, state, click_at_pos(entry.clipped_rect.center()))?;
        Ok(intents)
    }

    #[test]
    fn r20_7_methods_tab_scope_selectable_preview_save_sync_and_copied_reset() -> Result<(), String> {
        let mut app = UiApp::new();
        let stub = Arc::new(StubPlatformClipboardAdapter::new());
        app.set_clipboard_adapter(stub);
        app.set_human_path_only(true);

        app.execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-xy", "title": "Dose Response", "kind": "xy", "cols": 2, "rows": 6, "auto_graph": false }),
        )
        .map_err(|e| e.to_string())?;
        app.execute_command("table.setTitles", json!({ "sheet_id": "ds-xy", "titles": ["Vehicle", "Agonist"] })).map_err(|e| e.to_string())?;
        app.execute_command("table.setXCells", json!({ "sheet_id": "ds-xy", "values": [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0] }))
            .map_err(|e| e.to_string())?;
        app.execute_command(
            "table.setCells",
            json!({ "sheet_id": "ds-xy", "rows": [[5.0, 6.0], [12.0, 18.0], [48.0, 62.0], [86.0, 91.0], [96.0, 98.0], [99.0, 100.0]] }),
        )
        .map_err(|e| e.to_string())?;
        app.execute_command(
            "fit.run",
            json!({ "source_sheet_id": "ds-xy", "sheet_id": "res-4pl", "kind": "log_agonist_vs_response_4pl", "create_summary_graph": false }),
        )
        .map_err(|e| e.to_string())?;

        app.execute_command(
            "sheet.createData",
            json!({ "sheet_id": "ds-col", "title": "Column Study", "kind": "column", "cols": 3, "rows": 4, "auto_graph": false }),
        )
        .map_err(|e| e.to_string())?;
        app.execute_command(
            "table.setCells",
            json!({ "sheet_id": "ds-col", "rows": [[10.0, 14.0, 21.0], [11.0, 15.0, 22.0], [9.5, 14.5, 20.5], [10.5, 15.5, 21.5]] }),
        )
        .map_err(|e| e.to_string())?;
        app.execute_command("stats.run", json!({ "source_sheet_id": "ds-col", "sheet_id": "res-anova", "kind": "one_way_anova" }))
            .map_err(|e| e.to_string())?;

        app.execute_command("sheet.select", json!({ "sheet_id": "res-4pl" })).map_err(|e| e.to_string())?;
        app.sync_views();

        let layout = LayoutMetrics::CANONICAL.solve_shell_layout(ShellLayoutInput {
            viewport_w_pt: 1200.0,
            viewport_h_pt: 800.0,
            empty_workspace: false,
            navigator_open: false,
            inspector_open: true,
            family_open: false,
            leading_user_collapsed: false,
            trailing_user_collapsed: false,
            leading_width_override_pt: None,
            trailing_width_override_pt: None,
            host_draws_menu_bar: false,
            is_rtl: false,
        });
        let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
        let mut ts = TrailingStudioEguiState { results_tab: ResultsStudioTab::Methods, ..Default::default() };
        let ctx = egui::Context::default();

        let (meta, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        assert_eq!((ts.methods_scope.as_str(), ts.methods_format.as_str()), ("methods", "markdown"));
        for (wid, expected) in [
            ("inspector.methods.scope.methods", "Methods"),
            ("inspector.methods.scope.full", "Full Report"),
            ("inspector.copy_methods", "Copy"),
            ("inspector.save_methods", "Save"),
            ("inspector.cite_curvebook", "Cite"),
        ] {
            let entry = meta.find_by_widget_id(wid).ok_or_else(|| format!("missing widget `{wid}`"))?;
            assert_eq!(entry.label, expected, "unexpected label on `{wid}`");
            assert!(
                !entry.ellipsized && !entry.label.ends_with('…') && !entry.label.ends_with("..."),
                "R20.15 violation on `{wid}`: {}",
                entry.label
            );
        }

        let preview = meta.find_by_widget_id("inspector.methods.preview").ok_or("missing methods preview")?;
        assert_eq!(preview.role, "textbox");
        assert!(preview.enabled, "R20.7: preview must be selectable (enabled), not interactive(false)");
        let preview_h = preview.clipped_rect.height();
        assert!(preview_h > 220.0, "R20.7: preview must expand into available sidebar height (got {preview_h} pt, expected > 220 pt)");
        let preview_text = preview_text_from_meta(&meta)?;
        assert!(preview_text.contains(env!("CARGO_PKG_VERSION")), "Methods prose must include CurveBook version:\n{preview_text}");
        assert!(
            preview_text.contains("Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))"),
            "Clean Markdown equation expected:\n{preview_text}"
        );
        assert!(!preview_text.contains("\\*") && !preview_text.contains("\\_") && !preview_text.contains("\\<") && !preview_text.contains("\\>"));
        assert!(!preview_text.contains("res-4pl") && !preview_text.contains("ds-xy"), "Internal sheet IDs must not leak:\n{preview_text}");
        assert!(!preview_text.contains("Study Context and Data Provenance") && !preview_text.contains("| --- |"));

        let preview_center = Pos2::new(preview.clipped_rect.center().x, preview.clipped_rect.top() + 24.0);
        let mut edit_raw = click_at_pos(preview_center);
        edit_raw.events.push(egui::Event::Text(String::from("MUTATED_TOKEN")));
        let (meta_after_type, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, edit_raw)?;
        assert!(!preview_text_from_meta(&meta_after_type)?.contains("MUTATED_TOKEN"), "Read-only preview must ignore text mutation");

        let mut copy_intents = click_methods_widget(&ctx, &tokens, layout, &meta, "inspector.copy_methods", &app, &mut ts)?;
        assert!(ts.last_copied_methods);
        let has_copy_cmd = copy_intents.intents().iter().any(|i| {
            matches!(
                i,
                UiIntent::UiCommand { command_id, params }
                    if command_id == "ui.result.copyMethodsReport"
                        && params.get("result_sheet_id").and_then(Value::as_str) == Some("res-4pl")
                        && params.get("format").and_then(Value::as_str) == Some("markdown")
                        && params.get("scope").and_then(Value::as_str) == Some("methods")
            )
        });
        assert!(has_copy_cmd, "Copy must emit ui.result.copyMethodsReport with format+scope: {:?}", copy_intents.intents());
        let _ = copy_intents.commit_if_settled(&mut app, false).map_err(|e| e.to_string())?;

        let mut full_intents = click_methods_widget(&ctx, &tokens, layout, &meta, "inspector.methods.scope.full", &app, &mut ts)?;
        assert_eq!(ts.methods_scope, "full");
        assert!(!ts.last_copied_methods, "Scope switch must reset last_copied_methods");
        let _ = full_intents.commit_if_settled(&mut app, false).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.result_view.report_scope, "full");
        let (meta_full, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        let full_text = preview_text_from_meta(&meta_full)?;
        assert!(
            full_text.contains("Study Context and Data Provenance") && full_text.contains("Stored Results Summary") && full_text.contains("| --- |")
        );

        let mut tex_intents = click_methods_widget(&ctx, &tokens, layout, &meta_full, "inspector.methods.fmt.latex", &app, &mut ts)?;
        assert_eq!(ts.methods_format, "latex");
        let _ = tex_intents.commit_if_settled(&mut app, false).map_err(|e| e.to_string())?;
        assert_eq!(app.canvas_dialogs.result_view.report_format, "latex");

        let (meta_tex, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        let save_intents = click_methods_widget(&ctx, &tokens, layout, &meta_tex, "inspector.save_methods", &app, &mut ts)?;
        let has_save_cmd = save_intents.intents().iter().any(|i| {
            matches!(
                i,
                UiIntent::EngineCommand { command_id, params }
                    if command_id == "result.generateReport"
                        && params.get("result_sheet_id").and_then(Value::as_str) == Some("res-4pl")
                        && params.get("format").and_then(Value::as_str) == Some("latex")
                        && params.get("scope").and_then(Value::as_str) == Some("full")
            )
        });
        assert!(has_save_cmd, "Save must emit result.generateReport with format+scope: {:?}", save_intents.intents());
        assert!(is_desktop_host_routed_command("result.generateReport"));

        assert_methods_cite_button_and_locale_fit(&ctx, &tokens, layout, &meta_tex, &app, &mut ts)?;

        ts.last_copied_methods = true;
        ts.last_copied_methods_text = String::from("copied before external change");
        let _ = curvebook_ui_egui::dispatch_ui_command(
            &mut app,
            "ui.result.copyMethodsReport",
            &json!({ "result_sheet_id": "res-4pl", "format": "plain", "scope": "methods", "skip_os_clipboard": true }),
        )
        .map_err(|e| e.to_string())?;
        let (meta_synced, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        assert_eq!((ts.methods_format.as_str(), ts.methods_scope.as_str()), ("plain", "methods"));
        assert!(!ts.last_copied_methods, "External format/scope change must reset last_copied_methods");

        ts.last_copied_methods = true;
        ts.last_copied_methods_text = preview_text_from_meta(&meta_synced)?.to_owned();
        app.execute_command("analysis.updateParams", json!({ "sheet_id": "res-4pl", "confidence_percent": 99.0 })).map_err(|e| e.to_string())?;
        app.sync_views();
        let (meta_99, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        assert!(!ts.last_copied_methods, "Parameter change must reset last_copied_methods");
        let text_99 = preview_text_from_meta(&meta_99)?.to_owned();
        assert!(text_99.contains("99%"), "Preview must reflect updated confidence level:\n{text_99}");

        ts.last_copied_methods = true;
        ts.last_copied_methods_text = text_99;
        app.execute_command("sheet.select", json!({ "sheet_id": "res-anova" })).map_err(|e| e.to_string())?;
        app.sync_views();
        let _ = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        assert!(!ts.last_copied_methods, "Switching ResultSheet must reset last_copied_methods");

        app.canvas_dialogs.result_view.last_methods_report = Some(String::from("STALE_OTHER_SHEET_REPORT"));
        let (meta_anova, _) = render_methods_studio_once(&ctx, &tokens, layout, &app, &mut ts, egui::RawInput::default())?;
        assert!(!preview_text_from_meta(&meta_anova)?.contains("STALE_OTHER_SHEET_REPORT"));
        app.canvas_dialogs.result_view.last_methods_report = None;
        Ok(())
    }

    fn assert_methods_cite_button_and_locale_fit(
        ctx: &egui::Context,
        tokens: &Tokens,
        layout: curvebook_ui_egui::egui_shell::ResponsiveShellLayout,
        meta_tex: &FrameInspectMeta,
        app: &UiApp,
        ts: &mut TrailingStudioEguiState,
    ) -> Result<(), String> {
        let cite_intents = click_methods_widget(ctx, tokens, layout, meta_tex, "inspector.cite_curvebook", app, ts)?;
        assert!(
            cite_intents.intents().iter().any(|i| matches!(i, UiIntent::UiCommand { command_id, .. } if command_id == "app.cite")),
            "Cite button must emit app.cite: {:?}",
            cite_intents.intents()
        );
        for &loc in curvebook_ui_egui::i18n::BETA_ACTIVE_LOCALES {
            let loc_tokens = Tokens::from_theme_spec(&app.theme, loc);
            let (loc_meta, _) = render_methods_studio_once(ctx, &loc_tokens, layout, app, ts, egui::RawInput::default())?;
            for wid in ["inspector.copy_methods", "inspector.save_methods", "inspector.cite_curvebook"] {
                let entry = loc_meta.find_by_widget_id(wid).ok_or_else(|| format!("{loc}: missing `{wid}`"))?;
                assert!(!entry.ellipsized, "{loc}: Methods button `{wid}` ellipsized: `{}`", entry.label);
            }
        }
        Ok(())
    }
}
